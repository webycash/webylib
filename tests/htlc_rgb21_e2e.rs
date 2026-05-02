//! End-to-end RGB21 HTLC swap exercised through webylib's `Client`.
//!
//! Wallet-side counterpart of the server-side
//! `webycash-server/crates/conformance/tests/server_rgb21_htlc.rs`. Same
//! HTLC primitive, same predicate evaluator, same AluVM gate — but
//! exercised through the typed wallet client API
//! (`webylib::server_client::Client::{replace_with_htlc, issue}`) instead
//! of raw JSON. Together with the existing `htlc_swap_e2e.rs` (fungible)
//! this proves both flavors of the RGB type work fully through the
//! webylib client surface.
//!
//! Per `webycash-server/docs/referee-zkp-based-swap.md` §7, RGB21 + HTLC
//! is the primitive that backs RGB21 ↔ Bitcoin ARK and RGB21 ↔ Webcash
//! HTLC swap flows.

#![cfg(feature = "native")]

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use webylib::server_client::{
    Client, HtlcLockEntry, HtlcLockRequest, HtlcWitness, HtlcWitnessEntry,
};

// ─────────────────────────────────────────────────────────────────────────────
// Shared harness helpers (same shape as htlc_swap_e2e.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn workspace_target_bin(name: &str) -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .unwrap()
        .join("webycash-server")
        .join("target")
        .join("debug")
        .join(name)
}

fn ephemeral_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn await_tcp(host: &str, port: u16, deadline: Duration) -> bool {
    let until = std::time::Instant::now() + deadline;
    while std::time::Instant::now() < until {
        if std::net::TcpStream::connect_timeout(
            &format!("{host}:{port}").parse().unwrap(),
            Duration::from_millis(200),
        )
        .is_ok()
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}

fn stop_container(name: &str) {
    let _ = Command::new("docker")
        .args(["stop", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn short_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos:x}").chars().rev().take(10).collect()
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

// ─────────────────────────────────────────────────────────────────────────────
// RGB21 server (rgb-collectible binary) with operator-signed issuance
// ─────────────────────────────────────────────────────────────────────────────

struct RgbCollectibleServer {
    child: Child,
    redis_name: String,
    pub bind: String,
    pub sk: SigningKey,
    pub issuer_fp: String,
}

impl RgbCollectibleServer {
    fn start(seed: u8) -> Option<Self> {
        if !docker_available() {
            eprintln!("skip: docker unavailable");
            return None;
        }
        let bin = workspace_target_bin("webycash-server-rgb-collectible");
        if !bin.exists() {
            eprintln!("skip: {bin:?} not built");
            return None;
        }

        let redis_port = ephemeral_port();
        let redis_name = format!("webylib-rgb21-htlc-{seed:02x}-{}", short_id());
        let started = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                &redis_name,
                "-p",
                &format!("{redis_port}:6379"),
                "redis:7-alpine",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !started {
            return None;
        }
        if !await_tcp("127.0.0.1", redis_port, Duration::from_secs(15)) {
            stop_container(&redis_name);
            return None;
        }

        let sk = SigningKey::from_bytes(&[seed; 32]);
        let vk = sk.verifying_key();
        let pubkey_hex = hex::encode(vk.as_bytes());
        let issuer_fp = hex::encode(&vk.as_bytes()[..20]);

        let server_port = ephemeral_port();
        let bind = format!("127.0.0.1:{server_port}");
        let issuers_env = format!("{issuer_fp}:{pubkey_hex}");

        let child = Command::new(&bin)
            .env("WEBCASH_BIND_ADDR", &bind)
            .env("WEBCASH_MODE", "testnet")
            .env("REDIS_URL", format!("redis://127.0.0.1:{redis_port}"))
            .env("WEBYCASH_ISSUERS", &issuers_env)
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rgb-collectible");
        if !await_tcp("127.0.0.1", server_port, Duration::from_secs(10)) {
            stop_container(&redis_name);
            panic!("server didn't bind on {bind}");
        }
        Some(Self {
            child,
            redis_name,
            bind,
            sk,
            issuer_fp,
        })
    }

    fn url(&self) -> String {
        format!("http://{}", self.bind)
    }

    fn client(&self) -> Client {
        Client::new(self.url())
    }

    /// Issue a single RGB21 collectible via /api/v1/issue. Returns the
    /// client (so callers can drive replace_with_htlc against it).
    fn issue(&self, contract: &str, secret_hex: &str, nonce: &str) -> Client {
        let body = serde_json::json!({
            "issuer_fp": self.issuer_fp,
            "outputs": [format!("secret:{secret_hex}:{contract}:{}", self.issuer_fp)],
            "nonce": nonce,
            "ts": 1714003200_u64,
            "legalese": {"terms": true},
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let sig = self.sk.sign(&body_bytes);
        let sig_hex = hex::encode(sig.to_bytes());
        let client = self.client();
        client.issue(&body_bytes, &sig_hex).expect("issue");
        client
    }
}

impl Drop for RgbCollectibleServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        stop_container(&self.redis_name);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — full HTLC primitive surface on RGB21 through webylib's Client
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn rgb21_lock_and_claim_via_typed_client() {
    let Some(server) = RgbCollectibleServer::start(0x10) else {
        return;
    };
    let contract = format!("rgb21-walletapi-{}", short_id());
    let alice_in = "1".repeat(64);
    let issuer = &server.issuer_fp;
    let client = server.issue(&contract, &alice_in, "issue-claim-1");

    let locked_secret = "2".repeat(64);
    let bob_secret = "3".repeat(64);
    let alice_refund_secret = "4".repeat(64);
    let x_hex = "a".repeat(64);
    let h_hex = sha256_hex(&x_hex);

    // Step 1: Alice locks her RGB21 collectible into HTLC.
    let inputs = vec![format!("secret:{alice_in}:{contract}:{issuer}")];
    let outputs = vec![format!("secret:{locked_secret}:{contract}:{issuer}")];
    let locks = vec![HtlcLockEntry {
        output_index: 0,
        request: HtlcLockRequest {
            committed_h_hex: h_hex.clone(),
            refund_after_seconds_from_now: 3600,
            claim_owner_secret_hex: bob_secret.clone(),
            refund_owner_secret_hex: alice_refund_secret.clone(),
        },
    }];
    client
        .replace_with_htlc(&inputs, &outputs, &locks, &[])
        .expect("rgb21 lock should succeed");

    // Step 2: Bob claims with the preimage.
    let bob_final = format!("secret:{bob_secret}:{contract}:{issuer}");
    let claim_inputs = vec![format!("secret:{locked_secret}:{contract}:{issuer}")];
    let witnesses = vec![HtlcWitnessEntry {
        input_index: 0,
        witness: HtlcWitness::claim(x_hex.clone(), &bob_secret),
    }];
    client
        .replace_with_htlc(
            &claim_inputs,
            std::slice::from_ref(&bob_final),
            &[],
            &witnesses,
        )
        .expect("rgb21 claim should succeed");

    // Post-conditions.
    let body = client
        .health_check(&[
            format!("public:{}:{contract}:{issuer}", sha256_hex(&locked_secret)),
            format!("public:{}:{contract}:{issuer}", sha256_hex(&bob_secret)),
        ])
        .expect("hc");
    assert!(
        body.contains(r#""spent": true"#),
        "rgb21 locked must be spent: {body}"
    );
    assert!(
        body.contains(r#""spent": false"#),
        "rgb21 claim output must be unspent: {body}"
    );
}

#[test]
fn rgb21_refund_after_timeout_via_typed_client() {
    let Some(server) = RgbCollectibleServer::start(0x11) else {
        return;
    };
    let contract = format!("rgb21-refund-{}", short_id());
    let alice_in = "5".repeat(64);
    let issuer = &server.issuer_fp;
    let client = server.issue(&contract, &alice_in, "issue-refund-1");

    let locked_secret = "6".repeat(64);
    let bob_secret = "7".repeat(64);
    let refund_secret = "8".repeat(64);
    let h_hex = sha256_hex(&"9".repeat(64));

    client
        .replace_with_htlc(
            &[format!("secret:{alice_in}:{contract}:{issuer}")],
            &[format!("secret:{locked_secret}:{contract}:{issuer}")],
            &[HtlcLockEntry {
                output_index: 0,
                request: HtlcLockRequest {
                    committed_h_hex: h_hex,
                    refund_after_seconds_from_now: 1,
                    claim_owner_secret_hex: bob_secret,
                    refund_owner_secret_hex: refund_secret.clone(),
                },
            }],
            &[],
        )
        .expect("rgb21 lock");

    std::thread::sleep(Duration::from_secs(2));

    client
        .replace_with_htlc(
            &[format!("secret:{locked_secret}:{contract}:{issuer}")],
            &[format!("secret:{refund_secret}:{contract}:{issuer}")],
            &[],
            &[HtlcWitnessEntry {
                input_index: 0,
                witness: HtlcWitness::refund(&refund_secret),
            }],
        )
        .expect("rgb21 refund after timeout should succeed");
}

#[test]
fn rgb21_claim_with_wrong_preimage_surfaces_error() {
    let Some(server) = RgbCollectibleServer::start(0x12) else {
        return;
    };
    let contract = format!("rgb21-bad-{}", short_id());
    let alice_in = "a".repeat(64);
    let issuer = &server.issuer_fp;
    let client = server.issue(&contract, &alice_in, "issue-bad-1");

    let locked_secret = "b".repeat(64);
    let bob_secret = "c".repeat(64);
    let x = "d".repeat(64);
    let h = sha256_hex(&x);

    client
        .replace_with_htlc(
            &[format!("secret:{alice_in}:{contract}:{issuer}")],
            &[format!("secret:{locked_secret}:{contract}:{issuer}")],
            &[HtlcLockEntry {
                output_index: 0,
                request: HtlcLockRequest {
                    committed_h_hex: h,
                    refund_after_seconds_from_now: 3600,
                    claim_owner_secret_hex: bob_secret.clone(),
                    refund_owner_secret_hex: "e".repeat(64),
                },
            }],
            &[],
        )
        .expect("rgb21 lock");

    let wrong_x = "0".repeat(64);
    let err = client
        .replace_with_htlc(
            &[format!("secret:{locked_secret}:{contract}:{issuer}")],
            &[format!("secret:{bob_secret}:{contract}:{issuer}")],
            &[],
            &[HtlcWitnessEntry {
                input_index: 0,
                witness: HtlcWitness::claim(wrong_x, &bob_secret),
            }],
        )
        .expect_err("wrong preimage must reject");

    use webylib::server_client::ClientError;
    assert!(
        matches!(err, ClientError::Http { status: 500, .. }),
        "expected 500: {err:?}"
    );
}

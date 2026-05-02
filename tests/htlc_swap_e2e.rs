//! End-to-end HTLC swap exercised through webylib's `Client`.
//!
//! Spawns a `webycash-server-rgb` binary (sibling workspace) plus its own
//! redis container, drives the full HTLC primitive (lock + claim + refund)
//! via `webylib_server_client::Client::replace_with_htlc`, and asserts the
//! protocol behaves as documented in
//! `webycash-server/docs/referee-zkp-based-swap.md`.
//!
//! This is the wallet-side counterpart of the server-side
//! `crates/conformance/tests/server_rgb_htlc_swap.rs` — same primitive,
//! exercised through the typed wallet client API instead of raw JSON.
//! Together they prove the wire format and the wallet helper agree.

#![cfg(feature = "native")]

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};

use webylib_server_client::{
    Client, HtlcLockEntry, HtlcLockRequest, HtlcWitness, HtlcWitnessEntry,
};

const ISSUER: &str = "aabbccddeeff00112233445566778899aabbccdd";

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

/// Minimal PoW for testnet difficulty 4.
fn find_pow(template: &str, bits: u32) -> String {
    fn lzb(h: &[u8]) -> u32 {
        let z = h.iter().take_while(|&&b| b == 0).count() as u32;
        h.get(z as usize).map_or(0, |b| b.leading_zeros()) + z * 8
    }
    for n in 0..1_000_000u64 {
        let p = template.replace("__N__", &n.to_string());
        if lzb(&Sha256::digest(p.as_bytes())) >= bits {
            return p;
        }
    }
    panic!("no pow at bits={bits}");
}

struct RgbServer {
    child: Child,
    redis_name: String,
    pub bind: String,
}

impl RgbServer {
    fn start() -> Option<Self> {
        if !docker_available() {
            eprintln!("skip: docker unavailable");
            return None;
        }
        let bin = workspace_target_bin("webycash-server-rgb");
        if !bin.exists() {
            eprintln!("skip: {bin:?} not built");
            return None;
        }
        let redis_port = ephemeral_port();
        let redis_name = format!("htlc-e2e-redis-{}", short_id());
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
            eprintln!("skip: docker run redis failed");
            return None;
        }
        if !await_tcp("127.0.0.1", redis_port, Duration::from_secs(15)) {
            stop_container(&redis_name);
            return None;
        }
        let server_port = ephemeral_port();
        let bind = format!("127.0.0.1:{server_port}");
        let child = Command::new(&bin)
            .env("WEBCASH_BIND_ADDR", &bind)
            .env("WEBCASH_MODE", "testnet")
            .env("WEBYCASH_DIFFICULTY", "4")
            .env("REDIS_URL", format!("redis://127.0.0.1:{redis_port}"))
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rgb");
        if !await_tcp("127.0.0.1", server_port, Duration::from_secs(10)) {
            stop_container(&redis_name);
            panic!("server didn't bind on {bind}");
        }
        Some(Self {
            child,
            redis_name,
            bind,
        })
    }

    fn url(&self) -> String {
        format!("http://{}", self.bind)
    }

    fn mine_one(&self, contract: &str, secret_hex: &str) {
        let subsidy = "0".repeat(64);
        let template = format!(
            r#"{{"webcash":["e1.0:secret:{secret_hex}:{contract}:{ISSUER}"],"subsidy":["e0.5:secret:{subsidy}:{contract}:{ISSUER}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
        );
        let preimage = find_pow(&template, 4);
        Client::new(self.url())
            .mining_report(&preimage)
            .expect("mining_report");
    }
}

impl Drop for RgbServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        stop_container(&self.redis_name);
    }
}

#[test]
fn wallet_drives_full_lock_claim_via_client() {
    let Some(server) = RgbServer::start() else {
        return;
    };

    let contract = format!("rgb20-walletapi-{}", short_id());
    let alice_in = "1".repeat(64);
    server.mine_one(&contract, &alice_in);

    let locked_secret = "2".repeat(64);
    let bob_secret = "3".repeat(64);
    let alice_refund_secret = "4".repeat(64);
    let x_hex = "a".repeat(64);
    let h_hex = sha256_hex(&x_hex);

    let client = Client::new(server.url());

    // Step 1: lock — Alice replaces her plain token with an HTLC-locked one.
    let inputs = vec![format!("e1.0:secret:{alice_in}:{contract}:{ISSUER}")];
    let outputs = vec![format!("e1.0:secret:{locked_secret}:{contract}:{ISSUER}")];
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
        .expect("lock should succeed");

    // Step 2: Bob claims with the preimage. Output owner = Bob's claim secret.
    let bob_final_token = format!("e1.0:secret:{bob_secret}:{contract}:{ISSUER}");
    let claim_inputs = vec![format!("e1.0:secret:{locked_secret}:{contract}:{ISSUER}")];
    let witnesses = vec![HtlcWitnessEntry {
        input_index: 0,
        witness: HtlcWitness::claim(x_hex.clone(), &bob_secret),
    }];
    client
        .replace_with_htlc(
            &claim_inputs,
            std::slice::from_ref(&bob_final_token),
            &[],
            &witnesses,
        )
        .expect("claim should succeed");

    // Sanity: post-conditions.
    let body = client
        .health_check(&[
            format!(
                "e1.0:public:{}:{contract}:{ISSUER}",
                sha256_hex(&locked_secret)
            ),
            format!(
                "e1.0:public:{}:{contract}:{ISSUER}",
                sha256_hex(&bob_secret)
            ),
        ])
        .expect("hc");
    assert!(
        body.contains(r#""spent": true"#),
        "locked must be spent: {body}"
    );
    assert!(
        body.contains(r#""spent": false"#),
        "claim output must be unspent: {body}"
    );
}

#[test]
fn wallet_refund_after_timeout() {
    let Some(server) = RgbServer::start() else {
        return;
    };
    let contract = format!("rgb20-refund-{}", short_id());
    let alice_in = "5".repeat(64);
    server.mine_one(&contract, &alice_in);

    let locked_secret = "6".repeat(64);
    let bob_secret = "7".repeat(64);
    let refund_secret = "8".repeat(64);
    let h_hex = sha256_hex(&"9".repeat(64));
    let client = Client::new(server.url());

    client
        .replace_with_htlc(
            &[format!("e1.0:secret:{alice_in}:{contract}:{ISSUER}")],
            &[format!("e1.0:secret:{locked_secret}:{contract}:{ISSUER}")],
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
        .expect("lock");

    std::thread::sleep(Duration::from_secs(2));

    client
        .replace_with_htlc(
            &[format!("e1.0:secret:{locked_secret}:{contract}:{ISSUER}")],
            &[format!("e1.0:secret:{refund_secret}:{contract}:{ISSUER}")],
            &[],
            &[HtlcWitnessEntry {
                input_index: 0,
                witness: HtlcWitness::refund(&refund_secret),
            }],
        )
        .expect("refund after timeout should succeed");
}

#[test]
fn wallet_claim_with_wrong_preimage_surfaces_error() {
    let Some(server) = RgbServer::start() else {
        return;
    };
    let contract = format!("rgb20-bad-{}", short_id());
    let alice_in = "a".repeat(64);
    server.mine_one(&contract, &alice_in);

    let locked_secret = "b".repeat(64);
    let bob_secret = "c".repeat(64);
    let x = "d".repeat(64);
    let h = sha256_hex(&x);
    let client = Client::new(server.url());

    client
        .replace_with_htlc(
            &[format!("e1.0:secret:{alice_in}:{contract}:{ISSUER}")],
            &[format!("e1.0:secret:{locked_secret}:{contract}:{ISSUER}")],
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
        .expect("lock");

    let wrong_x = "0".repeat(64);
    let err = client
        .replace_with_htlc(
            &[format!("e1.0:secret:{locked_secret}:{contract}:{ISSUER}")],
            &[format!("e1.0:secret:{bob_secret}:{contract}:{ISSUER}")],
            &[],
            &[HtlcWitnessEntry {
                input_index: 0,
                witness: HtlcWitness::claim(wrong_x, &bob_secret),
            }],
        )
        .expect_err("wrong preimage must reject");

    use webylib_server_client::ClientError;
    assert!(
        matches!(err, ClientError::Http { status: 500, .. }),
        "expected 500: {err:?}"
    );
}

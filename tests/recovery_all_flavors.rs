//! Per-flavor recovery against the running compose stack.
//!
//! Exercises the asset-generic [`webylib_core::recover`] against each
//! flavor in `webycash-server/docker-compose.local.yml`:
//!   - Webcash (port 8181, no namespace)
//!   - RGB20 fungible (port 8182, IssuedNamespace)
//!   - Voucher (port 8183, IssuedNamespace)
//!   - RGB21 collectible (port 8184) — minted via mining_report-equivalent
//!     replace-from-genesis path; covered separately because the wire form
//!     drops the amount segment.
//!
//! Per scenario: mine N at MINING:0..N-1, wipe local memory, run recover,
//! assert exactly N outputs come back with correct amounts. Plus the
//! transient-error regression (point at a closed port).
//!
//! Tests skip if the compose isn't reachable; the recovery_compose.rs
//! suite covers the webcash-only deep scenarios. This file proves the
//! generic op works for every flavor.

#![cfg(feature = "native")]

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use webylib_core::{recover, ChainCode, IssuedNamespace, RecoveryError};
use webylib_hd::HdWallet;
use webylib_server_client::Client;
use webylib_wallet_rgb::{RgbCollectible, RgbFungible};
use webylib_wallet_voucher::Voucher;
use webylib_wallet_webcash::Webcash;

const PORT_WEBCASH: u16 = 8181;
const PORT_RGB: u16 = 8182;
const PORT_VOUCHER: u16 = 8183;
const PORT_RGB_COLLECTIBLE: u16 = 8184;

fn server_reachable(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

fn server_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Pin a unique 32-byte master seed per test invocation. The compose
/// stack is persistent; reusing the same seed across runs would collide
/// at the same HD depths and fail on the server's single-use-seal
/// guard. The salt byte separates this test's seeds from sibling tests.
fn fresh_seed(salt: u8) -> [u8; 32] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut seed = [0u8; 32];
    seed[0] = salt;
    seed[1..17].copy_from_slice(&nanos.to_be_bytes());
    for (i, b) in seed.iter_mut().enumerate().skip(17) {
        *b = (i as u8).wrapping_mul(salt).wrapping_add(0x5a);
    }
    seed
}

fn leading_zero_bits(hash: &[u8]) -> u32 {
    let zb = hash.iter().take_while(|&&b| b == 0).count() as u32;
    hash.get(zb as usize).map_or(0, |b| b.leading_zeros()) + zb * 8
}

fn find_pow(template_with_marker: &str, bits: u32) -> String {
    for nonce in 0..1_000_000u64 {
        let p = template_with_marker.replace("__N__", &nonce.to_string());
        if leading_zero_bits(&Sha256::digest(p.as_bytes())) >= bits {
            return p;
        }
    }
    panic!("could not satisfy difficulty {bits}");
}

/// Subsidy secret derived deterministically from the mining secret so
/// that re-mining the same depth on the same seed (which never happens
/// in test, but defensively) doesn't collide on the server's
/// single-use-seal check.
fn subsidy_secret_for(mining_secret: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"recovery-test-subsidy:");
    h.update(mining_secret.as_bytes());
    hex::encode(h.finalize())
}

/// Mine one webcash token at MINING:depth into the server.
fn mine_webcash(seed: [u8; 32], depth: u64) {
    let hd = HdWallet::from_master_secret(seed);
    let secret = hd.derive_secret(ChainCode::Mining, depth);
    let subsidy = subsidy_secret_for(&secret);
    let template = format!(
        r#"{{"webcash":["e1.0:secret:{secret}"],"subsidy":["e0.5:secret:{subsidy}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    Client::new(server_url(PORT_WEBCASH))
        .mining_report(&preimage)
        .expect("webcash mine");
}

/// Mine one RGB20 token at MINING:depth into the server.
fn mine_rgb20(seed: [u8; 32], depth: u64, ns: &IssuedNamespace) {
    let hd = HdWallet::from_master_secret(seed);
    let secret = hd.derive_secret(ChainCode::Mining, depth);
    let subsidy = subsidy_secret_for(&secret);
    let token = format!(
        "e1.0:secret:{secret}:{contract}:{issuer}",
        contract = ns.contract_id,
        issuer = ns.issuer_fp
    );
    let subsidy_token = format!(
        "e0.5:secret:{subsidy}:{contract}:{issuer}",
        contract = ns.contract_id,
        issuer = ns.issuer_fp
    );
    let template = format!(
        r#"{{"webcash":["{token}"],"subsidy":["{subsidy_token}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    Client::new(server_url(PORT_RGB))
        .mining_report(&preimage)
        .expect("rgb20 mine");
}

/// Mine one Voucher token at MINING:depth into the server.
fn mine_voucher(seed: [u8; 32], depth: u64, ns: &IssuedNamespace) {
    let hd = HdWallet::from_master_secret(seed);
    let secret = hd.derive_secret(ChainCode::Mining, depth);
    let subsidy = subsidy_secret_for(&secret);
    let token = format!(
        "e1.0:secret:{secret}:{contract}:{issuer}",
        contract = ns.contract_id,
        issuer = ns.issuer_fp
    );
    let subsidy_token = format!(
        "e0.5:secret:{subsidy}:{contract}:{issuer}",
        contract = ns.contract_id,
        issuer = ns.issuer_fp
    );
    let template = format!(
        r#"{{"webcash":["{token}"],"subsidy":["{subsidy_token}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    Client::new(server_url(PORT_VOUCHER))
        .mining_report(&preimage)
        .expect("voucher mine");
}

// ─────────────────────────────────────────────────────────────────────
// Webcash
// ─────────────────────────────────────────────────────────────────────

#[test]
fn recover_webcash_finds_all_mined() {
    if !server_reachable(PORT_WEBCASH) {
        eprintln!("skip: webcash :{PORT_WEBCASH} unreachable");
        return;
    }
    let seed = fresh_seed(0xb0);
    const N: u64 = 3;
    for d in 0..N {
        mine_webcash(seed, d);
    }
    let client = Client::new(server_url(PORT_WEBCASH));
    let hd = HdWallet::from_master_secret(seed);
    let report = recover::<Webcash>(&client, &hd, &(), 5, &HashMap::new()).expect("recover");
    assert_eq!(report.count() as u64, N);
    assert_eq!(report.total_wats(), N as i64 * 100_000_000);
    // Every recovered output must come from the MINING chain and
    // carry an amount (Webcash sets SERVER_REPORTS_AMOUNT = true).
    for o in &report.recovered {
        assert_eq!(o.chain, ChainCode::Mining);
        assert!(o.amount_wats.is_some(), "webcash output must have amount");
    }
    // last_used_depth advanced to at least N-1.
    assert!(matches!(
        report.last_used_depth.get(&ChainCode::Mining).copied(),
        Some(d) if d == N - 1
    ));
}

// ─────────────────────────────────────────────────────────────────────
// RGB20 fungible
// ─────────────────────────────────────────────────────────────────────

#[test]
fn recover_rgb20_finds_all_mined_in_namespace() {
    if !server_reachable(PORT_RGB) {
        eprintln!("skip: rgb20 :{PORT_RGB} unreachable");
        return;
    }
    // Per-test contract id so re-runs don't collide on the persistent
    // RGB ledger.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns = IssuedNamespace::new(
        format!("rgb20-recovery-{nanos:x}"),
        "aabbccddeeff00112233445566778899aabbccdd",
    );
    let seed = fresh_seed(0xb1);

    const N: u64 = 2;
    for d in 0..N {
        mine_rgb20(seed, d, &ns);
    }

    let client = Client::new(server_url(PORT_RGB));
    let hd = HdWallet::from_master_secret(seed);
    let report = recover::<RgbFungible>(&client, &hd, &ns, 5, &HashMap::new()).expect("recover");

    assert_eq!(report.count() as u64, N, "rgb20 must surface mined outputs");
    assert_eq!(report.total_wats(), N as i64 * 100_000_000);
    for o in &report.recovered {
        assert_eq!(o.namespace, ns, "recovered output stays in namespace");
    }
}

#[test]
fn recover_rgb20_isolates_namespaces() {
    if !server_reachable(PORT_RGB) {
        eprintln!("skip: rgb20 :{PORT_RGB} unreachable");
        return;
    }
    // Mine under contract A; recover under contract B with the same
    // seed → must return zero (namespace partition is real).
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns_a = IssuedNamespace::new(
        format!("rgb20-iso-A-{nanos:x}"),
        "aabbccddeeff00112233445566778899aabbccdd",
    );
    let ns_b = IssuedNamespace::new(
        format!("rgb20-iso-B-{nanos:x}"),
        "aabbccddeeff00112233445566778899aabbccdd",
    );
    let seed = fresh_seed(0xb2);
    mine_rgb20(seed, 0, &ns_a);

    let client = Client::new(server_url(PORT_RGB));
    let hd = HdWallet::from_master_secret(seed);
    let report_b = recover::<RgbFungible>(&client, &hd, &ns_b, 5, &HashMap::new())
        .expect("recover B");
    assert_eq!(
        report_b.count(),
        0,
        "namespace B must not see tokens minted under namespace A"
    );

    let report_a = recover::<RgbFungible>(&client, &hd, &ns_a, 5, &HashMap::new())
        .expect("recover A");
    assert_eq!(report_a.count(), 1, "namespace A surfaces its own token");
}

// ─────────────────────────────────────────────────────────────────────
// Voucher
// ─────────────────────────────────────────────────────────────────────

#[test]
fn recover_voucher_finds_all_mined_in_namespace() {
    if !server_reachable(PORT_VOUCHER) {
        eprintln!("skip: voucher :{PORT_VOUCHER} unreachable");
        return;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns = IssuedNamespace::new(
        format!("voucher-recovery-{nanos:x}"),
        "ccddeeff00112233445566778899aabbccddeeff",
    );
    let seed = fresh_seed(0xb3);

    const N: u64 = 2;
    for d in 0..N {
        mine_voucher(seed, d, &ns);
    }

    let client = Client::new(server_url(PORT_VOUCHER));
    let hd = HdWallet::from_master_secret(seed);
    let report = recover::<Voucher>(&client, &hd, &ns, 5, &HashMap::new()).expect("recover");

    assert_eq!(report.count() as u64, N, "voucher must surface mined outputs");
    assert_eq!(report.total_wats(), N as i64 * 100_000_000);
}

// ─────────────────────────────────────────────────────────────────────
// RGB21 collectible — recovery infrastructure smoke
// ─────────────────────────────────────────────────────────────────────

/// Cold-start: empty server, fresh seed → recovery yields 0.
/// Asserts the asset-generic loop runs cleanly against the collectible
/// flavor (no `amount` field in response) and exits without decode
/// errors.
#[test]
fn recover_rgb21_returns_empty_for_unminted_namespace() {
    if !server_reachable(PORT_RGB_COLLECTIBLE) {
        eprintln!("skip: rgb21 :{PORT_RGB_COLLECTIBLE} unreachable");
        return;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns = IssuedNamespace::new(
        format!("rgb21-recovery-{nanos:x}"),
        "1122334455667788990011223344556677889900",
    );
    let seed = fresh_seed(0xb4);

    let client = Client::new(server_url(PORT_RGB_COLLECTIBLE));
    let hd = HdWallet::from_master_secret(seed);
    let report =
        recover::<RgbCollectible>(&client, &hd, &ns, 5, &HashMap::new()).expect("recover");
    assert_eq!(report.count(), 0, "fresh namespace, no mints, must recover 0");
}

/// Mint-then-recover for RGB21.
///
/// RGB21 has no mining endpoint — minting is operator-only via
/// `/api/v1/issue` with an Ed25519-signed envelope. To exercise the
/// full round-trip, this test spawns its own `webycash-server-rgb-collectible`
/// binary with a deterministic test issuer registered, signs and POSTs
/// `/issue` for HD-derived secrets at MINING:0..N-1, then runs the
/// generic recovery and asserts every minted token surfaces.
#[test]
fn recover_rgb21_after_signed_issue() {
    use ed25519_dalek::{Signer, SigningKey};
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    if !docker_available() {
        eprintln!("skip: docker unavailable");
        return;
    }

    // Locate the debug binary built by the workspace.
    let bin = workspace_target_bin("webycash-server-rgb-collectible");
    if !bin.exists() {
        eprintln!("skip: {bin:?} not built (run `cargo build -p webycash-server-rgb-collectible`)");
        return;
    }

    // Issuer keypair — deterministic seed for a stable per-test fingerprint.
    let sk = SigningKey::from_bytes(&[0xc1u8; 32]);
    let vk = sk.verifying_key();
    let pubkey_hex = hex::encode(vk.as_bytes());
    let issuer = hex::encode(&vk.as_bytes()[..20]);

    // Bring up an ephemeral redis + server on free ports so we don't
    // collide with the compose stack.
    let redis_port = ephemeral_port();
    let redis_name = format!("recovery-rgb21-redis-{}", short_id());
    let redis_started = Command::new("docker")
        .args([
            "run", "-d", "--rm", "--name", &redis_name,
            "-p", &format!("{redis_port}:6379"),
            "redis:7-alpine",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !redis_started {
        eprintln!("skip: docker run redis failed");
        return;
    }
    if !await_tcp("127.0.0.1", redis_port, Duration::from_secs(15)) {
        let _ = stop_container(&redis_name);
        return;
    }

    let server_port = ephemeral_port();
    let bind = format!("127.0.0.1:{server_port}");
    let issuers_env = format!("{issuer}:{pubkey_hex}");
    let mut child: Child = Command::new(&bin)
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
        let _ = child.kill();
        let _ = child.wait();
        let _ = stop_container(&redis_name);
        panic!("server didn't bind on {bind}");
    }

    // Wrap the body so the cleanup runs even on panic.
    let result = std::panic::catch_unwind(|| {
        let contract = "rgb21-recovery-test";
        let ns = IssuedNamespace::new(contract, &issuer);
        let seed = fresh_seed(0xb6);
        let hd = HdWallet::from_master_secret(seed);

        const N: u64 = 3;
        // Mint N NFTs at MINING:0..N-1 via /issue.
        let started_at = Instant::now();
        for d in 0..N {
            let secret = hd.derive_secret(ChainCode::Mining, d);
            let token = format!("secret:{secret}:{contract}:{issuer}");
            let body = serde_json::json!({
                "issuer_fp": issuer,
                "outputs": [token],
                "nonce": format!("issue-rgb21-recover-{d}-{:?}", started_at),
                "ts": 1714003200_u64,
                "legalese": {"terms": true},
            });
            let body_bytes = serde_json::to_vec(&body).unwrap();
            let sig_hex = hex::encode(sk.sign(&body_bytes).to_bytes());
            let client = Client::new(format!("http://{bind}"));
            client
                .issue(&body_bytes, &sig_hex)
                .unwrap_or_else(|e| panic!("/issue depth {d} failed: {e}"));
        }

        // Now run the asset-generic recovery against the same server.
        let client = Client::new(format!("http://{bind}"));
        let report =
            recover::<RgbCollectible>(&client, &hd, &ns, 5, &HashMap::new()).expect("recover");
        assert_eq!(
            report.count() as u64,
            N,
            "all N issued NFTs must surface on recovery"
        );
        // RGB21: amount_wats is None for every recovered output.
        for o in &report.recovered {
            assert!(
                o.amount_wats.is_none(),
                "rgb21 output must NOT carry amount, got {:?}",
                o.amount_wats
            );
            assert_eq!(o.namespace, ns);
            assert_eq!(o.chain, ChainCode::Mining);
        }
    });

    let _ = child.kill();
    let _ = child.wait();
    let _ = stop_container(&redis_name);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Process / port helpers for the spawn-server test
// ─────────────────────────────────────────────────────────────────────

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn workspace_target_bin(name: &str) -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is webylib's; the server binary lives in the
    // sibling webycash-server workspace. The container of this repo is
    // …/webycash/webylib and …/webycash/webycash-server are siblings.
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
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

fn short_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos:x}").chars().rev().take(8).collect()
}

fn await_tcp(host: &str, port: u16, deadline: std::time::Duration) -> bool {
    let until = std::time::Instant::now() + deadline;
    while std::time::Instant::now() < until {
        if std::net::TcpStream::connect_timeout(
            &format!("{host}:{port}").parse().unwrap(),
            std::time::Duration::from_millis(200),
        )
        .is_ok()
        {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    false
}

fn stop_container(name: &str) -> bool {
    std::process::Command::new("docker")
        .args(["stop", name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ─────────────────────────────────────────────────────────────────────
// Cross-cutting: error propagation
// ─────────────────────────────────────────────────────────────────────

#[test]
fn recover_propagates_transport_errors_for_every_flavor() {
    // Pointing each flavor at a closed port must surface as
    // `RecoveryError::Server` — never `Ok` with a partial count.
    let closed = "http://127.0.0.1:1".to_string();
    let seed = fresh_seed(0xb5);
    let hd = HdWallet::from_master_secret(seed);
    let issuer = "f".repeat(40);

    let webcash_err = recover::<Webcash>(
        &Client::new(closed.clone()),
        &hd,
        &(),
        5,
        &HashMap::new(),
    )
    .unwrap_err();
    assert!(
        matches!(webcash_err, RecoveryError::Server { .. }),
        "webcash: {webcash_err:?}"
    );

    let rgb_err = recover::<RgbFungible>(
        &Client::new(closed.clone()),
        &hd,
        &IssuedNamespace::new("c", &issuer),
        5,
        &HashMap::new(),
    )
    .unwrap_err();
    assert!(matches!(rgb_err, RecoveryError::Server { .. }));

    let voucher_err = recover::<Voucher>(
        &Client::new(closed.clone()),
        &hd,
        &IssuedNamespace::new("c", &issuer),
        5,
        &HashMap::new(),
    )
    .unwrap_err();
    assert!(matches!(voucher_err, RecoveryError::Server { .. }));

    let rgb21_err = recover::<RgbCollectible>(
        &Client::new(closed),
        &hd,
        &IssuedNamespace::new("c", &issuer),
        5,
        &HashMap::new(),
    )
    .unwrap_err();
    assert!(matches!(rgb21_err, RecoveryError::Server { .. }));
}

#[test]
fn recover_rejects_zero_gap_limit() {
    let hd = HdWallet::from_master_secret([0u8; 32]);
    let err = recover::<Webcash>(
        &Client::new("http://nowhere.invalid"),
        &hd,
        &(),
        0,
        &HashMap::new(),
    )
    .unwrap_err();
    assert!(matches!(err, RecoveryError::InvalidGapLimit));
}

//! End-to-end test of the `webyc` CLI binary against the running
//! webycash-server compose stack.
//!
//! Spawns the freshly-built `webyc` binary as a child process for each
//! flavor's verbs and verifies the resulting state via direct
//! /api/v1/health_check calls.
//!
//! Skips when Docker / built images are unavailable, mirroring the
//! pattern used by the other compose-driven tests.

#![cfg(feature = "native")]

use std::process::{Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};
use webylib_wallet_webcash::WebcashWallet;
use webylib_wallet_rgb::RgbWallet;
use webylib_wallet_voucher::VoucherWallet;

const PORT_WEBCASH: u16 = 8181;
const PORT_RGB_FUNGIBLE: u16 = 8182;
const PORT_VOUCHER: u16 = 8183;

fn docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn server_reachable(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

fn ensure_compose() -> bool {
    docker_available()
        && server_reachable(PORT_WEBCASH)
        && server_reachable(PORT_RGB_FUNGIBLE)
        && server_reachable(PORT_VOUCHER)
}

fn webyc_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("webyca")
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn find_pow(template: &str, bits: u32) -> String {
    for nonce in 0..200_000u64 {
        let p = template.replace("__N__", &nonce.to_string());
        let lz = leading_zero_bits(&Sha256::digest(p.as_bytes()));
        if lz >= bits {
            return p;
        }
    }
    panic!("could not satisfy difficulty {bits}");
}

fn leading_zero_bits(hash: &[u8]) -> u32 {
    let zeros = hash.iter().take_while(|&&b| b == 0).count() as u32;
    hash.get(zeros as usize).map_or(0, |b| b.leading_zeros()) + zeros * 8
}

/// Run-unique 64-char hex prefix so re-runs don't collide on minted
/// secrets in a persistent Redis. Pads with `0` to keep length 64.
fn run_unique_secret(prefix: u8) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let head = format!("{prefix:02x}{nanos:032x}");
    let mut padded = head;
    padded.truncate(64);
    while padded.len() < 64 {
        padded.push('0');
    }
    padded
}

#[test]
fn webyc_webcash_pay_then_insert() {
    if !ensure_compose() {
        eprintln!("skipping: compose unreachable");
        return;
    }
    let webyc = webyc_path();
    if !webyc.exists() {
        eprintln!("skipping: webyc not built — run `cargo build -p webylib-cli` first");
        return;
    }
    let server_url = format!("http://127.0.0.1:{PORT_WEBCASH}");

    // Mine 1.0 webcash via the wallet (CLI doesn't do mining).
    let wallet = WebcashWallet::new(server_url.clone());
    let secret = run_unique_secret(0x30);
    let subsidy_secret = run_unique_secret(0x31);
    let template = format!(
        r#"{{"webcash":["e1.0:secret:{secret}"],"subsidy":["e0.5:secret:{subsidy_secret}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    wallet.server().mining_report(&preimage).expect("mine");

    // webyc webcash pay: split 1.0 → 0.4 + 0.6.
    let recipient = run_unique_secret(0x32);
    let change = run_unique_secret(0x33);
    let out = Command::new(&webyc)
        .args([
            "--server", &server_url,
            "webcash", "pay",
            "--inputs", &format!("e1.0:secret:{secret}"),
            "--outputs",
            &format!("e0.4:secret:{recipient},e0.6:secret:{change}"),
        ])
        .output()
        .expect("spawn webyc");
    assert!(
        out.status.success(),
        "webyc webcash pay exited non-zero\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // webyc webcash insert: rotate the change to a Bob-owned secret.
    let bob = run_unique_secret(0x34);
    let status = Command::new(&webyc)
        .args([
            "--server", &server_url,
            "webcash", "insert",
            "--received", &format!("e0.6:secret:{change}"),
            "--rotate-to", &format!("e0.6:secret:{bob}"),
        ])
        .status()
        .expect("spawn webyc");
    assert!(status.success(), "webyc webcash insert exited non-zero");

    let bob_hash = sha256_hex(&bob);
    let recipient_hash = sha256_hex(&recipient);
    let body = wallet
        .server()
        .health_check(&[
            format!("e0.6:public:{bob_hash}"),
            format!("e0.4:public:{recipient_hash}"),
        ])
        .expect("hc");
    assert!(body.contains(r#""spent": false"#), "[webcash cli] hc: {body}");
}

#[test]
fn webyc_rgb_transfer_then_insert() {
    if !ensure_compose() {
        return;
    }
    let webyc = webyc_path();
    if !webyc.exists() {
        return;
    }
    let server_url = format!("http://127.0.0.1:{PORT_RGB_FUNGIBLE}");
    let wallet = RgbWallet::new(server_url.clone());

    let issuer = "aabbccddeeff00112233445566778899aabbccdd";
    let contract = "rgb20-cli-test";
    let secret = run_unique_secret(0x40);

    let template = format!(
        r#"{{"webcash":["e50.0:secret:{secret}:{contract}:{issuer}"],"subsidy":[],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    wallet.server().mining_report(&preimage).expect("mine");

    let recipient = run_unique_secret(0x41);
    let change = run_unique_secret(0x42);
    let status = Command::new(&webyc)
        .args([
            "--server", &server_url,
            "rgb", "transfer",
            "--inputs", &format!("e50.0:secret:{secret}:{contract}:{issuer}"),
            "--outputs",
            &format!(
                "e20.0:secret:{recipient}:{contract}:{issuer},e30.0:secret:{change}:{contract}:{issuer}"
            ),
        ])
        .status()
        .expect("spawn webyc");
    assert!(status.success(), "webyc rgb transfer exited non-zero");

    let bob = run_unique_secret(0x43);
    let status = Command::new(&webyc)
        .args([
            "--server", &server_url,
            "rgb", "insert",
            "--received", &format!("e20.0:secret:{recipient}:{contract}:{issuer}"),
            "--rotate-to", &format!("e20.0:secret:{bob}:{contract}:{issuer}"),
        ])
        .status()
        .expect("spawn webyc");
    assert!(status.success(), "webyc rgb insert exited non-zero");
}

#[test]
fn webyc_voucher_pay_then_insert() {
    if !ensure_compose() {
        return;
    }
    let webyc = webyc_path();
    if !webyc.exists() {
        return;
    }
    let server_url = format!("http://127.0.0.1:{PORT_VOUCHER}");
    let wallet = VoucherWallet::new(server_url.clone());

    let issuer = "aabbccddeeff00112233445566778899aabbccdd";
    let contract = "credits-cli-test";
    let secret = run_unique_secret(0x50);

    let template = format!(
        r#"{{"webcash":["e25.0:secret:{secret}:{contract}:{issuer}"],"subsidy":[],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    wallet.server().mining_report(&preimage).expect("mine");

    let recipient = run_unique_secret(0x51);
    let change = run_unique_secret(0x52);
    let status = Command::new(&webyc)
        .args([
            "--server", &server_url,
            "voucher", "pay",
            "--inputs", &format!("e25.0:secret:{secret}:{contract}:{issuer}"),
            "--outputs",
            &format!(
                "e10.0:secret:{recipient}:{contract}:{issuer},e15.0:secret:{change}:{contract}:{issuer}"
            ),
        ])
        .status()
        .expect("spawn webyc");
    assert!(status.success(), "webyc voucher pay exited non-zero");

    let bob = run_unique_secret(0x53);
    let status = Command::new(&webyc)
        .args([
            "--server", &server_url,
            "voucher", "insert",
            "--received",
            &format!("e10.0:secret:{recipient}:{contract}:{issuer}"),
            "--rotate-to", &format!("e10.0:secret:{bob}:{contract}:{issuer}"),
        ])
        .status()
        .expect("spawn webyc");
    assert!(status.success(), "webyc voucher insert exited non-zero");
}

/// `webyca target` returns 0 and prints the server's mining target JSON
/// for every flavor. Read-only; no state mutation.
#[test]
fn webyc_target_against_every_flavor() {
    if !ensure_compose() {
        return;
    }
    let webyc = webyc_path();
    if !webyc.exists() {
        return;
    }
    for port in [PORT_WEBCASH, PORT_RGB_FUNGIBLE, PORT_VOUCHER] {
        let url = format!("http://127.0.0.1:{port}");
        let out = Command::new(&webyc)
            .args(["--server", &url, "target"])
            .output()
            .expect("spawn webyc target");
        assert!(
            out.status.success(),
            "[port {port}] target failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("difficulty_target_bits") && stdout.contains("mining_amount"),
            "[port {port}] unexpected stdout: {stdout}"
        );
    }
}

/// `webyca check` prints the server's health_check JSON for an
/// unknown public token (`spent: null`). Exercises the verb against
/// the webcash flavor.
#[test]
fn webyc_check_against_webcash_unknown_token() {
    if !ensure_compose() {
        return;
    }
    let webyc = webyc_path();
    if !webyc.exists() {
        return;
    }
    let url = format!("http://127.0.0.1:{PORT_WEBCASH}");
    let novel_hash = sha256_hex(&run_unique_secret(0x60));
    let out = Command::new(&webyc)
        .args([
            "--server", &url,
            "check", "--tokens", &format!("e1.0:public:{novel_hash}"),
        ])
        .output()
        .expect("spawn webyc check");
    assert!(out.status.success(), "check failed: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(r#""spent": null"#),
        "expected unknown token; got: {stdout}"
    );
}

/// `webyca burn` permanently destroys a freshly-mined webcash secret.
/// Verifies the server transitions the public hash to `spent: true`.
#[test]
fn webyc_burn_consumes_a_mined_secret() {
    if !ensure_compose() {
        return;
    }
    let webyc = webyc_path();
    if !webyc.exists() {
        return;
    }
    let url = format!("http://127.0.0.1:{PORT_WEBCASH}");
    let wallet = WebcashWallet::new(url.clone());

    // Mine a fresh 1.0 webcash so we have something to burn.
    let secret = run_unique_secret(0x70);
    let subsidy = run_unique_secret(0x71);
    let template = format!(
        r#"{{"webcash":["e1.0:secret:{secret}"],"subsidy":["e0.5:secret:{subsidy}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    wallet.server().mining_report(&preimage).expect("mine");

    // webyca burn the secret.
    let out = Command::new(&webyc)
        .args([
            "--server", &url,
            "burn", "--secret", &format!("e1.0:secret:{secret}"),
        ])
        .output()
        .expect("spawn webyc burn");
    assert!(
        out.status.success(),
        "burn failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The hash must now be marked spent.
    let h = sha256_hex(&secret);
    let body = wallet
        .server()
        .health_check(&[format!("e1.0:public:{h}")])
        .expect("hc");
    assert!(
        body.contains(r#""spent": true"#),
        "burn didn't mark spent: {body}"
    );
}

/// `webyca stats` returns 0 and prints the server's economy snapshot
/// for every flavor.
#[test]
fn webyc_stats_against_every_flavor() {
    if !ensure_compose() {
        return;
    }
    let webyc = webyc_path();
    if !webyc.exists() {
        return;
    }
    for port in [PORT_WEBCASH, PORT_RGB_FUNGIBLE, PORT_VOUCHER] {
        let url = format!("http://127.0.0.1:{port}");
        let out = Command::new(&webyc)
            .args(["--server", &url, "stats"])
            .output()
            .expect("spawn webyc stats");
        assert!(
            out.status.success(),
            "[port {port}] stats failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("total_circulation") && stdout.contains("epoch"),
            "[port {port}] unexpected stdout: {stdout}"
        );
    }
}

/// `webyca mining-report` accepts a freshly-found PoW preimage and
/// returns 0. Verifies the resulting hash is now mined (unspent).
#[test]
fn webyc_mining_report_via_cli() {
    if !ensure_compose() {
        return;
    }
    let webyc = webyc_path();
    if !webyc.exists() {
        return;
    }
    let url = format!("http://127.0.0.1:{PORT_WEBCASH}");

    let secret = run_unique_secret(0x80);
    let subsidy = run_unique_secret(0x81);
    let template = format!(
        r#"{{"webcash":["e1.0:secret:{secret}"],"subsidy":["e0.5:secret:{subsidy}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);

    let out = Command::new(&webyc)
        .args(["--server", &url, "mining-report", "--preimage", &preimage])
        .output()
        .expect("spawn webyc mining-report");
    assert!(
        out.status.success(),
        "mining-report failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The mined secret's hash must now be unspent in the registry.
    let h = sha256_hex(&secret);
    let wallet = WebcashWallet::new(url.clone());
    let body = wallet
        .server()
        .health_check(&[format!("e1.0:public:{h}")])
        .expect("hc");
    assert!(
        body.contains(r#""spent": false"#),
        "mining-report didn't mark hash unspent: {body}"
    );
}

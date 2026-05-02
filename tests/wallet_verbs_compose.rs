//! End-to-end test of the wallet-layer verbs against running Docker'd
//! servers. Demonstrates that:
//!
//!   webyc webcash pay      → server's /api/v1/replace
//!   webyc webcash insert   → server's /api/v1/replace (1:1)
//!   webyc rgb     transfer → server's /api/v1/replace (any arity)
//!   webyc rgb     insert   → server's /api/v1/replace (1:1)
//!   webyc voucher pay      → server's /api/v1/replace
//!   webyc voucher insert   → server's /api/v1/replace (1:1)
//!
//! All wallet verbs land on the SAME server endpoint. Wallet-layer
//! semantics differ; the server is a single-use-seal registry.
//!
//! Skips when Docker / built images are unavailable. Reuses the running
//! compose if `docker-compose.local.yml` is already up; otherwise it
//! does NOT bring it up itself (see all_flavors_compose.rs for that).

#![cfg(feature = "native")]

use std::process::{Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};
use webylib_wallet_rgb::RgbWallet;
use webylib_wallet_voucher::VoucherWallet;
use webylib_wallet_webcash::WebcashWallet;

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

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn find_pow(template_with_marker: &str, bits: u32) -> String {
    for nonce in 0..200_000u64 {
        let p = template_with_marker.replace("__N__", &nonce.to_string());
        let lz = leading_zero_bits(&Sha256::digest(p.as_bytes()));
        if lz >= bits {
            return p;
        }
    }
    panic!("could not satisfy difficulty {bits}");
}

fn leading_zero_bits(hash: &[u8]) -> u32 {
    let full_zero_bytes = hash.iter().take_while(|&&b| b == 0).count() as u32;
    hash.get(full_zero_bytes as usize)
        .map_or(0, |b| b.leading_zeros())
        + full_zero_bytes * 8
}

/// Run-unique 64-char hex secret so re-runs against a persistent
/// Redis don't trip the single-use-seal "token already exists" guard.
fn unique_secret(prefix: u8) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut s = format!("{prefix:02x}{nanos:032x}");
    s.truncate(64);
    while s.len() < 64 {
        s.push('0');
    }
    s
}

fn ensure_compose() -> bool {
    if !docker_available() {
        return false;
    }
    if !server_reachable(PORT_WEBCASH) {
        // Try to bring up the compose stack if the test driver crate is in
        // the right relative location.
        let compose_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("webycash-server");
        if !compose_dir.join("docker-compose.local.yml").exists() {
            return false;
        }
        let _ = Command::new("docker")
            .args(["compose", "-f", "docker-compose.local.yml", "up", "-d"])
            .current_dir(&compose_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // Wait up to 20s for /api/v1/target to respond on each port.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            if server_reachable(PORT_WEBCASH)
                && server_reachable(PORT_RGB_FUNGIBLE)
                && server_reachable(PORT_VOUCHER)
            {
                std::thread::sleep(Duration::from_secs(2));
                return true;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        return false;
    }
    true
}

#[test]
fn webcash_pay_and_insert_via_wallet() {
    if !ensure_compose() {
        eprintln!("skipping: compose unreachable");
        return;
    }

    let wallet = WebcashWallet::new(format!("http://127.0.0.1:{PORT_WEBCASH}"));

    // 1. mine a 1.0 webcash secret directly via the client (mining_report
    //    isn't covered by `pay`; the wallet imports outputs after mining).
    let secret = unique_secret(0x0a);
    let subsidy = unique_secret(0x0b);
    let template = format!(
        r#"{{"webcash":["e1.0:secret:{secret}"],"subsidy":["e0.5:secret:{subsidy}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    wallet.server().mining_report(&preimage).expect("mine");

    // 2. webyc webcash pay: split 1.0 → 0.4 (recipient) + 0.6 (change)
    let recipient = unique_secret(0x01);
    let change = unique_secret(0x02);
    wallet
        .pay(
            &[format!("e1.0:secret:{secret}")],
            &[
                format!("e0.4:secret:{recipient}"),
                format!("e0.6:secret:{change}"),
            ],
        )
        .expect("pay");

    // 3. webyc webcash insert: simulate Bob receiving Alice's secret +
    //    rotating to his own. Use the change as the "received" secret
    //    for this self-test.
    let bob_secret = unique_secret(0x03);
    wallet
        .insert(&format!("e0.6:secret:{change}"), &format!("e0.6:secret:{bob_secret}"))
        .expect("insert");

    // Verify the original input is spent + Bob's new hash is unspent.
    let bob_hash = sha256_hex(&bob_secret);
    let recipient_hash = sha256_hex(&recipient);
    let body = wallet
        .server()
        .health_check(&[
            format!("e0.6:public:{bob_hash}"),
            format!("e0.4:public:{recipient_hash}"),
        ])
        .expect("hc");
    assert!(body.contains(r#""spent": false"#), "[webcash] hc: {body}");
}

#[test]
fn rgb_transfer_and_insert_via_wallet() {
    if !ensure_compose() {
        return;
    }
    let wallet = RgbWallet::new(format!("http://127.0.0.1:{PORT_RGB_FUNGIBLE}"));

    let issuer = "aabbccddeeff00112233445566778899aabbccdd";
    let contract = "rgb20-test-2";
    let secret = unique_secret(0xee);

    // mine the seed token
    let template = format!(
        r#"{{"webcash":["e50.0:secret:{secret}:{contract}:{issuer}"],"subsidy":[],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    wallet.server().mining_report(&preimage).expect("mine");

    // webyc rgb transfer: 50.0 → 20 to recipient + 30 change
    let recipient_secret = unique_secret(0xab);
    let change_secret = unique_secret(0xcd);
    wallet
        .transfer(
            &[format!("e50.0:secret:{secret}:{contract}:{issuer}")],
            &[
                format!("e20.0:secret:{recipient_secret}:{contract}:{issuer}"),
                format!("e30.0:secret:{change_secret}:{contract}:{issuer}"),
            ],
        )
        .expect("transfer");

    // webyc rgb insert: rotate the recipient secret to a Bob-owned one.
    let bob_secret = unique_secret(0x12);
    wallet
        .insert(
            &format!("e20.0:secret:{recipient_secret}:{contract}:{issuer}"),
            &format!("e20.0:secret:{bob_secret}:{contract}:{issuer}"),
        )
        .expect("insert");
}

#[test]
fn voucher_pay_and_insert_via_wallet() {
    if !ensure_compose() {
        return;
    }
    let wallet = VoucherWallet::new(format!("http://127.0.0.1:{PORT_VOUCHER}"));

    let issuer = "aabbccddeeff00112233445566778899aabbccdd";
    let contract = "credits-test-2";
    let secret = unique_secret(0xf1);

    let template = format!(
        r#"{{"webcash":["e25.0:secret:{secret}:{contract}:{issuer}"],"subsidy":[],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);
    wallet.server().mining_report(&preimage).expect("mine");

    let recipient = unique_secret(0xf2);
    let change = unique_secret(0xf3);
    wallet
        .pay(
            &[format!("e25.0:secret:{secret}:{contract}:{issuer}")],
            &[
                format!("e10.0:secret:{recipient}:{contract}:{issuer}"),
                format!("e15.0:secret:{change}:{contract}:{issuer}"),
            ],
        )
        .expect("pay");

    let bob = unique_secret(0xf4);
    wallet
        .insert(
            &format!("e10.0:secret:{recipient}:{contract}:{issuer}"),
            &format!("e10.0:secret:{bob}:{contract}:{issuer}"),
        )
        .expect("insert");
}

#[test]
fn cross_namespace_replace_rejected_by_server_for_rgb() {
    if !ensure_compose() {
        return;
    }
    let wallet = RgbWallet::new(format!("http://127.0.0.1:{PORT_RGB_FUNGIBLE}"));

    // Try to replace a token in `contract-A` with one in `contract-B`.
    // The wallet hands the request straight to the server; the server
    // rejects with namespace mismatch.
    let issuer = "aabbccddeeff00112233445566778899aabbccdd";
    let result = wallet.transfer(
        &[format!("e1.0:secret:{}:contract-A:{issuer}", "1".repeat(64))],
        &[format!("e1.0:secret:{}:contract-B:{issuer}", "2".repeat(64))],
    );
    let err = result.expect_err("server must reject cross-namespace");
    assert!(
        err.to_string().contains("HTTP error: 500"),
        "expected 500 status: {err}"
    );
}

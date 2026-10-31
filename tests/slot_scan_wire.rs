//! `wallet::scan_slots` against a mock webcash server: only slots the
//! server reports activity for come back, and transport errors fail
//! closed instead of yielding a partial scan.
#![cfg(feature = "native")]

use mockito::Matcher;
use webylib::hd::{ChainCode, HDWallet};
use webylib::server::NetworkMode;
use webylib::wallet::{scan_slots, SlotCandidate};
use zeroize::Zeroizing;

fn slot_secret(byte: u8) -> String {
    hex::encode([byte; 32])
}

fn public_hash(secret: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn candidate(index: u32, label: &str, byte: u8) -> SlotCandidate {
    SlotCandidate {
        index,
        label: label.to_string(),
        master_secret_hex: Zeroizing::new(slot_secret(byte)),
    }
}

#[tokio::test]
async fn returns_only_slots_with_server_activity() {
    let hit = HDWallet::from_master_secret([0x22; 32]).derive_secret(ChainCode::Receive, 0);
    let hit_hash = public_hash(&hit);
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/api/v1/health_check")
        .match_body(Matcher::Regex(hit_hash.clone()))
        .with_body(format!(
            r#"{{"status":"success","results":{{"e7:public:{hit_hash}":{{"spent":false,"amount":"7"}}}}}}"#
        ))
        .create_async()
        .await;
    server
        .mock("POST", "/api/v1/health_check")
        .with_body(r#"{"status":"success","results":{}}"#)
        .create_async()
        .await;

    let candidates = [
        candidate(0, "main", 0x11),
        candidate(1, "wallet-1", 0x22),
        candidate(2, "wallet-2", 0x33),
    ];
    let active = scan_slots(NetworkMode::Custom(server.url()), &candidates, 5)
        .await
        .expect("scan");

    assert_eq!(active.len(), 1);
    assert_eq!(active[0].index, 1);
    assert_eq!(active[0].label, "wallet-1");
    assert_eq!(active[0].recovered_count, 1);
    assert_eq!(active[0].balance.wats, 7 * 100_000_000);
    assert!(active[0].state_json.contains(&hit));
    assert!(
        !format!("{:?}", active[0]).contains(&hit),
        "Debug must redact state"
    );
}

#[tokio::test]
async fn transport_error_fails_the_whole_scan() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/api/v1/health_check")
        .with_status(503)
        .create_async()
        .await;
    let candidates = [candidate(0, "main", 0x11)];
    let err = scan_slots(NetworkMode::Custom(server.url()), &candidates, 5)
        .await
        .expect_err("503 must fail the scan");
    assert!(err.to_string().contains("health_check"), "{err}");
}

#[tokio::test]
async fn empty_candidate_list_is_an_empty_scan() {
    let active = scan_slots(NetworkMode::Custom("http://127.0.0.1:9".into()), &[], 5)
        .await
        .expect("no network needed");
    assert!(active.is_empty());
}

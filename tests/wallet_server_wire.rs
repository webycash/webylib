//! Wallet ↔ server over the shared `server_client::wire` protocol, against a
//! mock server that answers exactly like webycash-server / webcash.org
//! (health_check keys normalised to `e{amount}:public:{hash}`).
#![cfg(feature = "native")]

use mockito::Matcher;
use std::str::FromStr;
use webylib::server::NetworkMode;
use webylib::wallet::Wallet;
use webylib::webcash::SecretWebcash;

const SECRET: &str = "aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11";

fn public_hash(secret: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn health_body(entries: &[(String, &str)]) -> String {
    let rows: Vec<String> = entries
        .iter()
        .map(|(h, row)| format!(r#""e1:public:{h}":{row}"#))
        .collect();
    format!(r#"{{"status":"success","results":{{{}}}}}"#, rows.join(","))
}

async fn wallet(server: &mockito::Server) -> Wallet {
    Wallet::open_memory_with_network(NetworkMode::Custom(server.url())).expect("wallet")
}

fn input() -> SecretWebcash {
    SecretWebcash::parse(&format!("e1:secret:{SECRET}")).unwrap()
}

#[tokio::test]
async fn validated_insert_reads_normalised_health_key_then_replaces() {
    let mut server = mockito::Server::new_async().await;
    let hc = server
        .mock("POST", "/api/v1/health_check")
        .match_header("content-type", "text/plain;charset=UTF-8")
        .with_body(health_body(&[(
            public_hash(SECRET),
            r#"{"spent":false,"amount":"1"}"#,
        )]))
        .expect(1)
        .create_async()
        .await;
    let replace = server
        .mock("POST", "/api/v1/replace")
        .match_header("content-type", "text/plain;charset=UTF-8")
        .match_body(Matcher::Regex(format!("e1:secret:{SECRET}")))
        .with_body(r#"{"status": "success"}"#)
        .expect(1)
        .create_async()
        .await;
    let w = wallet(&server).await;
    w.insert_with_validation(input(), true)
        .await
        .expect("insert");
    hc.assert_async().await;
    replace.assert_async().await;
    assert_eq!(w.balance().await.unwrap(), "1");
}

#[tokio::test]
async fn validated_insert_refuses_spent_input_without_replacing() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/api/v1/health_check")
        .with_body(health_body(&[(
            public_hash(SECRET),
            r#"{"spent":true,"amount":"1"}"#,
        )]))
        .create_async()
        .await;
    let replace = server
        .mock("POST", "/api/v1/replace")
        .expect(0)
        .create_async()
        .await;
    let w = wallet(&server).await;
    let err = w.insert_with_validation(input(), true).await.unwrap_err();
    assert!(err.to_string().contains("spent"), "{err}");
    replace.assert_async().await;
}

#[tokio::test]
async fn replace_without_success_status_is_not_settled() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/api/v1/replace")
        .with_body("<html>gateway</html>")
        .create_async()
        .await;
    let w = wallet(&server).await;
    assert!(w.insert(input()).await.is_err());
    assert_eq!(w.balance().await.unwrap(), "0");
}

#[tokio::test]
async fn check_counts_spent_unspent_and_unknown_by_hash() {
    let mut server = mockito::Server::new_async().await;
    let secrets = ["bb".repeat(32), "cc".repeat(32), "dd".repeat(32)];
    server
        .mock("POST", "/api/v1/health_check")
        .with_body(health_body(&[
            (public_hash(&secrets[0]), r#"{"spent":false,"amount":"1"}"#),
            (public_hash(&secrets[1]), r#"{"spent":true,"amount":"1"}"#),
            (public_hash(&secrets[2]), r#"{"spent":null,"amount":null}"#),
        ]))
        .create_async()
        .await;
    let w = wallet(&server).await;
    for s in &secrets {
        w.store_directly(SecretWebcash::parse(&format!("e1:secret:{s}")).unwrap())
            .await
            .unwrap();
    }
    let r = w.check().await.expect("check");
    assert_eq!((r.valid_count, r.spent_count, r.unknown_count), (1, 1, 1));
}

#[tokio::test]
async fn recover_scans_over_the_async_client() {
    let mut server = mockito::Server::new_async().await;
    // Nothing known: one window per chain, nothing recovered.
    let hc = server
        .mock("POST", "/api/v1/health_check")
        .with_body(r#"{"status":"success","results":{}}"#)
        .expect(4)
        .create_async()
        .await;
    let w = wallet(&server).await;
    let r = w.recover(&"11".repeat(32), 5).await.expect("recover");
    assert_eq!(r.recovered_count, 0);
    hc.assert_async().await;
    assert!(
        w.recover(&"11".repeat(32), 0).await.is_err(),
        "zero gap fails closed"
    );
}

const MASTER: &str = "22222222222222222222222222222222222222222222222222222222222222aa";

async fn seeded_wallet(server: &mockito::Server) -> Wallet {
    let w = wallet(server).await;
    w.store_master_secret(MASTER).await.unwrap();
    w
}

fn depth(w: &Wallet, chain: &str) -> i64 {
    w.export_snapshot()
        .unwrap()
        .depths
        .get(chain)
        .copied()
        .unwrap_or(0)
}

#[tokio::test]
async fn pay_webcash_returns_the_payment_token_and_keeps_change() {
    let mut server = mockito::Server::new_async().await;
    let replace = server
        .mock("POST", "/api/v1/replace")
        .match_header("content-type", "text/plain;charset=UTF-8")
        .with_body(r#"{"status": "success"}"#)
        .expect(1)
        .create_async()
        .await;
    let w = seeded_wallet(&server).await;
    w.store_directly(SecretWebcash::parse(&format!("e3:secret:{SECRET}")).unwrap())
        .await
        .unwrap();
    let payment = w
        .pay_webcash(webylib::Amount::from_str("1").unwrap(), "")
        .await
        .expect("pay");
    replace.assert_async().await;
    assert_eq!(payment.amount.to_string(), "1");
    assert_eq!(w.balance().await.unwrap(), "2");
    assert_eq!(depth(&w, "PAY"), 1);
    // The token is not kept by the payer.
    let held: Vec<String> = w
        .list_webcash()
        .await
        .unwrap()
        .iter()
        .map(|t| t.to_string())
        .collect();
    assert!(!held.contains(&payment.to_string()));
}

#[tokio::test]
async fn pay_webcash_leaves_wallet_untouched_when_replace_fails() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/api/v1/replace")
        .with_status(500)
        .with_body("boom")
        .create_async()
        .await;
    let w = seeded_wallet(&server).await;
    w.store_directly(SecretWebcash::parse(&format!("e3:secret:{SECRET}")).unwrap())
        .await
        .unwrap();
    assert!(w
        .pay_webcash(webylib::Amount::from_str("1").unwrap(), "")
        .await
        .is_err());
    assert_eq!(w.balance().await.unwrap(), "3");
    assert_eq!(depth(&w, "PAY"), 0);
}

fn target(bits: u32, subsidy: &str) -> webylib::server::TargetResponse {
    webylib::server_client::wire::parse_target(
        200,
        format!(
            r#"{{"difficulty_target_bits":{bits},"epoch":1,"mining_amount":"200","mining_subsidy_amount":"{subsidy}","ratio":1}}"#
        )
        .as_bytes(),
    )
    .unwrap()
}

#[tokio::test]
async fn mine_batch_without_solution_changes_nothing() {
    let mut server = mockito::Server::new_async().await;
    let report = server
        .mock("POST", "/api/v1/mining_report")
        .expect(0)
        .create_async()
        .await;
    let w = seeded_wallet(&server).await;
    let b = w
        .mine_batch(&target(255, "0"), 1_700_000_000, 0, 50)
        .await
        .unwrap();
    assert_eq!(b.attempted, 50);
    assert!(b.result.is_none());
    assert_eq!(depth(&w, "MINING"), 0);
    assert_eq!(w.balance().await.unwrap(), "0");
    report.assert_async().await;
}

#[tokio::test]
async fn mine_batch_reports_preimage_then_stores_hd_mining_webcash() {
    let mut server = mockito::Server::new_async().await;
    let report = server
        .mock("POST", "/api/v1/mining_report")
        .match_header("content-type", "text/plain;charset=UTF-8")
        .match_body(Matcher::Regex(r#""preimage""#.into()))
        .with_body(r#"{"status":"success","difficulty_target":4}"#)
        .expect(1)
        .create_async()
        .await;
    let w = seeded_wallet(&server).await;
    let b = w
        .mine_batch(&target(4, "50"), 1_700_000_000, 0, 1_000_000)
        .await
        .unwrap();
    report.assert_async().await;
    let r = b.result.expect("difficulty 4 is found");
    assert!(r.preimage.contains(&r.webcash));
    assert!(r.subsidy.as_deref().is_some_and(|s| r.preimage.contains(s)));
    assert_eq!(depth(&w, "MINING"), 2, "webcash + subsidy secrets reserved");
    assert_eq!(w.balance().await.unwrap(), "200");
    // The kept secret is the HD MINING chain secret at depth 0.
    let hd =
        webylib::hd::HdWallet::from_master_secret(hex::decode(MASTER).unwrap().try_into().unwrap());
    let expected = hd.derive_secret(webylib::hd::ChainCode::Mining, 0);
    assert_eq!(r.webcash, format!("e200:secret:{expected}"));
}

#[tokio::test]
async fn mine_batch_rejected_report_burns_secrets_but_stores_nothing() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/api/v1/mining_report")
        .with_status(400)
        .with_body(r#"{"error":"Didn't use a new secret"}"#)
        .create_async()
        .await;
    let w = seeded_wallet(&server).await;
    assert!(w
        .mine_batch(&target(4, "0"), 1_700_000_000, 0, 1_000_000)
        .await
        .is_err());
    assert_eq!(depth(&w, "MINING"), 1, "a secret is never offered twice");
    assert_eq!(w.balance().await.unwrap(), "0");
}

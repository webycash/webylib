//! Wallet ↔ server over the shared `server_client::wire` protocol, against a
//! mock server that answers exactly like webycash-server / webcash.org
//! (health_check keys normalised to `e{amount}:public:{hash}`).
#![cfg(feature = "native")]

use mockito::Matcher;
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

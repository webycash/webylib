use webylib::server::*;
use webylib::PublicWebcash;

#[test]
fn test_server_config_default() {
    let config = ServerConfig::default();
    assert_eq!(config.base_url(), "https://webcash.org");
    assert_eq!(config.timeout_seconds, 30);
}

#[tokio::test]
async fn test_server_client_creation() {
    let _client = ServerClient::new().unwrap();
}

#[test]
fn test_server_config_custom() {
    let config = ServerConfig {
        network: NetworkMode::Custom("https://test.webcash.org".to_string()),
        timeout_seconds: 60,
    };
    assert_eq!(config.base_url(), "https://test.webcash.org");
    assert_eq!(config.timeout_seconds, 60);
}

#[test]
fn test_network_mode_urls() {
    assert_eq!(NetworkMode::Production.base_url(), "https://webcash.org");
    assert_eq!(
        NetworkMode::Testnet.base_url(),
        "https://weby.cash/api/webcash/testnet"
    );
    assert_eq!(
        NetworkMode::Custom("http://localhost:8080".into()).base_url(),
        "http://localhost:8080"
    );
}

#[test]
fn test_request_response_structures() {
    // Test ReplaceRequest serialization
    let request = ReplaceRequest {
        webcashes: vec![
            "e1.00000000:secret:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
                .to_string(),
        ],
        new_webcashes: vec![
            "e0.50000000:secret:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .to_string(),
        ],
        legalese: Legalese { terms: true },
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("webcashes"));
    assert!(json.contains("new_webcashes"));
    assert!(json.contains("legalese"));
    assert!(json.contains("terms"));

    // Test MiningReportRequest serialization
    let report = MiningReportRequest {
        preimage: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
        legalese: Legalese { terms: true },
    };

    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("preimage"));
    assert!(json.contains("legalese"));
}

#[test]
fn test_response_parsing() {
    use webylib::server_client::{wire, TokenStatus};

    // health_check: the server normalises keys to `e{amount}:public:{hash}`;
    // statuses come back in request order, matched by hash.
    let h = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let requested = vec![format!("e1.00000000:public:{h}")];
    let health_json = format!(
        r#"{{"status":"success","results":{{"e1:public:{h}":{{"spent":false,"amount":"1"}}}}}}"#
    );
    let statuses = wire::parse_health_check(200, health_json.as_bytes(), &requested).unwrap();
    assert_eq!(
        statuses,
        vec![TokenStatus::Unspent {
            amount: Some("1".into())
        }]
    );

    let target_json = r#"{
        "difficulty_target_bits": 28,
        "epoch": 12345,
        "mining_amount": "200000",
        "mining_subsidy_amount": "10000",
        "ratio": 0.5
    }"#;
    let target_response: TargetResponse = wire::parse_target(200, target_json.as_bytes()).unwrap();
    assert_eq!(target_response.difficulty_target_bits, 28);
    assert_eq!(target_response.epoch, 12345);
    assert_eq!(target_response.mining_amount, "200000");
    assert_eq!(target_response.mining_subsidy_amount, "10000");
    assert_eq!(target_response.ratio, 0.5);

    let mining_json = r#"{"status": "success", "difficulty_target": 26}"#;
    let mining_response: MiningReportResponse =
        wire::parse_mining_report(200, mining_json.as_bytes()).unwrap();
    assert_eq!(mining_response.difficulty_target, Some(26));
}

#[test]
fn test_public_webcash_conversion() {
    // Test that PublicWebcash can be converted to string format for API requests
    let webcash = PublicWebcash::parse(
        "e1.00000000:public:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    )
    .unwrap();
    let webcash_str = format!("{}", webcash);

    // Verify the string format is correct
    assert_eq!(
        webcash_str,
        "e1:public:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
    );

    // Test with multiple webcash entries
    let webcash_list = [
        PublicWebcash::parse(
            "e1.00000000:public:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        )
        .unwrap(),
        PublicWebcash::parse(
            "e2.50000000:public:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        )
        .unwrap(),
    ];

    let string_list: Vec<String> = webcash_list.iter().map(|wc| format!("{}", wc)).collect();
    assert_eq!(string_list.len(), 2);
    assert!(string_list[0].starts_with("e1:public:"));
    assert!(string_list[1].starts_with("e2.5:public:"));
}

#[test]
fn test_empty_webcash_list() {
    // Test handling of empty webcash list
    let empty_list: Vec<PublicWebcash> = vec![];
    let string_list: Vec<String> = empty_list.iter().map(|wc| format!("{}", wc)).collect();
    assert!(string_list.is_empty());
}

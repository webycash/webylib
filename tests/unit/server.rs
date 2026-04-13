use webylib::server::*;
use webylib::PublicWebcash;

#[test]
fn test_server_config_default() {
    let config = ServerConfig::default();
    assert_eq!(config.base_url, "https://webcash.org");
    assert_eq!(config.timeout_seconds, 30);
}

#[tokio::test]
async fn test_server_client_creation() {
    let _client = ServerClient::new().unwrap();
    // Just test that it can be created without panicking
    assert!(true);
}

#[test]
fn test_server_config_custom() {
    let config = ServerConfig {
        base_url: "https://test.webcash.org".to_string(),
        timeout_seconds: 60,
    };
    assert_eq!(config.base_url, "https://test.webcash.org");
    assert_eq!(config.timeout_seconds, 60);
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
fn test_response_deserialization() {
    // Test HealthResponse deserialization
    let health_json = r#"{
        "status": "success",
        "results": {
            "e1.00000000:public:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890": {
                "spent": false,
                "amount": "1.00000000"
            }
        }
    }"#;

    let health_response: HealthResponse = serde_json::from_str(health_json).unwrap();
    assert_eq!(health_response.status, "success");
    assert_eq!(health_response.results.len(), 1);

    // Test TargetResponse deserialization
    let target_json = r#"{
        "difficulty_target_bits": 28,
        "epoch": 12345,
        "mining_amount": "10000000000000",
        "mining_subsidy_amount": "5000000000000",
        "ratio": 2.0
    }"#;

    let target_response: TargetResponse = serde_json::from_str(target_json).unwrap();
    assert_eq!(target_response.difficulty_target_bits, 28);
    assert_eq!(target_response.epoch, 12345);
    assert_eq!(target_response.mining_amount, "10000000000000");
    assert_eq!(target_response.mining_subsidy_amount, "5000000000000");
    assert_eq!(target_response.ratio, 2.0);

    // Test MiningReportResponse deserialization
    let mining_json = r#"{
        "status": "success",
        "difficulty_target": 26
    }"#;

    let mining_response: MiningReportResponse = serde_json::from_str(mining_json).unwrap();
    assert_eq!(mining_response.status, "success");
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
    let webcash_list = vec![
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

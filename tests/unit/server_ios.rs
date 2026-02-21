#[cfg(target_os = "ios")]
use webylib::{IOSServerClient, IOSServerConfig};

#[cfg(target_os = "ios")]
#[test]
fn test_ios_server_config_default() {
    let config = IOSServerConfig::default();
    assert_eq!(config.base_url, "https://webcash.org");
    assert_eq!(config.timeout_seconds, 30);
    assert!(config.tls_config.is_none());
}

#[cfg(target_os = "ios")]
#[tokio::test]
async fn test_ios_server_client_creation() {
    let _client = IOSServerClient::new().unwrap();
    // Just test that it can be created without panicking
    assert!(true);
}

#[cfg(not(target_os = "ios"))]
#[test]
fn test_ios_tests_skipped() {
    // This test runs on non-iOS platforms to indicate iOS tests are platform-specific
    println!("iOS-specific tests skipped on this platform");
}
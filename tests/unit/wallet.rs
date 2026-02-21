use webylib::{Wallet, Amount, Result};
use tempfile::NamedTempFile;

async fn create_test_wallet() -> Result<Wallet> {
    let temp_file = NamedTempFile::new().unwrap();
    let wallet_path = temp_file.path().with_extension("db");
    Wallet::open(&wallet_path).await
}

#[tokio::test]
async fn test_wallet_creation() {
    let wallet = create_test_wallet().await.unwrap();
    assert!(wallet.path().exists());

    // Check that tables were created
    let stats = wallet.stats().await.unwrap();
    assert_eq!(stats.total_webcash, 0);
    assert_eq!(stats.unspent_webcash, 0);
    assert_eq!(stats.spent_webcash, 0);
    assert_eq!(stats.total_balance, Amount::ZERO);
}
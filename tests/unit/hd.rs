use webylib::hd::{HDWallet, KeyType};

#[test]
fn test_hd_wallet_creation() {
    let wallet = HDWallet::new().unwrap();
    assert_eq!(wallet.master_secret().len(), 32);
}

#[test]
fn test_key_type_encoding() {
    assert_eq!(KeyType::Receive.to_encoding(), 0);
    assert_eq!(KeyType::Pay.to_encoding(), 1);
    assert_eq!(KeyType::Change.to_encoding(), 2);
    assert_eq!(KeyType::Mining.to_encoding(), 3);
}

#[test]
fn test_secret_derivation_deterministic() {
    let master_secret = [1u8; 32];
    let wallet = HDWallet::from_master_secret(master_secret);

    // Same inputs should give same output
    let secret1 = wallet.derive_secret(0, 0).unwrap();
    let secret2 = wallet.derive_secret(0, 0).unwrap();
    assert_eq!(secret1, secret2);

    // Different chain codes should give different outputs
    let secret3 = wallet.derive_secret(1, 0).unwrap();
    assert_ne!(secret1, secret3);

    // Different depths should give different outputs
    let secret4 = wallet.derive_secret(0, 1).unwrap();
    assert_ne!(secret1, secret4);
}

#[test]
fn test_secret_derivation_format() {
    let wallet = HDWallet::from_master_secret([42u8; 32]);
    let secret = wallet.derive_secret(0, 0).unwrap();

    // Should be valid hex
    assert!(hex::decode(&secret).is_ok());
    // Should be 32 bytes = 64 hex characters
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_generate_receive_secret() {
    let mut wallet = HDWallet::from_master_secret([1u8; 32]);
    let secret = wallet.generate_receive_secret(0).unwrap();

    // Should be valid hex and proper length
    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_generate_pay_secret() {
    let mut wallet = HDWallet::from_master_secret([2u8; 32]);
    let secret = wallet.generate_pay_secret(1).unwrap();

    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_generate_change_secret() {
    let mut wallet = HDWallet::from_master_secret([3u8; 32]);
    let secret = wallet.generate_change_secret(2).unwrap();

    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_generate_mining_secret() {
    let mut wallet = HDWallet::from_master_secret([4u8; 32]);
    let secret = wallet.generate_mining_secret(3).unwrap();

    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_master_secret_hex() {
    let master_secret = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0,
                        0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0,
                        0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0,
                        0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];

    let wallet = HDWallet::from_master_secret(master_secret);
    let hex_string = wallet.master_secret_hex();

    assert_eq!(hex_string, "123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0");
    assert_eq!(hex_string.len(), 64);
}

#[test]
fn test_domain_separation() {
    let wallet = HDWallet::from_master_secret([5u8; 32]);

    // All secrets for same index should be different due to different chain codes
    let receive_secret = wallet.derive_secret(0, 0).unwrap(); // RECEIVE = 0
    let pay_secret = wallet.derive_secret(1, 0).unwrap(); // PAY = 1
    let change_secret = wallet.derive_secret(2, 0).unwrap(); // CHANGE = 2
    let mining_secret = wallet.derive_secret(3, 0).unwrap(); // MINING = 3

    // All should be different
    let secrets = vec![receive_secret, pay_secret, change_secret, mining_secret];
    for i in 0..secrets.len() {
        for j in (i + 1)..secrets.len() {
            assert_ne!(secrets[i], secrets[j], "Secrets {} and {} should be different", i, j);
        }
    }
}
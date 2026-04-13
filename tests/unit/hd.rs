use webylib::hd::{ChainCode, HDWallet};

#[test]
fn test_hd_wallet_creation() {
    let wallet = HDWallet::new().unwrap();
    assert_eq!(wallet.master_secret().len(), 32);
}

#[test]
fn test_chain_code_values() {
    assert_eq!(ChainCode::Receive.as_u64(), 0);
    assert_eq!(ChainCode::Pay.as_u64(), 1);
    assert_eq!(ChainCode::Change.as_u64(), 2);
    assert_eq!(ChainCode::Mining.as_u64(), 3);
}

#[test]
fn test_chain_code_from_u64() {
    assert_eq!(ChainCode::from_u64(0), Some(ChainCode::Receive));
    assert_eq!(ChainCode::from_u64(1), Some(ChainCode::Pay));
    assert_eq!(ChainCode::from_u64(2), Some(ChainCode::Change));
    assert_eq!(ChainCode::from_u64(3), Some(ChainCode::Mining));
    assert_eq!(ChainCode::from_u64(4), None);
}

#[test]
fn test_chain_code_as_str() {
    assert_eq!(ChainCode::Receive.as_str(), "RECEIVE");
    assert_eq!(ChainCode::Pay.as_str(), "PAY");
    assert_eq!(ChainCode::Change.as_str(), "CHANGE");
    assert_eq!(ChainCode::Mining.as_str(), "MINING");
}

#[test]
fn test_secret_derivation_deterministic() {
    let master_secret = [1u8; 32];
    let wallet = HDWallet::from_master_secret(master_secret);

    let secret1 = wallet.derive_secret(ChainCode::Receive, 0).unwrap();
    let secret2 = wallet.derive_secret(ChainCode::Receive, 0).unwrap();
    assert_eq!(secret1, secret2);

    let secret3 = wallet.derive_secret(ChainCode::Pay, 0).unwrap();
    assert_ne!(secret1, secret3);

    let secret4 = wallet.derive_secret(ChainCode::Receive, 1).unwrap();
    assert_ne!(secret1, secret4);
}

#[test]
fn test_secret_derivation_format() {
    let wallet = HDWallet::from_master_secret([42u8; 32]);
    let secret = wallet.derive_secret(ChainCode::Receive, 0).unwrap();

    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_derive_receive_secret() {
    let wallet = HDWallet::from_master_secret([1u8; 32]);
    let secret = wallet.derive_secret(ChainCode::Receive, 0).unwrap();
    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_derive_pay_secret() {
    let wallet = HDWallet::from_master_secret([2u8; 32]);
    let secret = wallet.derive_secret(ChainCode::Pay, 1).unwrap();
    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_derive_change_secret() {
    let wallet = HDWallet::from_master_secret([3u8; 32]);
    let secret = wallet.derive_secret(ChainCode::Change, 2).unwrap();
    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_derive_mining_secret() {
    let wallet = HDWallet::from_master_secret([4u8; 32]);
    let secret = wallet.derive_secret(ChainCode::Mining, 3).unwrap();
    assert!(hex::decode(&secret).is_ok());
    assert_eq!(secret.len(), 64);
}

#[test]
fn test_master_secret_hex() {
    let master_secret = [
        0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE,
        0xF0, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC,
        0xDE, 0xF0,
    ];
    let wallet = HDWallet::from_master_secret(master_secret);
    let hex_string = wallet.master_secret_hex();
    assert_eq!(
        hex_string,
        "123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0"
    );
    assert_eq!(hex_string.len(), 64);
}

#[test]
fn test_domain_separation() {
    let wallet = HDWallet::from_master_secret([5u8; 32]);

    let receive = wallet.derive_secret(ChainCode::Receive, 0).unwrap();
    let pay = wallet.derive_secret(ChainCode::Pay, 0).unwrap();
    let change = wallet.derive_secret(ChainCode::Change, 0).unwrap();
    let mining = wallet.derive_secret(ChainCode::Mining, 0).unwrap();

    let secrets = vec![receive, pay, change, mining];
    for i in 0..secrets.len() {
        for j in (i + 1)..secrets.len() {
            assert_ne!(
                secrets[i], secrets[j],
                "Secrets {} and {} should be different",
                i, j
            );
        }
    }
}

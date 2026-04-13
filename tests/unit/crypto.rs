use webylib::crypto::*;

#[test]
fn test_sha256() {
    let data = b"hello world";
    let hash = sha256(data);
    assert_eq!(hash.len(), 32);

    // Test consistency
    let hash2 = sha256(data);
    assert_eq!(hash, hash2);
}

#[test]
fn test_sha256_string() {
    let data = "webcash";
    let hash = sha256_string(data);
    assert_eq!(hash.len(), 32);
}

#[test]
fn test_generate_secret() {
    let secret1 = generate_secret().unwrap();
    let secret2 = generate_secret().unwrap();

    // Should be different (extremely unlikely to be the same)
    assert_ne!(secret1, secret2);

    // Should be valid hex
    assert!(hex::decode(&secret1).is_ok());
    assert_eq!(secret1.len(), 64); // 32 bytes = 64 hex chars
}

#[test]
fn test_verify_hash() {
    let data = b"test data";
    let hash = sha256(data);
    assert!(verify_hash(data, &hash));

    let wrong_hash = sha256(b"wrong data");
    assert!(!verify_hash(data, &wrong_hash));
}

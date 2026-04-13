use sha2::{Digest, Sha256};
use webylib::{PublicWebcash, SecretWebcash};

#[test]
fn test_secret_webcash_parse() {
    let secret = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let amount_str = "1.00000000";
    let webcash_str = format!("e{}:secret:{}", amount_str, secret);

    let webcash = SecretWebcash::parse(&webcash_str).unwrap();
    assert_eq!(webcash.secret.as_str().unwrap(), secret);
    assert_eq!(format!("{}", webcash.amount), "1"); // Amount normalizes trailing zeros
    assert_eq!(
        format!("{}", webcash),
        format!("e{}:secret:{}", "1", secret)
    );
}

#[test]
fn test_secret_webcash_to_public() {
    let secret = SecretWebcash::parse(
        "e1.00000000:secret:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    )
    .unwrap();
    let public = secret.to_public();

    // Verify the hash is correct - should hash the ASCII string bytes (matches Python)
    // Python: hashlib.sha256(bytes(str(secret_value), "ascii")).hexdigest()
    let secret_str = secret.secret.as_str().unwrap();
    let expected_hash = Sha256::digest(secret_str.as_bytes());
    let expected_array: [u8; 32] = expected_hash.into();

    assert_eq!(public.hash, expected_array);
    assert_eq!(public.amount, secret.amount);

    println!("Secret: {}", secret_str);
    println!("Public webcash: {}", public.to_string());
    println!("Hash hex: {}", public.hash_hex());
}

#[test]
fn test_public_webcash_parse() {
    let hash = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    let amount_str = "1.00000000";
    let webcash_str = format!("e{}:public:{}", amount_str, hash);

    let webcash = PublicWebcash::parse(&webcash_str).unwrap();
    assert_eq!(webcash.hash_hex(), hash);
    assert_eq!(format!("{}", webcash.amount), "1"); // Amount normalizes trailing zeros
    assert_eq!(format!("{}", webcash), format!("e{}:public:{}", "1", hash));
}

#[test]
fn test_webcash_formats() {
    // Test with ₩ prefix
    let webcash = PublicWebcash::parse(
        "₩1.00000000:public:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    )
    .unwrap();
    assert_eq!(format!("{}", webcash.amount), "1");

    // Test without prefix (should work for public)
    let webcash2 = PublicWebcash::parse(
        "1.00000000:public:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    )
    .unwrap();
    assert_eq!(webcash, webcash2);
}

#[test]
fn test_invalid_formats() {
    // Invalid secret format
    assert!(SecretWebcash::parse("invalid").is_err());
    assert!(SecretWebcash::parse("e1.0:secret:invalid").is_err());
    assert!(SecretWebcash::parse("e1.0:public:1234").is_err());

    // Invalid public format
    assert!(PublicWebcash::parse("invalid").is_err());
    assert!(PublicWebcash::parse("e1.0:secret:1234").is_err());
    assert!(PublicWebcash::parse("e1.0:public:invalid").is_err());
}

#[test]
fn test_round_trip() {
    let original =
        "e1.50000000:secret:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let secret = SecretWebcash::parse(original).unwrap();
    let public = secret.to_public();

    // Secret round trip - Amount normalizes trailing zeros
    let expected_secret =
        "e1.5:secret:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    assert_eq!(format!("{}", secret), expected_secret);

    // Public serialization
    let public_str = format!("{}", public);
    let public_parsed = PublicWebcash::parse(&public_str).unwrap();
    assert_eq!(public, public_parsed);
}

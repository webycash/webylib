//! Cryptographic utilities for Webcash
//!
//! This module provides cryptographic functions used throughout the Webcash system,
//! including SHA256 hashing, HMAC-SHA512, secure random number generation, and
//! biometric encryption capabilities.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Compute SHA256 hash of data
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Create incremental SHA256 hasher (matches Python hashlib.sha256 pattern)
pub fn sha256_incremental() -> Sha256 {
    Sha256::new()
}

/// Finalize incremental SHA256 hash
pub fn sha256_finalize(hasher: Sha256) -> [u8; 32] {
    hasher.finalize().into()
}

/// Compute SHA256 hash of string data
pub fn sha256_string(data: &str) -> [u8; 32] {
    sha256(data.as_bytes())
}

/// A cryptographically secure 32-byte secret with automatic memory zeroing
#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct CryptoSecret([u8; 32]);

impl CryptoSecret {
    /// Generate a new cryptographically secure 32-byte secret using platform RNG
    ///
    /// This uses getrandom which interfaces with:
    /// - iOS: SecRandomCopyBytes (Secure Enclave hardware RNG)
    /// - Android: /dev/urandom (hardware RNG pool)
    /// - Other platforms: OS-level secure RNG
    pub fn generate() -> Result<Self, CryptoError> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes)
            .map_err(|e| CryptoError::RandomGeneration(format!("{}", e)))?;
        Ok(CryptoSecret(bytes))
    }

    /// Create from existing bytes (use carefully - prefer generate())
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        CryptoSecret(bytes)
    }

    /// Create from hex string
    pub fn from_hex(hex_str: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(hex_str).map_err(|_| CryptoError::InvalidHex)?;
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidLength);
        }
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(CryptoSecret(array))
    }

    /// Get reference to the secret bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert to array (consumes self)
    pub fn into_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }

    /// Get the raw key bytes (for compatibility with encryption libraries)
    pub fn as_key_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Create an AES-256-GCM cipher from this secret
    pub fn create_cipher(&self) -> aes_gcm::Aes256Gcm {
        use aes_gcm::{Aes256Gcm, KeyInit};
        Aes256Gcm::new_from_slice(&self.0).expect("32-byte key is valid for AES-256-GCM")
    }
}

// Prevent accidental logging of secrets
impl fmt::Debug for CryptoSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CryptoSecret([REDACTED])")
    }
}

impl fmt::Display for CryptoSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED 32-byte secret]")
    }
}

/// Cryptography-related errors
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Failed to generate random bytes: {0}")]
    RandomGeneration(String),

    #[error("Invalid hex string format")]
    InvalidHex,

    #[error("Invalid secret length - must be 32 bytes")]
    InvalidLength,

    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Key derivation failed: {0}")]
    KeyDerivationFailed(String),
}

/// Generate a random 32-byte secret as hex string (legacy function)
pub fn generate_secret() -> std::result::Result<String, Box<dyn std::error::Error>> {
    let secret = CryptoSecret::generate()?;
    Ok(secret.to_hex())
}

/// Generate a random 32-byte array (legacy function)
pub fn generate_random_bytes() -> [u8; 32] {
    CryptoSecret::generate()
        .expect("Platform RNG should always work")
        .into_bytes()
}

/// Compute HMAC-SHA512
pub fn hmac_sha512(key: &[u8], data: &[u8]) -> [u8; 64] {
    type HmacSha512 = Hmac<Sha512>;
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC key too long");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// Verify that a hash matches the expected value
pub fn verify_hash(data: &[u8], expected_hash: &[u8; 32]) -> bool {
    &sha256(data) == expected_hash
}

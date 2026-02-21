//! Webcash types and serialization
//!
//! This module contains the core Webcash data structures:
//! - `SecretWebcash`: Contains the secret value and amount
//! - `PublicWebcash`: Contains the public hash and amount
//!
//! Both types support serialization to/from the standard Webcash string format.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::amount::Amount;
use crate::error::{Error, Result};

/// UTF-8 byte length of the ₩ symbol
const WEBCASH_SYMBOL_BYTES: usize = 3;

/// Secure string type for sensitive data with zeroize-on-drop
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecureString(Vec<u8>);

impl SecureString {
    /// Create a new SecureString from a string
    pub fn new<S: Into<String>>(s: S) -> Self {
        let string = s.into();
        let bytes = string.into_bytes();
        SecureString(bytes)
    }

    /// Create a new SecureString from bytes
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        SecureString(bytes)
    }

    /// Get the string value (use with caution - data remains in memory)
    pub fn as_str(&self) -> std::result::Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.0)
    }

    /// Get the raw bytes (use with caution)
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Get the length
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }
}

impl fmt::Debug for SecureString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecureString([redacted, {} bytes])", self.0.len())
    }
}

impl fmt::Display for SecureString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[redacted]")
    }
}

impl Drop for SecureString {
    fn drop(&mut self) {
        // Secure zeroization of sensitive data
        self.0.iter_mut().for_each(|byte| *byte = 0);
    }
}

/// Secret Webcash containing the actual secret value
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretWebcash {
    /// The secret value (32 bytes hex)
    pub secret: SecureString,
    /// The amount
    pub amount: Amount,
}

impl SecretWebcash {
    /// Create a new SecretWebcash
    pub fn new(secret: SecureString, amount: Amount) -> Self {
        SecretWebcash { secret, amount }
    }

    /// Parse from Webcash string format
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if !s.starts_with('e') {
            return Err(Error::parse("SecretWebcash must start with 'e'"));
        }

        let parts: Vec<&str> = s[1..].split(':').collect();
        if parts.len() < 3 {
            return Err(Error::parse("Invalid SecretWebcash format"));
        }

        if parts[1] != "secret" {
            return Err(Error::parse("Expected 'secret' type"));
        }

        let amount_str = parts[0];
        let amount = Amount::from_str(amount_str)?;

        let secret = parts[2..].join(":");

        // Validate secret is valid hex and 32 bytes
        if secret.len() != 64 {
            return Err(Error::parse("Secret must be 64 hex characters (32 bytes)"));
        }

        hex::decode(&secret).map_err(|_| Error::parse("Secret must be valid hex"))?;

        Ok(SecretWebcash {
            secret: SecureString::new(secret),
            amount,
        })
    }

    /// Convert to PublicWebcash
    /// CRITICAL: Hash must match Python implementation exactly
    /// Python: hashlib.sha256(bytes(str(secret_value), "ascii")).hexdigest()
    /// This means we hash the secret STRING as ASCII bytes, NOT the hex-decoded bytes
    pub fn to_public(&self) -> PublicWebcash {
        let secret_str = self.secret.as_str().unwrap_or("");
        // Hash the ASCII string representation of the secret (matches Python implementation)
        // Python does: hashlib.sha256(bytes(str(secret_value), "ascii")).hexdigest()
        let hash = Sha256::digest(secret_str.as_bytes());
        PublicWebcash {
            hash: hash.into(),
            amount: self.amount,
        }
    }

    /// Serialize to Webcash string format
    /// Webcash strings use DECIMAL format for amounts (e.g., e0.0001:secret:...)
    /// The amount prefix is the decimal representation, not wats
    pub fn to_webcash_string(&self) -> String {
        let secret_str = self.secret.as_str().unwrap_or("");
        // Use decimal format (Display trait formats as decimal)
        format!("e{}:secret:{}", self.amount, secret_str)
    }

}

impl fmt::Display for SecretWebcash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_webcash_string())
    }
}

impl FromStr for SecretWebcash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// Public Webcash containing the hash of the secret
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicWebcash {
    /// SHA256 hash of the secret (32 bytes)
    pub hash: [u8; 32],
    /// The amount
    pub amount: Amount,
}

impl PublicWebcash {
    /// Create a new PublicWebcash
    pub fn new(hash: [u8; 32], amount: Amount) -> Self {
        PublicWebcash { hash, amount }
    }

    /// Parse from Webcash string format
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();

        // Handle different prefixes
        let s = if let Some(stripped) = s.strip_prefix('e') {
            stripped
        } else if s.starts_with('₩') {
            &s[WEBCASH_SYMBOL_BYTES..]
        } else {
            s
        };

        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return Err(Error::parse("Invalid PublicWebcash format"));
        }

        if parts[1] != "public" {
            return Err(Error::parse("Expected 'public' type"));
        }

        let amount_str = parts[0];
        let amount = Amount::from_str(amount_str)?;

        let hash_str = parts[2];
        if hash_str.len() != 64 {
            return Err(Error::parse("Hash must be 64 hex characters (32 bytes)"));
        }

        let hash_bytes = hex::decode(hash_str)
            .map_err(|_| Error::parse("Hash must be valid hex"))?;

        let mut hash = [0u8; 32];
        hash.copy_from_slice(&hash_bytes);

        Ok(PublicWebcash { hash, amount })
    }

    /// Get the hash as a hex string
    pub fn hash_hex(&self) -> String {
        hex::encode(self.hash)
    }

    /// Serialize to Webcash string format
    /// Webcash strings use DECIMAL format for amounts (e.g., e0.0001:public:...)
    /// The amount prefix is the decimal representation, not wats
    pub fn to_webcash_string(&self) -> String {
        // Use decimal format (Display trait formats as decimal)
        format!("e{}:public:{}", self.amount, self.hash_hex())
    }
}

impl fmt::Display for PublicWebcash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_webcash_string())
    }
}

impl FromStr for PublicWebcash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl From<&SecretWebcash> for PublicWebcash {
    fn from(secret: &SecretWebcash) -> Self {
        secret.to_public()
    }
}


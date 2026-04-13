//! Hierarchical Deterministic (HD) wallet key derivation.
//!
//! Implements the Webcash HD derivation scheme (compatible with the Python
//! reference implementation). Four chain codes partition the key space:
//!
//! | Chain | Code | Purpose |
//! |---------|------|-------------------------------|
//! | Receive | 0 | Incoming payments |
//! | Pay | 1 | Outgoing payment outputs |
//! | Change | 2 | Transaction change |
//! | Mining | 3 | Mining reward collection |
//!
//! ## Derivation Algorithm
//!
//! ```text
//! tag = SHA256("webcashwalletv1")
//! secret = SHA256(tag ‖ tag ‖ master_secret ‖ chain_code_be64 ‖ depth_be64)
//! ```

use sha2::Digest;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::crypto;
use crate::error::{Error, Result};

/// Chain codes for HD wallet derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainCode {
    /// Keys for receiving payments.
    Receive = 0,
    /// Keys for making payments.
    Pay = 1,
    /// Keys for transaction change.
    Change = 2,
    /// Keys for mining rewards.
    Mining = 3,
}

impl ChainCode {
    /// Convert from a u64 value.
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(ChainCode::Receive),
            1 => Some(ChainCode::Pay),
            2 => Some(ChainCode::Change),
            3 => Some(ChainCode::Mining),
            _ => None,
        }
    }

    /// Convert to u64.
    pub fn as_u64(self) -> u64 {
        self as u64
    }

    /// Get the canonical string name (matches Python walletdepths keys).
    pub fn as_str(self) -> &'static str {
        match self {
            ChainCode::Receive => "RECEIVE",
            ChainCode::Pay => "PAY",
            ChainCode::Change => "CHANGE",
            ChainCode::Mining => "MINING",
        }
    }
}

/// HD wallet with secure key derivation from a 32-byte master secret.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct HDWallet {
    master_secret: [u8; 32],
}

impl HDWallet {
    /// Create a new HD wallet with a cryptographically secure random master secret.
    pub fn new() -> Result<Self> {
        let secret = crypto::CryptoSecret::generate()
            .map_err(|e| Error::crypto(format!("Failed to generate master secret: {}", e)))?;
        Ok(HDWallet {
            master_secret: secret.into_bytes(),
        })
    }

    /// Create HD wallet from an existing master secret.
    pub fn from_master_secret(master_secret: [u8; 32]) -> Self {
        HDWallet { master_secret }
    }

    /// Derive a secret for the given chain code and depth.
    ///
    /// Uses the exact algorithm from the Python reference implementation:
    /// ```text
    /// tag = SHA256("webcashwalletv1")
    /// H = SHA256(tag ‖ tag ‖ master_secret ‖ chain_code_be64 ‖ depth_be64)
    /// ```
    pub fn derive_secret(&self, chain_code: ChainCode, depth: u64) -> Result<String> {
        let tag = crypto::sha256(b"webcashwalletv1");
        let mut hasher = crypto::sha256_incremental();
        hasher.update(tag);
        hasher.update(tag);
        hasher.update(self.master_secret);
        hasher.update(chain_code.as_u64().to_be_bytes());
        hasher.update(depth.to_be_bytes());
        let final_hash = crypto::sha256_finalize(hasher);
        Ok(hex::encode(final_hash))
    }

    /// Get a reference to the master secret (use with caution — only for backup/recovery).
    pub fn master_secret(&self) -> &[u8; 32] {
        &self.master_secret
    }

    /// Get master secret as hex string for backup.
    pub fn master_secret_hex(&self) -> String {
        hex::encode(self.master_secret)
    }
}

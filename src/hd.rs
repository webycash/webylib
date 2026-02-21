//! HD (Hierarchical Deterministic) wallet functionality
//!
//! This module implements secure hierarchical deterministic key generation
//! for Webcash wallets, following the pattern used in the C++ reference implementation.

use crate::crypto;
use sha2::Digest;

/// Key types for domain separation in HD wallet
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    /// Keys for receiving payments (mine=false, sweep=true)
    Receive = 0,
    /// Keys for making payments (mine=false, sweep=false)
    Pay = 1,
    /// Keys for transaction change (mine=true, sweep=false)
    Change = 2,
    /// Keys for mining rewards (mine=true, sweep=true)
    Mining = 3,
}

impl KeyType {
    /// Convert key type to the encoding used in derivation
    pub fn to_encoding(self) -> u8 {
        match self {
            KeyType::Receive => 0,
            KeyType::Pay => 1,
            KeyType::Change => 2,
            KeyType::Mining => 3,
        }
    }
}

/// HD wallet implementation with secure key derivation
pub struct HDWallet {
    /// Master secret key (32 bytes)
    master_secret: [u8; 32],
}

impl HDWallet {
    /// Create a new HD wallet with a cryptographically secure random master secret
    pub fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let master_secret = crypto::generate_random_bytes();
        Ok(HDWallet { master_secret })
    }

    /// Create HD wallet from existing master secret
    pub fn from_master_secret(master_secret: [u8; 32]) -> Self {
        HDWallet { master_secret }
    }

    /// Derive a secret using the exact algorithm from Python reference implementation
    pub fn derive_secret(&self, chain_code: u64, depth: u64) -> std::result::Result<String, Box<dyn std::error::Error>> {
        // Domain tag for Webcash wallet (matches Python exactly)
        let tag_str = "webcashwalletv1";
        let tag = crypto::sha256(tag_str.as_bytes());

        // Create incremental hasher and initialize with tag + tag (matches Python)
        let mut hasher = crypto::sha256_incremental();
        hasher.update(tag);
        hasher.update(tag);

        // Update with master secret (matches Python)
        hasher.update(self.master_secret);

        // Update with chain code (big-endian 8 bytes, matches Python struct.pack(">Q", CHAIN_CODES[chain_code]))
        let chaincode_bytes = chain_code.to_be_bytes();
        hasher.update(chaincode_bytes);

        // Update with depth (big-endian 8 bytes, matches Python struct.pack(">Q", walletdepth))
        let depth_bytes = depth.to_be_bytes();
        hasher.update(depth_bytes);

        // Finalize and convert to hex string (matches Python)
        let final_hash = crypto::sha256_finalize(hasher);
        Ok(hex::encode(final_hash))
    }

    /// Generate a new secret for receiving payments (matches Python generate_new_secret with chain_code="RECEIVE")
    pub fn generate_receive_secret(&mut self, depth: u64) -> std::result::Result<String, Box<dyn std::error::Error>> {
        self.derive_secret(0, depth) // CHAIN_CODES["RECEIVE"] = 0
    }

    /// Generate a new secret for making payments (matches Python generate_new_secret with chain_code="PAY")
    pub fn generate_pay_secret(&mut self, depth: u64) -> std::result::Result<String, Box<dyn std::error::Error>> {
        self.derive_secret(1, depth) // CHAIN_CODES["PAY"] = 1
    }

    /// Generate a new secret for transaction change (matches Python generate_new_secret with chain_code="CHANGE")
    pub fn generate_change_secret(&mut self, depth: u64) -> std::result::Result<String, Box<dyn std::error::Error>> {
        self.derive_secret(2, depth) // CHAIN_CODES["CHANGE"] = 2
    }

    /// Generate a new secret for mining rewards (matches Python generate_new_secret with chain_code="MINING")
    pub fn generate_mining_secret(&mut self, depth: u64) -> std::result::Result<String, Box<dyn std::error::Error>> {
        self.derive_secret(3, depth) // CHAIN_CODES["MINING"] = 3
    }

    /// Get the master secret (use with caution - only for backup/recovery)
    pub fn master_secret(&self) -> &[u8; 32] {
        &self.master_secret
    }

    /// Get master secret as hex string for backup
    pub fn master_secret_hex(&self) -> String {
        hex::encode(self.master_secret)
    }
}

impl Drop for HDWallet {
    fn drop(&mut self) {
        // Secure cleanup - zeroize the master secret
        self.master_secret = [0u8; 32];
    }
}


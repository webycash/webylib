//! # Webcash Wallet Library
//!
//! A secure, efficient Rust implementation of the Webcash electronic cash wallet.
//!
//! This library provides:
//! - Deterministic HD wallet functionality
//! - Secure storage using SQLite with biometric encryption
//! - Server communication for Webcash operations
//! - Biometric authentication (Face ID/Touch ID on iOS, Biometric API on Android)
//! - State-of-the-art cryptographic security with hardware RNG
//! - C bindings for interoperability
//!
//! ## Example
//!
//! ```rust,no_run
//! use webylib::{Wallet, SecretWebcash};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create or open a wallet
//! let wallet = Wallet::open("my_wallet.db").await?;
//!
//! // Insert webcash
//! let webcash = SecretWebcash::parse("e1.00000000:secret:abcdef123456...")?;
//! wallet.insert(webcash).await?;
//!
//! // Check balance
//! let balance = wallet.balance().await?;
//! println!("Balance: {}", balance);
//! # Ok(())
//! # }
//! ```

pub mod amount;
pub mod error;
pub mod webcash;
pub mod crypto;
pub mod wallet;
pub mod server;
pub mod hd;
pub mod biometric;

pub use amount::Amount;
pub use error::{Error, Result};
pub use webcash::{SecretWebcash, PublicWebcash};
pub use wallet::Wallet;

// Conditionally export iOS-specific client
#[cfg(target_os = "ios")]
pub use server::ios::{IOSServerClient, IOSServerConfig, IOSCustomTLS};

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Webcash server endpoints
pub mod endpoints {
    pub const HEALTH_CHECK: &str = "/api/v1/health_check";
    pub const REPLACE: &str = "/api/v1/replace";
    pub const TARGET: &str = "/api/v1/target";
    pub const MINING_REPORT: &str = "/api/v1/mining_report";
}

/// Chain codes for HD wallet
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainCode {
    Receive = 0,
    Pay = 1,
    Change = 2,
    Mining = 3,
}

impl ChainCode {
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(ChainCode::Receive),
            1 => Some(ChainCode::Pay),
            2 => Some(ChainCode::Change),
            3 => Some(ChainCode::Mining),
            _ => None,
        }
    }

    pub fn as_u64(self) -> u64 {
        self as u64
    }
}

/// Terms of service that must be accepted
pub const TERMS_OF_SERVICE: &str = "I acknowledge and agree to the Terms of Service located at https://webcash.org/terms";


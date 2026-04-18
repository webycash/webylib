#![allow(clippy::await_holding_lock)]
//! # webylib — Webcash HD Wallet Library
//!
//! A secure, production-grade Rust implementation of the Webcash electronic cash protocol.
//!
//! ## Features
//!
//! - Deterministic HD wallet (BIP32-style 4-chain derivation)
//! - SQLite storage with WAL mode for crash safety
//! - Passkey and password-based encryption (Argon2 + AES-256-GCM)
//! - Full server client (health check, replace, mining)
//! - Zeroize-on-drop for all secret material
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use webylib::{Wallet, SecretWebcash};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let wallet = Wallet::open("my_wallet.db").await?;
//! let webcash = SecretWebcash::parse("e1.00000000:secret:abcdef123456...")?;
//! wallet.insert(webcash).await?;
//! let balance = wallet.balance().await?;
//! # Ok(())
//! # }
//! ```

// ── Always available (WASM-safe) ────────────────────────────────
pub mod amount;
pub mod crypto;
pub mod error;
pub mod hd;
pub mod protocol;
pub mod server;
pub mod webcash;

// ── Native-only (SQLite, tokio, reqwest, keyring) ───────────────
#[cfg(feature = "native")]
pub mod passkey;
#[cfg(any(feature = "native", feature = "wasm"))]
pub mod wallet;
#[cfg(feature = "native")]
pub mod miner;
#[cfg(feature = "ffi")]
pub mod ffi;

// ── Re-exports ──────────────────────────────────────────────────
pub use amount::Amount;
pub use error::{Error, Result};
pub use hd::ChainCode;
pub use protocol::{TERMS_OF_SERVICE, VERSION};
pub use server::endpoints;
pub use server::NetworkMode;
pub use webcash::{PublicWebcash, SecretWebcash};

#[cfg(any(feature = "native", feature = "wasm"))]
pub use wallet::{Wallet, WalletSnapshot, WalletStats};

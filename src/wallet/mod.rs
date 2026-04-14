//! Wallet engine — SQLite-backed HD wallet for Webcash.
//!
//! The wallet is the primary interface for managing webcash: storing secrets,
//! making payments, recovering from master seeds, and encrypting data at rest.
//!
//! # Architecture
//!
//! - **`schema`** — Database schema, WAL mode, migrations.
//! - **`operations`** — Insert, pay, merge, recover, check, balance.
//! - **`encryption`** — Database-level and seed-level encryption.
//! - **`snapshot`** — JSON export/import for backup and recovery.
//!
//! The `Wallet` struct owns a `Mutex<Connection>` for thread-safe DB access
//! and a `Mutex<Box<dyn ServerClientTrait>>` for server communication.
//! Each wallet instance is self-contained — no global singletons.

pub mod encryption;
pub mod operations;
pub mod schema;
pub mod snapshot;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::passkey::{EncryptionConfig, PasskeyEncryption};
use crate::server::{NetworkMode, ServerClient, ServerClientTrait, ServerConfig};

// Re-export types consumers need
pub use operations::{CheckResult, RecoveryResult, WalletStats};
pub use snapshot::{SpentHashSnapshot, UnspentOutputSnapshot, WalletSnapshot};

/// SQLite-backed Webcash wallet.
pub struct Wallet {
    /// Path to the wallet database file.
    pub(crate) path: PathBuf,
    /// SQLite connection (Mutex for thread safety).
    pub(crate) connection: Mutex<Connection>,
    /// Server client for webcash operations (tokio Mutex: safe to hold across .await).
    pub(crate) server_client: tokio::sync::Mutex<Box<dyn ServerClientTrait + Send>>,
    /// Passkey encryption handler (optional).
    pub(crate) passkey_encryption: Option<Mutex<PasskeyEncryption>>,
    /// Whether this wallet uses runtime encryption.
    pub(crate) is_encrypted: bool,
    /// Temporary path for decrypted database during runtime.
    pub(crate) temp_db_path: Option<PathBuf>,
    /// Network mode (Production, Testnet, or Custom URL).
    pub(crate) network: NetworkMode,
}

impl Wallet {
    /// Open or create a wallet at the given path.
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_passkey(path, false).await
    }

    /// Open or create a wallet with optional passkey encryption.
    pub async fn open_with_passkey<P: AsRef<Path>>(path: P, enable_passkey: bool) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (connection, is_encrypted, temp_db_path) =
            if enable_passkey && Self::is_database_encrypted(&path)? {
                let temp_path = Self::decrypt_database_for_runtime(&path).await?;
                let connection = Connection::open(&temp_path)?;
                (connection, true, Some(temp_path))
            } else if enable_passkey {
                let connection = Connection::open(&path)?;
                (connection, true, None)
            } else {
                let connection = Connection::open(&path)?;
                (connection, false, None)
            };

        schema::initialize_schema(&connection)?;

        let passkey_encryption = if enable_passkey {
            let config = EncryptionConfig {
                app_identifier: "com.webycash.webylib".to_string(),
                service_name: format!(
                    "WalletEncryption_{}",
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("default")
                ),
                require_auth_every_use: true,
                auth_timeout_seconds: 0,
                allow_device_passcode_fallback: true,
            };
            PasskeyEncryption::new(config).ok().map(Mutex::new)
        } else {
            None
        };

        let server_client: Box<dyn ServerClientTrait + Send> = Box::new(ServerClient::new()?);

        let wallet = Wallet {
            path,
            connection: Mutex::new(connection),
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption,
            is_encrypted,
            temp_db_path,
            network: NetworkMode::Production,
        };

        let _ = wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Open or create a wallet targeting a specific network.
    pub async fn open_with_network<P: AsRef<Path>>(path: P, network: NetworkMode) -> Result<Self> {
        let config = ServerConfig {
            network: network.clone(),
            timeout_seconds: 30,
        };
        let server_client: Box<dyn ServerClientTrait + Send> =
            Box::new(ServerClient::with_config(config)?);

        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&path)?;
        schema::initialize_schema(&connection)?;

        let wallet = Wallet {
            path,
            connection: Mutex::new(connection),
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption: None,
            is_encrypted: false,
            temp_db_path: None,
            network,
        };
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Open or create a wallet with a caller-provided seed.
    /// If the wallet already has a different master secret, returns an error.
    pub async fn open_with_seed<P: AsRef<Path>>(path: P, seed: &[u8; 32]) -> Result<Self> {
        let wallet = Self::open(path).await?;
        let hex = hex::encode(seed);

        // Check if there's already a different master secret
        let existing = wallet.master_secret_hex()?;
        if existing != hex {
            // If the wallet was just created, the auto-generated secret is fine to overwrite
            // But if it has transactions, refuse
            let stats = wallet.stats().await?;
            if stats.total_webcash > 0 {
                return Err(Error::wallet(
                    "Wallet already has a different master secret with existing transactions",
                ));
            }
            wallet.store_master_secret(&hex).await?;
        }
        Ok(wallet)
    }

    /// Open an in-memory wallet (useful for testing or stateless execution).
    pub fn open_memory() -> Result<Self> {
        Self::open_memory_with_network(NetworkMode::Production)
    }

    /// Open an in-memory wallet targeting a specific network.
    pub fn open_memory_with_network(network: NetworkMode) -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        schema::initialize_schema(&connection)?;

        let config = ServerConfig {
            network: network.clone(),
            timeout_seconds: 30,
        };
        let server_client: Box<dyn ServerClientTrait + Send> =
            Box::new(ServerClient::with_config(config)?);

        let wallet = Wallet {
            path: PathBuf::from(":memory:"),
            connection: Mutex::new(connection),
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption: None,
            is_encrypted: false,
            temp_db_path: None,
            network,
        };
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Get the wallet database path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the network mode this wallet targets.
    pub fn network(&self) -> &NetworkMode {
        &self.network
    }

    /// Close the wallet (flushes any pending operations).
    pub async fn close(mut self) -> Result<()> {
        if self.is_encrypted {
            // Drop the SQLite connection BEFORE encrypting to ensure the DB file
            // is fully flushed and won't be overwritten when `self` is dropped.
            let placeholder = Connection::open_in_memory().map_err(|e| {
                Error::wallet(format!("Failed to create placeholder connection: {}", e))
            })?;
            let old_connection = std::mem::replace(&mut self.connection, Mutex::new(placeholder));
            let connection = old_connection
                .into_inner()
                .map_err(|_| Error::wallet("Failed to acquire database lock during close"))?;
            drop(connection);

            self.encrypt_database().await?;
        }
        if let Some(passkey_mutex) = self.passkey_encryption.take() {
            let mut passkey = passkey_mutex
                .into_inner()
                .map_err(|_| Error::wallet("Failed to acquire passkey lock during close"))?;
            passkey.clear_cached_keys();
        }
        Ok(())
    }
}

impl Drop for Wallet {
    fn drop(&mut self) {
        if self.is_encrypted {
            if let Some(temp_path) = &self.temp_db_path {
                let _ = std::fs::remove_file(temp_path);
            }
        }
    }
}

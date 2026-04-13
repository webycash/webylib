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

use crate::biometric::{BiometricEncryption, EncryptionConfig};
use crate::error::{Error, Result};
use crate::server::{ServerClient, ServerClientTrait};

// Re-export types consumers need
pub use operations::{CheckResult, RecoveryResult, WalletStats};
pub use snapshot::{SpentHashSnapshot, UnspentOutputSnapshot, WalletSnapshot};

/// SQLite-backed Webcash wallet.
pub struct Wallet {
    /// Path to the wallet database file.
    pub(crate) path: PathBuf,
    /// SQLite connection (Mutex for thread safety).
    pub(crate) connection: Mutex<Connection>,
    /// Server client for webcash operations.
    pub(crate) server_client: Mutex<Box<dyn ServerClientTrait + Send>>,
    /// Biometric encryption handler (optional).
    pub(crate) biometric_encryption: Option<Mutex<BiometricEncryption>>,
    /// Whether this wallet uses runtime encryption.
    pub(crate) is_encrypted: bool,
    /// Temporary path for decrypted database during runtime.
    pub(crate) temp_db_path: Option<PathBuf>,
}

impl Wallet {
    /// Open or create a wallet at the given path.
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_biometric(path, false).await
    }

    /// Open or create a wallet with optional biometric encryption.
    pub async fn open_with_biometric<P: AsRef<Path>>(
        path: P,
        enable_biometric: bool,
    ) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (connection, is_encrypted, temp_db_path) =
            if enable_biometric && Self::is_database_encrypted(&path)? {
                let temp_path = Self::decrypt_database_for_runtime(&path).await?;
                let connection = Connection::open(&temp_path)?;
                (connection, true, Some(temp_path))
            } else if enable_biometric {
                let connection = Connection::open(&path)?;
                (connection, true, None)
            } else {
                let connection = Connection::open(&path)?;
                (connection, false, None)
            };

        schema::initialize_schema(&connection)?;

        let biometric_encryption = if enable_biometric {
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
            BiometricEncryption::new(config).ok().map(Mutex::new)
        } else {
            None
        };

        let server_client: Box<dyn ServerClientTrait + Send> = Box::new(ServerClient::new()?);

        let wallet = Wallet {
            path,
            connection: Mutex::new(connection),
            server_client: Mutex::new(server_client),
            biometric_encryption,
            is_encrypted,
            temp_db_path,
        };

        let _ = wallet.get_or_generate_master_secret()?;
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
        let connection = Connection::open_in_memory()?;
        schema::initialize_schema(&connection)?;

        let server_client: Box<dyn ServerClientTrait + Send> = Box::new(ServerClient::new()?);

        let wallet = Wallet {
            path: PathBuf::from(":memory:"),
            connection: Mutex::new(connection),
            server_client: Mutex::new(server_client),
            biometric_encryption: None,
            is_encrypted: false,
            temp_db_path: None,
        };
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Get the wallet database path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Close the wallet (flushes any pending operations).
    pub async fn close(mut self) -> Result<()> {
        if self.is_encrypted {
            self.encrypt_database().await?;
        }
        if let Some(biometric_mutex) = self.biometric_encryption.take() {
            let mut biometric = biometric_mutex
                .into_inner()
                .map_err(|_| Error::wallet("Failed to acquire biometric lock during close"))?;
            biometric.clear_cached_keys();
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

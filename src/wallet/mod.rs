//! Wallet engine — pluggable-storage HD wallet for Webcash.
//!
//! # Architecture
//!
//! - **`store`** — Storage trait: `SqliteStore` (native) or `MemStore` (WASM).
//! - **`operations`** — Insert, pay, merge, recover, check, balance.
//! - **`encryption`** — Database-level and seed-level encryption.
//! - **`snapshot`** — JSON export/import for backup and recovery.
//! - **`schema`** — SQLite schema init (native only).

#[cfg(not(target_arch = "wasm32"))]
pub mod encryption;
pub mod operations;
#[cfg(not(target_arch = "wasm32"))]
pub mod schema;
pub mod snapshot;
pub mod store;

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::server::{NetworkMode, ServerClient, ServerConfig};
#[cfg(not(target_arch = "wasm32"))]
use crate::server::ServerClientTrait;

#[cfg(not(target_arch = "wasm32"))]
use crate::passkey::{EncryptionConfig, PasskeyEncryption};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;

pub use operations::{CheckResult, RecoveryResult, WalletStats};
pub use snapshot::{SpentHashSnapshot, UnspentOutputSnapshot, WalletSnapshot};
pub use store::Store;

/// Webcash wallet with pluggable storage backend.
pub struct Wallet {
    pub(crate) path: PathBuf,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) store: Box<dyn Store + Send + Sync>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) store: Box<dyn Store>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) server_client: tokio::sync::Mutex<Box<dyn ServerClientTrait + Send>>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) server_client: ServerClient,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) passkey_encryption: Option<Mutex<PasskeyEncryption>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) is_encrypted: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) temp_db_path: Option<PathBuf>,
    pub(crate) network: NetworkMode,
}

// ── Native constructors ──────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
impl Wallet {
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_passkey(path, false).await
    }

    pub async fn open_with_passkey<P: AsRef<Path>>(path: P, enable_passkey: bool) -> Result<Self> {
        use rusqlite::Connection;

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
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("default")
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
        let store: Box<dyn Store + Send + Sync> = Box::new(store::sqlite::SqliteStore(Mutex::new(connection)));

        let wallet = Wallet {
            path,
            store,
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption,
            is_encrypted,
            temp_db_path,
            network: NetworkMode::Production,
        };
        let _ = wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    pub async fn open_with_network<P: AsRef<Path>>(path: P, network: NetworkMode) -> Result<Self> {
        use rusqlite::Connection;

        let config = ServerConfig { network: network.clone(), timeout_seconds: 30 };
        let server_client: Box<dyn ServerClientTrait + Send> =
            Box::new(ServerClient::with_config(config)?);

        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&path)?;
        schema::initialize_schema(&connection)?;

        let store: Box<dyn Store + Send + Sync> = Box::new(store::sqlite::SqliteStore(Mutex::new(connection)));

        let wallet = Wallet {
            path,
            store,
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption: None,
            is_encrypted: false,
            temp_db_path: None,
            network,
        };
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    pub async fn open_with_seed<P: AsRef<Path>>(path: P, seed: &[u8; 32]) -> Result<Self> {
        let wallet = Self::open(path).await?;
        let hex = hex::encode(seed);
        let existing = wallet.master_secret_hex()?;
        if existing != hex {
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

    pub fn open_memory() -> Result<Self> {
        Self::open_memory_with_network(NetworkMode::Production)
    }

    pub fn open_memory_with_network(network: NetworkMode) -> Result<Self> {
        use rusqlite::Connection;

        let connection = Connection::open_in_memory()?;
        schema::initialize_schema(&connection)?;

        let config = ServerConfig { network: network.clone(), timeout_seconds: 30 };
        let server_client: Box<dyn ServerClientTrait + Send> =
            Box::new(ServerClient::with_config(config)?);

        let store: Box<dyn Store + Send + Sync> = Box::new(store::sqlite::SqliteStore(Mutex::new(connection)));

        let wallet = Wallet {
            path: PathBuf::from(":memory:"),
            store,
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption: None,
            is_encrypted: false,
            temp_db_path: None,
            network,
        };
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Open a wallet backed by a JSON file. Creates the file if it doesn't exist.
    pub fn open_json<P: AsRef<Path>>(path: P, network: NetworkMode) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let json_store = store::json::JsonStore::open(path.clone())?;
        let config = ServerConfig { network: network.clone(), timeout_seconds: 30 };
        let server_client: Box<dyn ServerClientTrait + Send> =
            Box::new(ServerClient::with_config(config)?);
        let store: Box<dyn Store + Send + Sync> = Box::new(json_store);

        let wallet = Wallet {
            path,
            store,
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption: None,
            is_encrypted: false,
            temp_db_path: None,
            network,
        };
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Create an in-memory JSON wallet (no file persistence).
    /// Use `to_json()` to retrieve the state.
    pub fn open_json_memory(network: NetworkMode) -> Result<Self> {
        let json_store = store::json::JsonStore::new(None);
        let config = ServerConfig { network: network.clone(), timeout_seconds: 30 };
        let server_client: Box<dyn ServerClientTrait + Send> =
            Box::new(ServerClient::with_config(config)?);
        let store: Box<dyn Store + Send + Sync> = Box::new(json_store);

        let wallet = Wallet {
            path: PathBuf::from(":json-memory:"),
            store,
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption: None,
            is_encrypted: false,
            temp_db_path: None,
            network,
        };
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Create from a JSON string (in-memory, no file persistence).
    pub fn from_json_native(json: &str, network: NetworkMode) -> Result<Self> {
        let json_store = store::json::JsonStore::from_json(json, None)?;
        let config = ServerConfig { network: network.clone(), timeout_seconds: 30 };
        let server_client: Box<dyn ServerClientTrait + Send> =
            Box::new(ServerClient::with_config(config)?);
        let store: Box<dyn Store + Send + Sync> = Box::new(json_store);

        Ok(Wallet {
            path: PathBuf::from(":json-memory:"),
            store,
            server_client: tokio::sync::Mutex::new(server_client),
            passkey_encryption: None,
            is_encrypted: false,
            temp_db_path: None,
            network,
        })
    }

    pub async fn close(mut self) -> Result<()> {
        if self.is_encrypted {
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

// ── WASM constructors ────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
impl Wallet {
    /// Create a wallet with in-memory storage (for WASM).
    pub fn new_memory(network: NetworkMode) -> Result<Self> {
        let store: Box<dyn Store> = Box::new(store::mem::MemStore::new());
        let config = ServerConfig { network: network.clone(), timeout_seconds: 30 };
        let server_client = ServerClient::with_config(config)?;
        let wallet = Wallet {
            path: PathBuf::from(":memory:"),
            store,
            server_client,
            network,
        };
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Create from JSON state (loaded from IndexedDB by JS).
    pub fn from_json(json: &str, network: NetworkMode) -> Result<Self> {
        let store: Box<dyn Store> = Box::new(store::mem::MemStore::from_json(json)?);
        let config = ServerConfig { network: network.clone(), timeout_seconds: 30 };
        let server_client = ServerClient::with_config(config)?;
        Ok(Wallet {
            path: PathBuf::from(":memory:"),
            store,
            server_client,
            network,
        })
    }

}

// ── Shared methods ───────────────────────────────────────────────

impl Wallet {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn network(&self) -> &NetworkMode {
        &self.network
    }

    /// Serialize wallet state to JSON.
    /// Works with MemStore (WASM) and JsonStore (native).
    pub fn to_json(&self) -> Result<String> {
        #[cfg(target_arch = "wasm32")]
        {
            let mem = self.store.as_any().downcast_ref::<store::mem::MemStore>()
                .ok_or_else(|| Error::wallet("Store does not support JSON serialization"))?;
            return mem.to_json();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(json_store) = self.store.as_any().downcast_ref::<store::json::JsonStore>() {
                return json_store.to_json();
            }
            Err(Error::wallet("Store does not support JSON serialization (use JsonStore or MemStore)"))
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Wallet {
    fn drop(&mut self) {
        if self.is_encrypted {
            if let Some(temp_path) = &self.temp_db_path {
                let _ = std::fs::remove_file(temp_path);
            }
        }
    }
}

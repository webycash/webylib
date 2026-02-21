//! Wallet functionality for Webcash
//!
//! This module provides the main wallet interface for managing Webcash,
//! including SQLite storage, transactions, and balance management.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;
use std::fs;

use rusqlite::{Connection, params, OptionalExtension};

use crate::amount::Amount;
use crate::error::{Error, Result};
use crate::webcash::{SecretWebcash, PublicWebcash};
use crate::crypto::CryptoSecret;
use crate::biometric::{BiometricEncryption, EncryptionConfig, EncryptedData, decrypt_with_password, encrypt_with_password};

/// Main wallet structure with SQLite backend
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct WalletSnapshot {
    pub master_secret: String,
    pub unspent_outputs: Vec<UnspentOutputSnapshot>,
    pub spent_hashes: Vec<SpentHashSnapshot>,
    pub depths: std::collections::HashMap<String, i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct UnspentOutputSnapshot {
    pub secret: String,
    pub amount: i64,
    pub created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct SpentHashSnapshot {
    pub hash: String,
    pub spent_at: String,
}

pub struct Wallet {
    /// Path to the wallet database file
    path: PathBuf,
    /// SQLite connection (wrapped in Mutex for thread safety)
    connection: Mutex<Connection>,
    /// Biometric encryption handler (optional)
    biometric_encryption: Option<Mutex<BiometricEncryption>>,
    /// Whether this wallet uses runtime encryption
    is_encrypted: bool,
    /// Temporary path for decrypted database during runtime
    temp_db_path: Option<PathBuf>,
}

impl Wallet {
    /// Get master secret from wallet metadata, generating one if it doesn't exist
    fn get_or_generate_master_secret(&self) -> Result<String> {
        let connection = self.connection.lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        
        // Try to get existing master secret
        let master_secret_hex: Option<String> = connection.query_row(
            "SELECT value FROM wallet_metadata WHERE key = 'master_secret'",
            [],
            |row| row.get(0),
        ).optional()
        .map_err(|e| Error::Database(e).with_context("Failed to query master secret"))?;

        match master_secret_hex {
            Some(secret) => Ok(secret),
            None => {
                // Generate new master secret using secure RNG
                let master_secret = CryptoSecret::generate()
                    .map_err(|e| Error::crypto(&format!("Failed to generate master secret: {}", e)))?;
                
                let master_secret_hex = master_secret.to_hex();
                
                // Store the new master secret
                connection.execute(
                    "INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES ('master_secret', ?1)",
                    params![master_secret_hex],
                )?;
                
                println!("🔑 Generated new master secret using hardware RNG");
                Ok(master_secret_hex)
            }
        }
    }

    /// Get master secret from wallet metadata (legacy method for backward compatibility)
    fn get_master_secret(&self) -> Result<String> {
        self.get_or_generate_master_secret()
    }

    /// Get master secret hex for backup/recovery. Persist to secrets/bench_master_secret.
    pub fn master_secret_hex(&self) -> Result<String> {
        self.get_master_secret()
    }

    /// Validate and parse master secret into array
    fn validate_master_secret(&self, master_secret_hex: &str) -> Result<[u8; 32]> {
        let master_secret_bytes = hex::decode(master_secret_hex)
            .map_err(|_| Error::wallet("Invalid master secret format"))?;

        if master_secret_bytes.len() != 32 {
            return Err(Error::wallet(format!(
                "Master secret must be 32 bytes (64 hex characters), got {} bytes",
                master_secret_bytes.len()
            )));
        }

        let mut master_secret_array = [0u8; 32];
        master_secret_array.copy_from_slice(&master_secret_bytes);
        Ok(master_secret_array)
    }

    /// Open or create a wallet at the given path
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_biometric(path, false).await
    }

    /// Open or create a wallet with optional biometric encryption
    pub async fn open_with_biometric<P: AsRef<Path>>(path: P, enable_biometric: bool) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Create wallet directory if it doesn't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Check if the database file is encrypted or needs to be encrypted
        let (connection, is_encrypted, temp_db_path) = if enable_biometric && Self::is_database_encrypted(&path)? {
            // Decrypt the database for runtime use
            let temp_path = Self::decrypt_database_for_runtime(&path).await?;
            let connection = Connection::open(&temp_path)?;
            (connection, true, Some(temp_path))
        } else if enable_biometric {
            // New biometric wallet - mark as encrypted but use normal database for now
            let connection = Connection::open(&path)?;
            (connection, true, None)
        } else {
            // Open normally
            let connection = Connection::open(&path)?;
            (connection, false, None)
        };

        // Initialize database schema
        Self::initialize_schema(&connection)?;

        // Setup biometric encryption if requested
        let biometric_encryption = if enable_biometric {
            let config = EncryptionConfig {
                app_identifier: "com.webycash.webylib".to_string(),
                service_name: format!("WalletEncryption_{}", 
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("default")),
                require_auth_every_use: true,
                auth_timeout_seconds: 0,
                allow_device_passcode_fallback: true,
            };
            
            match BiometricEncryption::new(config) {
                Ok(biometric) => {
                    println!("🔐 Biometric encryption enabled for wallet");
                    Some(Mutex::new(biometric))
                }
                Err(e) => {
                    println!("⚠️  Failed to enable biometric encryption: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let wallet = Wallet {
            path,
            connection: Mutex::new(connection),
            biometric_encryption,
            is_encrypted,
            temp_db_path,
        };

        // Generate master secret if it doesn't exist
        let _ = wallet.get_or_generate_master_secret()?;

        Ok(wallet)
    }

    /// Open an in-memory wallet backed by SQLite (useful for Lambda/Stateless execution)
    pub fn open_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        Self::initialize_schema(&connection)?;
        let wallet = Wallet {
            path: PathBuf::from(":memory:"),
            connection: Mutex::new(connection),
            biometric_encryption: None, // No encryption for memory wallet
            is_encrypted: false,
            temp_db_path: None,
        };
        // Ensure master secret is generated immediately
        wallet.get_or_generate_master_secret()?;
        Ok(wallet)
    }

    /// Export wallet state to a snapshot
    pub fn export_snapshot(&self) -> Result<WalletSnapshot> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire lock"))?;
        
        let master_secret: String = connection.query_row(
            "SELECT value FROM wallet_metadata WHERE key = 'master_secret'", [], |r| r.get(0)
        )?;

        let mut stmt = connection.prepare("SELECT secret, amount, created_at FROM unspent_outputs WHERE spent = 0")?;
        let unspent = stmt.query_map([], |row| {
            Ok(UnspentOutputSnapshot {
                secret: row.get(0)?,
                amount: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        let mut stmt = connection.prepare("SELECT hash, spent_at FROM spent_hashes")?;
        let spent = stmt.query_map([], |row| {
            let hash_blob: Vec<u8> = row.get(0)?;
            Ok(SpentHashSnapshot {
                hash: hex::encode(hash_blob),
                spent_at: row.get(1)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        let mut stmt = connection.prepare("SELECT chain_code, depth FROM walletdepths")?;
        let depths_map = stmt.query_map([], |row| {
             Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?.collect::<std::result::Result<std::collections::HashMap<String, i64>, _>>()?;

        Ok(WalletSnapshot {
            master_secret,
            unspent_outputs: unspent,
            spent_hashes: spent,
            depths: depths_map,
        })
    }

    /// Import wallet state from a snapshot (overwrites current state)
    pub fn import_snapshot(&self, snapshot: &WalletSnapshot) -> Result<()> {
        let mut connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire lock"))?;
        let tx = connection.transaction()?;
        
        // Metadata
        tx.execute("INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES ('master_secret', ?1)", params![snapshot.master_secret])?;

        // Depths
        for (code, depth) in &snapshot.depths {
            tx.execute("INSERT OR REPLACE INTO walletdepths (chain_code, depth) VALUES (?1, ?2)", params![code, depth])?;
        }

        // Unspent
        // We need to re-derive hash for unspent
        for item in &snapshot.unspent_outputs {
            let secret_hash = crate::crypto::sha256(item.secret.as_bytes());
            tx.execute(
                "INSERT OR REPLACE INTO unspent_outputs (secret_hash, secret, amount, created_at, spent) VALUES (?1, ?2, ?3, ?4, 0)",
                params![&secret_hash[..], item.secret, item.amount, item.created_at]
            )?;
        }
        
        // Spent
        for item in &snapshot.spent_hashes {
            let hash_bytes = hex::decode(&item.hash).map_err(|_| Error::wallet("Invalid hex in snapshot"))?;
            tx.execute(
                "INSERT OR REPLACE INTO spent_hashes (hash, spent_at) VALUES (?1, ?2)",
                params![hash_bytes, item.spent_at]
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Initialize the database schema
    fn initialize_schema(connection: &Connection) -> Result<()> {
        // Create unspent_outputs table for storing webcash
        connection.execute(
            "CREATE TABLE IF NOT EXISTS unspent_outputs (
                id INTEGER PRIMARY KEY,
                secret_hash BLOB UNIQUE NOT NULL,
                secret TEXT NOT NULL,
                amount INTEGER NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                spent INTEGER DEFAULT 0
            )",
            [],
        )?;

        // Create spent_hashes table for tracking spent webcash
        connection.execute(
            "CREATE TABLE IF NOT EXISTS spent_hashes (
                id INTEGER PRIMARY KEY,
                hash BLOB UNIQUE NOT NULL,
                spent_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Create wallet metadata table
        connection.execute(
            "CREATE TABLE IF NOT EXISTS wallet_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;

        // Create walletdepths table to track HD wallet depth per chain code (matches Python walletdepths)
        connection.execute(
            "CREATE TABLE IF NOT EXISTS walletdepths (
                chain_code TEXT PRIMARY KEY,
                depth INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        // Initialize walletdepths if empty (matches Python generate_initial_walletdepths)
        connection.execute(
            "INSERT OR IGNORE INTO walletdepths (chain_code, depth) VALUES 
             ('RECEIVE', 0), ('PAY', 0), ('CHANGE', 0), ('MINING', 0)",
            [],
        )?;

        // Create indexes for performance
        connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_unspent_outputs_secret_hash ON unspent_outputs(secret_hash)",
            [],
        )?;
        connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_unspent_outputs_spent ON unspent_outputs(spent)",
            [],
        )?;
        connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_spent_hashes_hash ON spent_hashes(hash)",
            [],
        )?;

        Ok(())
    }

    /// Check if a database file is encrypted
    pub fn is_database_encrypted(path: &PathBuf) -> Result<bool> {
        if !path.exists() {
            return Ok(false);
        }

        // Try to read the first few bytes to see if it looks like SQLite
        let bytes = fs::read(path)
            .map_err(|e| Error::wallet(&format!("Failed to read database file: {}", e)))?;
        
        if bytes.len() < 16 {
            return Ok(false);
        }

        // SQLite databases start with "SQLite format 3\0"
        let sqlite_magic = b"SQLite format 3\0";
        Ok(!bytes.starts_with(sqlite_magic))
    }

    /// Decrypt database for runtime use
    async fn decrypt_database_for_runtime(encrypted_path: &PathBuf) -> Result<PathBuf> {
        // Read encrypted database
        let encrypted_bytes = fs::read(encrypted_path)
            .map_err(|e| Error::wallet(&format!("Failed to read encrypted database: {}", e)))?;

        // Parse as EncryptedData JSON
        let encrypted_data: EncryptedData = serde_json::from_slice(&encrypted_bytes)
            .map_err(|e| Error::wallet(&format!("Invalid encrypted database format: {}", e)))?;

        // Decrypt using biometric (placeholder password for now)
        let decrypted_bytes = decrypt_with_password(&encrypted_data, "biometric_placeholder")
            .map_err(|e| Error::wallet(&format!("Failed to decrypt database: {}", e)))?;

        // Create temporary file for decrypted database
        let temp_path = encrypted_path.with_extension("temp.db");
        fs::write(&temp_path, decrypted_bytes)
            .map_err(|e| Error::wallet(&format!("Failed to write decrypted database: {}", e)))?;

        Ok(temp_path)
    }

    /// Encrypt the wallet database with password
    pub async fn encrypt_database_with_password(&self, password: &str) -> Result<()> {
        // Read the current database file
        let db_bytes = fs::read(&self.path)
            .map_err(|e| Error::wallet(&format!("Failed to read database: {}", e)))?;

        // Encrypt using password
        let encrypted_data = encrypt_with_password(&db_bytes, password)?;

        // Write encrypted data to main database file
        let encrypted_json = serde_json::to_vec_pretty(&encrypted_data)
            .map_err(|e| Error::wallet(&format!("Failed to serialize encrypted data: {}", e)))?;

        fs::write(&self.path, encrypted_json)
            .map_err(|e| Error::wallet(&format!("Failed to write encrypted database: {}", e)))?;

        Ok(())
    }

    /// Decrypt the wallet database with password
    pub async fn decrypt_database_with_password(&self, password: &str) -> Result<()> {
        // Read encrypted database
        let encrypted_bytes = fs::read(&self.path)
            .map_err(|e| Error::wallet(&format!("Failed to read encrypted database: {}", e)))?;

        // Parse as EncryptedData JSON
        let encrypted_data: EncryptedData = serde_json::from_slice(&encrypted_bytes)
            .map_err(|e| Error::wallet(&format!("Invalid encrypted database format: {}", e)))?;

        // Decrypt using password
        let decrypted_bytes = decrypt_with_password(&encrypted_data, password)
            .map_err(|e| Error::wallet(&format!("Failed to decrypt database: {}", e)))?;

        // Write decrypted database back
        fs::write(&self.path, decrypted_bytes)
            .map_err(|e| Error::wallet(&format!("Failed to write decrypted database: {}", e)))?;

        Ok(())
    }

    /// Encrypt the wallet database
    pub async fn encrypt_database(&self) -> Result<()> {
        if !self.is_encrypted {
            return Err(Error::wallet("Wallet is not configured for encryption"));
        }

        let mut biometric = self.biometric_encryption
            .as_ref()
            .ok_or_else(|| Error::wallet("Biometric encryption not available"))?
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire biometric lock"))?;

        // Read the current database file (either temp or main)
        let db_bytes = if let Some(temp_path) = &self.temp_db_path {
            // Read from temporary decrypted database
            fs::read(temp_path)
                .map_err(|e| Error::wallet(&format!("Failed to read temp database: {}", e)))?
        } else {
            // Read from main database
            fs::read(&self.path)
                .map_err(|e| Error::wallet(&format!("Failed to read database: {}", e)))?
        };

        // Encrypt the database
        let encrypted_data = biometric.encrypt_with_biometrics(&db_bytes).await?;

        // Write encrypted data to main database file
        let encrypted_json = serde_json::to_vec_pretty(&encrypted_data)
            .map_err(|e| Error::wallet(&format!("Failed to serialize encrypted data: {}", e)))?;

        fs::write(&self.path, encrypted_json)
            .map_err(|e| Error::wallet(&format!("Failed to write encrypted database: {}", e)))?;

        // Clean up temporary file if it exists
        if let Some(temp_path) = &self.temp_db_path {
            let _ = fs::remove_file(temp_path); // Ignore errors
        }

        Ok(())
    }

    /// Decrypt the wallet database for use
    pub async fn decrypt_database(&self) -> Result<()> {
        if !self.is_encrypted {
            return Err(Error::wallet("Wallet is not encrypted"));
        }

        // This should already be done in open_with_biometric, but can be called manually
        if self.temp_db_path.is_some() {
            return Ok(());
        }

        Err(Error::wallet("Database decryption must be done during wallet opening"))
    }

    /// Store webcash directly in wallet without ownership transfer (for change, etc.)
    /// This matches Python's direct storage: webcash_wallet["webcash"].append(str(mychange))
    pub async fn store_directly(&self, webcash: SecretWebcash) -> Result<()> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let secret_str = webcash.secret.as_str()
            .map_err(|_| Error::wallet("Invalid secret encoding"))?;
        // Hash must match to_public() calculation - hash ASCII string bytes, not hex-decoded
        let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
        connection.execute(
            "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
            params![&secret_hash[..], secret_str, webcash.amount.wats],
        )?;
        println!("✅ Webcash stored directly in wallet without ownership transfer");
        println!("💰 Stored amount: {}", webcash.amount);
        Ok(())
    }

    /// Insert webcash into the wallet with server validation
    /// In test mode, server validation can be skipped
    pub async fn insert(&self, webcash: SecretWebcash) -> Result<()> {
        // Match Python implementation: insert does NOT validate before replace
        // Python directly calls replace without health check validation
        self.insert_with_validation(webcash, false).await
    }

    /// Insert webcash into the wallet with server validation and ownership transfer
    /// This matches the Python reference implementation exactly:
    /// 1. Validate input webcash with server
    /// 2. Generate NEW webcash with HD-derived secret
    /// 3. Replace input with new webcash on server (transfers ownership)
    /// 4. Store the NEW webcash in wallet
    pub async fn insert_with_validation(&self, webcash: SecretWebcash, validate_with_server: bool) -> Result<()> {
        println!("🔄 Starting webcash insertion with ownership transfer...");

        // Get and validate master secret
        let master_secret_hex = self.get_master_secret()?;
        let master_secret_array = self.validate_master_secret(&master_secret_hex)?;

        // Create HD wallet for generating new secret
        let mut hd_wallet = crate::hd::HDWallet::from_master_secret(master_secret_array);

        // Get next depth for RECEIVE chain
        let depth = {
            let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;
            let current_depth: Option<i64> = connection.query_row(
                "SELECT COUNT(*) FROM unspent_outputs",
                [],
                |row| row.get(0),
            ).optional()?;
            current_depth.unwrap_or(0) as u64
        };

        // Generate NEW webcash with HD-derived secret
        // Match Python: always use RECEIVE chain for insert (line 416: chain_code="RECEIVE")
        // Python uses walletdepths["RECEIVE"] and increments it after generation
        let new_secret_hex = hd_wallet.generate_receive_secret(depth)
            .map_err(|e| Error::crypto(&format!("Failed to generate new secret: {}", e)))?;

        let new_webcash = SecretWebcash::new(
            crate::webcash::SecureString::new(new_secret_hex),
            webcash.amount
        );

        println!("🔑 Generated new webcash: {}", new_webcash.to_string());

        // Validate input webcash with server (if requested)
        // NOTE: Python implementation does NOT validate before replace - it just does the replace directly
        // Validation is disabled by default to match Python behavior
        if validate_with_server {
            println!("🔍 Validating input webcash with server...");
            use crate::server::{ServerClient, ServerConfig};

            let _server_config = ServerConfig::default();
            let server_client = ServerClient::new()?;
            let public_webcash = webcash.to_public();

            let health_check_result = server_client.health_check(&[public_webcash.clone()]).await;

            match health_check_result {
                Ok(response) => {
                    if response.status != "success" {
                        return Err(Error::server("Server validation failed"));
                    }

                    if let Some(health_result) = response.results.get(&public_webcash.to_string()) {
                        // Check if webcash is spent
                        if let Some(spent) = health_result.spent {
                            if spent {
                                return Err(Error::wallet("Input webcash has been spent"));
                            }
                        }
                        // If spent is null/None, treat as unspent and proceed

                        // Validate amount matches server response (if server provides it)
                        // If server doesn't provide amount (None), we proceed anyway - server will validate during replace
                        if let Some(server_amount) = &health_result.amount {
                            println!("🔍 Server returned amount: '{}'", server_amount);
                            let expected_amount = Amount::from_str(server_amount)
                                .map_err(|e| {
                                    println!("❌ Failed to parse server amount '{}': {}", server_amount, e);
                                    Error::wallet(&format!("Invalid amount from server: {}", server_amount))
                                })?;

                            if webcash.amount != expected_amount {
                                return Err(Error::wallet(&format!(
                                    "Amount mismatch: provided {}, server says {}",
                                    webcash.amount, expected_amount
                                )));
                            }
                        }

                        println!("✅ Input webcash validation successful");
                    } else {
                        return Err(Error::server("Input webcash not found in server response"));
                    }
                }
                Err(e) => {
                    println!("⚠️  Input webcash validation failed: {}. Proceeding with replacement anyway...", e);
                }
            }
        }

        // Perform server replacement to transfer ownership (matches Python exactly)
        // Python always does replace - validation is separate from replace
        // Replace MUST happen unless truly in offline mode
        println!("🔄 Replacing webcash on server to transfer ownership...");
        use crate::server::{ServerClient, ServerConfig, ReplaceRequest, Legalese};

        let _server_config = ServerConfig::default();
        let server_client = ServerClient::new()?;

        let replace_request = ReplaceRequest {
            webcashes: vec![webcash.to_string()],
            new_webcashes: vec![new_webcash.to_string()],
            legalese: Legalese { terms: true },
        };

        match server_client.replace(&replace_request).await {
            Ok(replace_response) if replace_response.status == "success" => {
                println!("✅ Server replacement successful - ownership transferred");

                // Store the NEW webcash in wallet (not the original!)
                let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

                let new_secret_str = new_webcash.secret.as_str().map_err(|_| Error::wallet("Invalid new secret encoding"))?;
                // Hash must match to_public() calculation - hash ASCII string bytes, not hex-decoded
                let new_secret_hash = crate::crypto::sha256(new_secret_str.as_bytes());

                connection.execute(
                    "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
                    params![&new_secret_hash[..], new_secret_str, new_webcash.amount.wats],
                )?;

                println!("✅ New webcash successfully stored in wallet");
                println!("💰 Inserted amount: {}", new_webcash.amount);

                Ok(())
            }
            Err(e) if matches!(&e, Error::Server { message } if message.contains("can only be replaced by itself")) => {
                // Same-lineage: input and output share HD tree. Server rejects replace.
                // Fall back to store_directly: validate unspent via health_check, then store as-is.
                println!("⚠️ Same-lineage token detected, storing directly without replace...");
                let public_webcash = webcash.to_public();
                let health_response = server_client.health_check(&[public_webcash.clone()]).await?;
                if health_response.status != "success" {
                    return Err(Error::server("Health check failed for same-lineage fallback"));
                }
                if let Some(hr) = health_response.results.get(&public_webcash.to_string()) {
                    if hr.spent == Some(true) {
                        return Err(Error::wallet("Input webcash has been spent"));
                    }
                }
                self.store_directly(webcash).await
            }
            Ok(_) => Err(Error::server("Server replacement failed - ownership transfer unsuccessful")),
            Err(e) => Err(e),
        }
    }

    /// Get the current wallet balance
    pub async fn balance(&self) -> Result<String> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let total_wats: Option<i64> = connection.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM unspent_outputs WHERE spent = 0",
            [],
            |row| row.get(0),
        ).optional()?;

        let total_wats = total_wats.unwrap_or(0);
        let balance = Amount::from_wats(total_wats);

        Ok(balance.to_string())
    }

    /// Get the current wallet balance as Amount
    pub async fn balance_amount(&self) -> Result<Amount> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let total_wats: Option<i64> = connection.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM unspent_outputs WHERE spent = 0",
            [],
            |row| row.get(0),
        ).optional()?;

        let total_wats = total_wats.unwrap_or(0);
        Ok(Amount::from_wats(total_wats))
    }

    /// List all unspent webcash in the wallet
    pub async fn list_webcash(&self) -> Result<Vec<SecretWebcash>> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let mut stmt = connection.prepare(
            "SELECT secret, amount FROM unspent_outputs WHERE spent = 0 ORDER BY created_at"
        )?;

        let webcash_iter = stmt.query_map([], |row| {
            let secret_str: String = row.get(0)?;
            let amount_wats: i64 = row.get(1)?;

            Ok((secret_str, amount_wats))
        })?;

        let mut result = Vec::new();
        for item in webcash_iter {
            let (secret_str, amount_wats) = item?;
            let secret = crate::webcash::SecureString::new(secret_str);
            let amount = Amount::from_wats(amount_wats);
            result.push(SecretWebcash::new(secret, amount));
        }

        Ok(result)
    }

    /// List all unspent webcash as public webcash
    pub async fn list_public_webcash(&self) -> Result<Vec<PublicWebcash>> {
        let webcash_list = self.list_webcash().await?;
        Ok(webcash_list.iter().map(|wc| wc.to_public()).collect())
    }

    /// Check wallet status against server
    pub async fn check(&self) -> Result<()> {
        use crate::server::{ServerClient, ServerConfig};

        // Get all public webcash from the wallet
        let public_webcash_list = self.list_public_webcash().await?;

        if public_webcash_list.is_empty() {
            println!("✅ Wallet is empty - no webcash to check");
            return Ok(());
        }

        // Create server client
        let _server_config = ServerConfig::default();
        let server_client = ServerClient::new()?;

        println!("🔍 Checking {} webcash entries against server...", public_webcash_list.len());

        // Perform health check
        let health_response = server_client.health_check(&public_webcash_list).await?;

        if health_response.status != "success" {
            return Err(crate::error::Error::server("Server returned non-success status"));
        }

        let mut valid_count = 0;
        let invalid_count = 0;
        let mut spent_count = 0;

        // Analyze results
        for (webcash_str, health_result) in &health_response.results {
            if let Some(spent) = health_result.spent {
                if spent {
                    spent_count += 1;
                    println!("❌ {} - SPENT", webcash_str);
                } else {
                    valid_count += 1;
                    println!("✅ {} - VALID", webcash_str);
                }
            } else {
                valid_count += 1;
                println!("✅ {} - VALID", webcash_str);
            }
        }

        println!("\n📊 Check Results:");
        println!("  Valid: {}", valid_count);
        println!("  Spent: {}", spent_count);
        println!("  Unknown: {}", invalid_count);

        if invalid_count > 0 {
            return Err(crate::error::Error::server("Some webcash entries are invalid or unknown"));
        }

        if spent_count > 0 {
            return Err(crate::error::Error::server("Some webcash entries have been spent"));
        }

        println!("✅ All webcash entries are valid and unspent");
        Ok(())
    }

    /// Get wallet statistics
    pub async fn stats(&self) -> Result<WalletStats> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let total_webcash: Option<i64> = connection.query_row(
            "SELECT COUNT(*) FROM unspent_outputs",
            [],
            |row| row.get(0),
        ).optional()?;

        let unspent_webcash: Option<i64> = connection.query_row(
            "SELECT COUNT(*) FROM unspent_outputs WHERE spent = 0",
            [],
            |row| row.get(0),
        ).optional()?;

        let spent_webcash: Option<i64> = connection.query_row(
            "SELECT COUNT(*) FROM spent_hashes",
            [],
            |row| row.get(0),
        ).optional()?;

        let total_balance_wats: Option<i64> = connection.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM unspent_outputs WHERE spent = 0",
            [],
            |row| row.get(0),
        ).optional()?;

        Ok(WalletStats {
            total_webcash: total_webcash.unwrap_or(0) as u64,
            unspent_webcash: unspent_webcash.unwrap_or(0) as u64,
            spent_webcash: spent_webcash.unwrap_or(0) as u64,
            total_balance: Amount::from_wats(total_balance_wats.unwrap_or(0)),
        })
    }

    /// Get the wallet database path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Pay amount using server-validated transaction (matches Python reference exactly)
    /// This generates new webcash outputs that can be manually given to recipients
    pub async fn pay(&self, amount: Amount, memo: &str) -> Result<String> {
        println!("💸 Starting payment transaction...");
        println!("  Amount: {}", amount);
        println!("  Memo: {}", memo);

        // Get and validate master secret
        let master_secret_hex = self.get_master_secret()?;
        let master_secret_array = self.validate_master_secret(&master_secret_hex)?;

        // Create HD wallet for generating output secrets
        let mut hd_wallet = crate::hd::HDWallet::from_master_secret(master_secret_array);

        // Select inputs that cover the payment amount (matches Python input selection)
        let inputs = self.select_inputs(amount).await?;

        if inputs.is_empty() {
            return Err(Error::wallet("Insufficient funds"));
        }

        // Calculate total input amount
        let input_total: Amount = inputs.iter().fold(Amount::ZERO, |acc, wc| acc + wc.amount);
        println!("  Selected inputs: {} (total: {})", inputs.len(), input_total);

        // Calculate change amount
        let change_amount = input_total - amount;
        println!("  Change amount: {}", change_amount);

        // Generate output webcashes using HD derivation (matches Python exactly)
        let mut new_webcashes = Vec::new();

        // Get current wallet depth for PAY chain (matches Python: uses walletdepths["PAY"])
        let (pay_depth, change_depth) = {
            let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;
            let pay_depth: u64 = connection.query_row(
                "SELECT depth FROM walletdepths WHERE chain_code = 'PAY'",
                [],
                |row| row.get::<_, i64>(0),
            ).optional()?
                .unwrap_or(0) as u64;
            
            // Increment PAY depth after use (matches Python: walletdepths["PAY"] = walletdepth + 1)
            connection.execute(
                "UPDATE walletdepths SET depth = ? WHERE chain_code = 'PAY'",
                params![(pay_depth + 1) as i64],
            )?;
            
            // Get CHANGE depth
            let change_depth: u64 = connection.query_row(
                "SELECT depth FROM walletdepths WHERE chain_code = 'CHANGE'",
                [],
                |row| row.get::<_, i64>(0),
            ).optional()?
                .unwrap_or(0) as u64;
            
            // Increment CHANGE depth after use
            connection.execute(
                "UPDATE walletdepths SET depth = ? WHERE chain_code = 'CHANGE'",
                params![(change_depth + 1) as i64],
            )?;
            (pay_depth, change_depth)
        };

        // Generate payment webcash using PAY chain (matches Python: chain_code="PAY" -> CHAIN_CODES["PAY"] = 1)
        let pay_secret_hex = hd_wallet.generate_pay_secret(pay_depth)
            .map_err(|e| Error::crypto(&format!("Failed to generate payment secret: {}", e)))?;

        let payment_webcash = SecretWebcash::new(
            crate::webcash::SecureString::new(pay_secret_hex),
            amount
        );
        new_webcashes.push(payment_webcash.to_string());
        println!("  Generated payment webcash: {}", payment_webcash.to_string());

        // Generate change webcash if needed (matches Python: chain_code="CHANGE" -> CHAIN_CODES["CHANGE"] = 2)
        let change_webcash_for_wallet = if change_amount > Amount::ZERO {
            let change_secret_hex = hd_wallet.generate_change_secret(change_depth)
                .map_err(|e| Error::crypto(&format!("Failed to generate change secret: {}", e)))?;

            let change_webcash = SecretWebcash::new(
                crate::webcash::SecureString::new(change_secret_hex),
                change_amount
            );
            new_webcashes.push(change_webcash.to_string());
            println!("  Generated change webcash: {}", change_webcash.to_string());
            Some(change_webcash)
        } else {
            None
        };

        // Create replace request (matches Python exactly)
        let replace_request = crate::server::ReplaceRequest {
            webcashes: inputs.iter().map(|wc| wc.to_string()).collect(),
            new_webcashes,
            legalese: crate::server::Legalese { terms: true },
        };

        // Create server client and submit transaction
        let _server_config = crate::server::ServerConfig::default();
        let server_client = crate::server::ServerClient::new()?;

        println!("📤 Submitting payment transaction to server...");

        let response = server_client.replace(&replace_request).await?;

        if response.status != "success" {
            return Err(Error::server("Payment transaction failed - server rejected replacement"));
        }

        println!("✅ Server accepted payment transaction");

        // Mark inputs as spent in wallet
        self.mark_inputs_spent(&inputs).await?;
        println!("✅ Marked {} inputs as spent", inputs.len());

        // Store change output directly if any (matches Python - change goes back to wallet)  
        if let Some(change_webcash) = change_webcash_for_wallet {
            // Store the same change webcash that was sent to server (no ownership transfer)
            self.store_directly(change_webcash).await?;
        }

        // Return the payment webcash for the recipient to use
        println!("🎉 Payment completed successfully!");
        Ok(format!("Payment completed! Send this webcash to recipient: {}", payment_webcash))
    }

    /// Select unspent inputs that cover the required amount
    async fn select_inputs(&self, amount: Amount) -> Result<Vec<SecretWebcash>> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let mut stmt = connection.prepare(
            "SELECT secret, amount FROM unspent_outputs WHERE spent = 0 ORDER BY amount DESC"
        )?;

        let webcash_iter = stmt.query_map([], |row| {
            let secret_str: String = row.get(0)?;
            let amount_wats: i64 = row.get(1)?;

            Ok((secret_str, amount_wats))
        })?;

        let mut selected = Vec::new();
        let mut total_selected = Amount::ZERO;

        for item in webcash_iter {
            let (secret_str, amount_wats) = item?;
            let secret = crate::webcash::SecureString::new(secret_str);
            let webcash_amount = Amount::from_wats(amount_wats);
            let webcash = SecretWebcash::new(secret, webcash_amount);

            selected.push(webcash.clone());
            total_selected += webcash_amount;

            if total_selected >= amount {
                break;
            }
        }

        if total_selected < amount {
            return Err(Error::wallet("Insufficient funds"));
        }

        Ok(selected)
    }

    /// Mark inputs as spent in the wallet
    pub async fn mark_inputs_spent(&self, inputs: &[SecretWebcash]) -> Result<()> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        for input in inputs {
            let secret_str = input.secret.as_str().unwrap_or("");
            let secret_hash = crate::crypto::sha256(secret_str.as_bytes());

            // Mark as spent in unspent_outputs
            connection.execute(
                "UPDATE unspent_outputs SET spent = 1 WHERE secret_hash = ?1",
                params![&secret_hash[..]],
            )?;

            // Add to spent_hashes for tracking
            connection.execute(
                "INSERT OR IGNORE INTO spent_hashes (hash) VALUES (?1)",
                params![&secret_hash[..]],
            )?;
        }

        Ok(())
    }

    /// Update the stored amount of an unspent output to match the server's actual amount.
    /// Used during recovery when the wallet has a stale/incorrect amount.
    pub async fn update_unspent_amount(&self, secret_webcash: &SecretWebcash, correct_amount: Amount) -> Result<()> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let secret_str = secret_webcash.secret.as_str().unwrap_or("");
        let secret_hash = crate::crypto::sha256(secret_str.as_bytes());

        let rows = connection.execute(
            "UPDATE unspent_outputs SET amount = ?1 WHERE secret_hash = ?2 AND spent = 0",
            params![correct_amount.wats, &secret_hash[..]],
        )?;

        if rows == 0 {
            return Err(Error::wallet("No matching unspent output found to update amount"));
        }
        Ok(())
    }

    /// Store master secret in wallet metadata for recovery
    pub async fn store_master_secret(&self, master_secret_hex: &str) -> Result<()> {
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        connection.execute(
            "INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES ('master_secret', ?1)",
            params![master_secret_hex],
        )?;

        println!("✅ Master secret stored in wallet for recovery purposes");
        Ok(())
    }

    /// Recover wallet from stored master secret in the wallet
    pub async fn recover_from_wallet(&self, gap_limit: usize) -> Result<String> {
        // Try to get master secret from wallet metadata
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let master_secret_hex: Option<String> = connection.query_row(
            "SELECT value FROM wallet_metadata WHERE key = 'master_secret'",
            [],
            |row| row.get(0),
        ).optional()?;

        match master_secret_hex {
            Some(secret) => {
                drop(connection); // Release the lock
                println!("🔑 Found stored master secret, proceeding with recovery...");
                self.recover(&secret, gap_limit).await
            }
            None => {
                Err(Error::wallet("No master secret found in wallet. Use 'setup -p <master_secret>' to create a wallet with a master secret, or use 'recover <master_secret>' to recover from an external master secret."))
            }
        }
    }

    /// Recover wallet from master secret using HD derivation
    pub async fn recover(&self, master_secret_hex: &str, gap_limit: usize) -> Result<String> {
        use crate::server::{ServerClient, ServerConfig};
        use crate::hd::{HDWallet, KeyType};

        println!("🔄 Starting wallet recovery...");
        println!("🎯 Master secret: {}...", &master_secret_hex[..8]);
        println!("📏 Gap limit: {}", gap_limit);

        // Parse master secret from hex
        let master_secret_bytes = hex::decode(master_secret_hex)
            .map_err(|_| Error::wallet("Invalid master secret hex format"))?;

        if master_secret_bytes.len() != 32 {
            return Err(Error::wallet("Master secret must be 32 bytes (64 hex characters)"));
        }

        let mut master_secret_array = [0u8; 32];
        master_secret_array.copy_from_slice(&master_secret_bytes);

        // Create HD wallet instance
        let hd_wallet = HDWallet::from_master_secret(master_secret_array);

        // Initialize recovery statistics
        let mut recovered_count = 0;
        let mut total_recovered_amount = Amount::ZERO;

        // Create server client for validation
        let _server_config = ServerConfig::default();
        let server_client = ServerClient::new()?;

        // Chain codes to scan - matching Python implementation
        let chain_codes = [
            ("RECEIVE", KeyType::Receive),
            ("CHANGE", KeyType::Change),
            ("MINING", KeyType::Mining),
        ];

        // Get reported walletdepths from database (matches Python line 330: reported_walletdepth = webcash_wallet["walletdepths"][chain_code])
        let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let mut reported_depths: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for (chain_name, _) in &chain_codes {
            let depth: i64 = connection.query_row(
                "SELECT depth FROM walletdepths WHERE chain_code = ?",
                params![chain_name],
                |row| row.get(0),
            ).optional()?
                .unwrap_or(0);
            reported_depths.insert(chain_name.to_string(), depth as u64);
        }
        drop(connection);

        // Scan each chain code
        for (chain_name, key_type) in &chain_codes {
            println!("🔍 Scanning chain code: {}", chain_name);

            let reported_walletdepth = *reported_depths.get(*chain_name).unwrap_or(&0);
            let mut consecutive_empty = 0u64;
            let mut current_depth = 0u64;
            let mut last_used_walletdepth = 0u64;
            let mut has_had_webcash = true;

            // Continue scanning until we hit gap limit (matches Python: while has_had_webcash)
            while has_had_webcash {
                // Assume this is the last iteration (Python line 340)
                has_had_webcash = false;
                // Check gap limit batch of addresses
                // Match Python: check_webcashes = {} dictionary keyed by hash
                let mut check_webcashes: std::collections::HashMap<String, (String, u64)> = std::collections::HashMap::new();
                let mut batch_webcash = Vec::new();

                // Generate batch of addresses to check (matches Python: range(current_walletdepth, current_walletdepth + gaplimit))
                for offset in 0..(gap_limit as usize) {
                    let depth = current_depth + offset as u64;

                    let derived_secret_hex = hd_wallet.derive_secret(key_type.to_encoding() as u64, depth as u64)
                        .map_err(|e| Error::crypto(format!("HD derivation failed: {}", e)))?;

                    // Create webcash from derived secret
                    // Match Python: use e1:secret: for discovery (Python uses "e1:secret:" + secret)
                    let derived_secret = crate::webcash::SecureString::new(derived_secret_hex.clone());
                    let test_webcash = SecretWebcash::new(derived_secret, Amount::from_str("1").unwrap());
                    let public_webcash = test_webcash.to_public();
                    let hash_hex = public_webcash.hash_hex();

                    // Match Python: check_webcashes[webcash.to_public().hashed_value] = webcash
                    check_webcashes.insert(hash_hex.clone(), (derived_secret_hex, depth));

                    batch_webcash.push(public_webcash);
                }

                // Check batch with server
                let health_check_result = server_client.health_check(&batch_webcash).await;

                match health_check_result {
                    Ok(response) => {
                        // Match Python: for (public_webcash, result) in response["results"].items():
                        // Iterate server results keyed by public webcash string — order-independent
                        for (public_webcash_str, health_result) in &response.results {
                            // Extract hash from the public webcash string (format: e{amount}:public:{hash})
                            let hash_hex = if let Some(hash_part) = public_webcash_str.split(':').nth(2) {
                                hash_part.to_string()
                            } else {
                                println!("⚠️  Invalid public webcash format: {}", public_webcash_str);
                                continue;
                            };

                            // Match Python: wc = check_webcashes[public_webcash]
                            if let Some((secret_hex, depth)) = check_webcashes.get(&hash_hex) {
                                let depth = *depth;

                                // Match Python line 360-363: track if webcash has been used (spent != None)
                                if health_result.spent.is_some() {
                                    has_had_webcash = true;
                                    consecutive_empty = 0;
                                    // Match Python line 363: last_used_walletdepth = check_webcashes[public_webcash].walletdepth
                                    if depth > last_used_walletdepth {
                                        last_used_walletdepth = depth;
                                    }
                                }

                                // Match Python line 365: only recover if spent == False
                                if health_result.spent == Some(false) {
                                    // Python line 367: use result["amount"] directly
                                    // The server MUST return the amount when spent == False
                                    if let Some(actual_amount_str) = &health_result.amount {
                                        let amount = Amount::from_str(actual_amount_str)
                                            .map_err(|_| Error::wallet("Invalid amount from server"))?;

                                        // Create the actual webcash with correct amount
                                        let actual_secret = crate::webcash::SecureString::new(secret_hex.clone());
                                        let actual_webcash = SecretWebcash::new(actual_secret, amount);

                                        // Store directly — this webcash was created with HD derivation from our master secret
                                        // Match Python line 373: append recovered webcash
                                        match self.store_directly(actual_webcash.clone()).await {
                                            Ok(()) => {
                                                recovered_count += 1;
                                                total_recovered_amount += amount;
                                                has_had_webcash = true;
                                                println!("✅ Recovered: {} WEBCASH at {}/{}", amount, chain_name, depth);
                                            }
                                            Err(e) => {
                                                // Webcash already exists — not an error for recovery
                                                // Python checks `if str(wc) not in webcash_wallet["webcash"]` before appending
                                                if e.to_string().contains("UNIQUE constraint") || e.to_string().contains("already exists") {
                                                    has_had_webcash = true;
                                                    println!("ℹ️  Skipped existing webcash at {}/{}: {} WEBCASH", chain_name, depth, amount);
                                                } else {
                                                    return Err(e);
                                                }
                                            }
                                        }
                                    } else {
                                        // Server should return amount when spent == False
                                        println!("⚠️  Server didn't return amount for unspent webcash at {}/{} - skipping", chain_name, depth);
                                    }
                                }
                                // Note: Python doesn't handle spent == True separately in recovery
                                // It just tracks has_had_webcash = true when spent != None
                            }
                        }
                    }
                    Err(e) => {
                        println!("⚠️  Server error during batch check: {}", e);
                        consecutive_empty += gap_limit as u64;
                    }
                }

                // Match Python line 379: continue anyway if the wallet says its walletdepth is greater
                if current_depth < reported_walletdepth {
                    has_had_webcash = true;
                }

                // Move to next batch (matches Python line 382-383)
                if has_had_webcash {
                    current_depth += gap_limit as u64;
                }

                // Check gap limit (matches Python: consecutive_empty logic)
                if !has_had_webcash && consecutive_empty >= gap_limit as u64 {
                    break;
                }

                // Safety check to prevent infinite loops
                if current_depth > 1000 {
                    println!("⚠️  Safety limit reached - stopping recovery at depth {}", current_depth);
                    break;
                }
            }

            // Match Python line 387-391: update walletdepths after scanning chain
            if last_used_walletdepth > 0 {
                let connection = self.connection.lock().map_err(|_| Error::wallet("Failed to acquire database lock"))?;
                if reported_walletdepth > last_used_walletdepth + 1 {
                    println!("⚠️  Something may have gone wrong: reported walletdepth was {} but only found up to {} depth", reported_walletdepth, last_used_walletdepth);
                }
                if reported_walletdepth < last_used_walletdepth {
                    // Update walletdepths to reflect actual usage (Python line 391)
                    connection.execute(
                        "UPDATE walletdepths SET depth = ? WHERE chain_code = ?",
                        params![(last_used_walletdepth + 1) as i64, chain_name],
                    )?;
                }
                drop(connection);
            }
        }

        Ok(format!(
            "Recovery completed!\n• Webcash recovered: {}\n• Total amount recovered: {} WEBCASH",
            recovered_count, total_recovered_amount
        ))
    }

    /// Merge small outputs to reduce wallet fragmentation
    pub async fn merge(&self, max_outputs: usize) -> Result<String> {
        use crate::server::{ServerClient, ServerConfig};

        println!("🔀 Starting output consolidation...");

        // Get all unspent webcash
        let all_webcash = self.list_webcash().await?;

        if all_webcash.len() <= 1 {
            return Ok("No consolidation needed - wallet has 1 or fewer outputs".to_string());
        }

        // Limit the number of outputs to merge
        let webcash_to_merge = if all_webcash.len() > max_outputs {
            &all_webcash[..max_outputs]
        } else {
            &all_webcash
        };

        if webcash_to_merge.len() <= 1 {
            return Ok("No consolidation needed - insufficient outputs to merge".to_string());
        }

        println!("📊 Consolidating {} outputs into 1...", webcash_to_merge.len());

        // Calculate total amount
        let total_amount: Amount = webcash_to_merge.iter().fold(Amount::ZERO, |acc, wc| acc + wc.amount);

        println!("💰 Total amount to consolidate: {} WEBCASH", total_amount);

        // Generate new secret for consolidated output
        let consolidated_secret = crate::crypto::generate_secret()
            .map_err(|e| Error::crypto(format!("Failed to generate consolidated secret: {}", e)))?;

        let consolidated_webcash = SecretWebcash::new(
            crate::webcash::SecureString::new(consolidated_secret),
            total_amount
        );

        // Prepare transaction
        let new_webcashes = vec![consolidated_webcash.to_string()];

        // Create replace request
        let replace_request = crate::server::ReplaceRequest {
            webcashes: webcash_to_merge.iter().map(|wc| wc.to_string()).collect(),
            new_webcashes,
            legalese: crate::server::Legalese { terms: true },
        };

        // Create server client and submit transaction
        let _server_config = ServerConfig::default();
        let server_client = ServerClient::new()?;

        println!("📤 Submitting consolidation transaction to server...");

        let response = server_client.replace(&replace_request).await?;

        if response.status != "success" {
            return Err(Error::server("Consolidation transaction failed"));
        }

        // Mark inputs as spent in wallet
        self.mark_inputs_spent(webcash_to_merge).await?;

        // Insert consolidated output
        self.insert(consolidated_webcash).await?;

        let summary = format!(
            "Output consolidation completed!\n\
             📊 Results:\n\
             • Outputs merged: {}\n\
             • New consolidated output: 1\n\
             • Total amount preserved: {} WEBCASH\n\
             • Transaction fee: 0 (Webcash has no fees)",
            webcash_to_merge.len(),
            total_amount
        );

        println!("✅ {}", summary);
        Ok(summary)
    }

    /// Encrypt the entire wallet using biometric authentication
    /// 
    /// This method:
    /// 1. Exports all wallet data to a secure format
    /// 2. Encrypts the data using biometric-protected keys
    /// 3. Returns encrypted data that can be stored or transmitted securely
    pub async fn encrypt_with_biometrics(&self) -> Result<EncryptedData> {
        if let Some(ref biometric_mutex) = self.biometric_encryption {
            let mut biometric = biometric_mutex.lock()
                .map_err(|_| Error::wallet("Failed to acquire biometric lock"))?;
            
            // Export wallet data
            let wallet_data = self.export_wallet_data().await?;
            
            // Encrypt with biometrics
            biometric.encrypt_with_biometrics(&wallet_data).await
                .map_err(|e| Error::wallet(&format!("Biometric encryption failed: {}", e)))
        } else {
            Err(Error::wallet("Biometric encryption not enabled for this wallet"))
        }
    }

    /// Decrypt and restore wallet data using biometric authentication
    /// 
    /// This method:
    /// 1. Triggers biometric authentication (Face ID/Touch ID)
    /// 2. Decrypts the wallet data
    /// 3. Restores the wallet state from decrypted data
    pub async fn decrypt_with_biometrics(&self, encrypted_data: &EncryptedData) -> Result<()> {
        if let Some(ref biometric_mutex) = self.biometric_encryption {
            let mut biometric = biometric_mutex.lock()
                .map_err(|_| Error::wallet("Failed to acquire biometric lock"))?;
            
            // Decrypt with biometrics
            let wallet_data = biometric.decrypt_with_biometrics(encrypted_data).await
                .map_err(|e| Error::wallet(&format!("Biometric decryption failed: {}", e)))?;
            
            // Import wallet data
            self.import_wallet_data(&wallet_data).await
        } else {
            Err(Error::wallet("Biometric encryption not enabled for this wallet"))
        }
    }

    /// Encrypt wallet with password (fallback when biometrics unavailable)
    pub async fn encrypt_with_password(&self, password: &str) -> Result<EncryptedData> {
        let wallet_data = self.export_wallet_data().await?;
        crate::biometric::encrypt_with_password(&wallet_data, password)
    }

    /// Decrypt wallet with password
    pub async fn decrypt_with_password(&self, encrypted_data: &EncryptedData, password: &str) -> Result<()> {
        let wallet_data = crate::biometric::decrypt_with_password(encrypted_data, password)?;
        self.import_wallet_data(&wallet_data).await
    }

    /// Export wallet data to bytes for encryption
    async fn export_wallet_data(&self) -> Result<Vec<u8>> {
        use serde_json;

        let connection = self.connection.lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        // Export all wallet metadata
        let mut stmt = connection.prepare(
            "SELECT key, value FROM wallet_metadata ORDER BY key"
        )?;
        let metadata_iter = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut metadata = std::collections::HashMap::new();
        for item in metadata_iter {
            let (key, value) = item?;
            metadata.insert(key, value);
        }

        // Export all unspent outputs
        let mut stmt = connection.prepare(
            "SELECT secret, amount, created_at, spent FROM unspent_outputs ORDER BY id"
        )?;
        let outputs_iter = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, // secret
                row.get::<_, i64>(1)?,    // amount
                row.get::<_, String>(2)?, // created_at
                row.get::<_, i32>(3)?,    // spent
            ))
        })?;

        let mut outputs = Vec::new();
        for item in outputs_iter {
            outputs.push(item?);
        }

        // Export spent hashes
        let mut stmt = connection.prepare(
            "SELECT hash, spent_at FROM spent_hashes ORDER BY id"
        )?;
        let spent_iter = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?, // hash
                row.get::<_, String>(1)?,  // spent_at
            ))
        })?;

        let mut spent_hashes = Vec::new();
        for item in spent_iter {
            spent_hashes.push(item?);
        }

        let wallet_export = WalletExport {
            version: "1.0".to_string(),
            metadata,
            outputs,
            spent_hashes,
            exported_at: chrono::Utc::now().to_rfc3339(),
        };

        serde_json::to_vec(&wallet_export)
            .map_err(|e| Error::wallet(&format!("Failed to serialize wallet data: {}", e)))
    }

    /// Import wallet data from bytes after decryption
    async fn import_wallet_data(&self, data: &[u8]) -> Result<()> {
        use serde_json;

        let wallet_export: WalletExport = serde_json::from_slice(data)
            .map_err(|e| Error::wallet(&format!("Failed to deserialize wallet data: {}", e)))?;

        let connection = self.connection.lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        // Begin transaction
        let tx = connection.unchecked_transaction()?;

        // Clear existing data
        tx.execute("DELETE FROM wallet_metadata", [])?;
        tx.execute("DELETE FROM unspent_outputs", [])?;
        tx.execute("DELETE FROM spent_hashes", [])?;

        // Import metadata
        for (key, value) in wallet_export.metadata {
            tx.execute(
                "INSERT INTO wallet_metadata (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
        }

        // Import outputs
        for (secret, amount, created_at, spent) in wallet_export.outputs {
            let secret_bytes = secret.as_bytes();
            let secret_hash = crate::crypto::sha256(secret_bytes);
            
            tx.execute(
                "INSERT INTO unspent_outputs (secret_hash, secret, amount, created_at, spent) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![&secret_hash[..], secret, amount, created_at, spent],
            )?;
        }

        // Import spent hashes
        for (hash, spent_at) in wallet_export.spent_hashes {
            tx.execute(
                "INSERT INTO spent_hashes (hash, spent_at) VALUES (?1, ?2)",
                params![hash, spent_at],
            )?;
        }

        tx.commit()?;
        println!("✅ Wallet data imported successfully from encrypted backup");
        Ok(())
    }

    /// Check if biometric encryption is enabled for this wallet
    pub fn is_biometric_enabled(&self) -> bool {
        self.biometric_encryption.is_some()
    }

    /// Check if biometric authentication is available on the current device
    pub async fn is_biometric_available(&self) -> Result<bool> {
        if let Some(ref biometric_mutex) = self.biometric_encryption {
            let biometric = biometric_mutex.lock()
                .map_err(|_| Error::wallet("Failed to acquire biometric lock"))?;
            biometric.is_biometric_available().await
        } else {
            Ok(false)
        }
    }

    /// Close the wallet (flushes any pending operations)
    pub async fn close(mut self) -> Result<()> {
        // Encrypt database if needed
        if self.is_encrypted {
            self.encrypt_database().await?;
        }
        
        // Clear any cached biometric keys
        if let Some(biometric_mutex) = self.biometric_encryption.take() {
            let mut biometric = biometric_mutex.into_inner()
                .map_err(|_| Error::wallet("Failed to acquire biometric lock during close"))?;
            biometric.clear_cached_keys();
        }
        
        // The connection will be closed when the Wallet is dropped
        Ok(())
    }
}

impl Drop for Wallet {
    fn drop(&mut self) {
        if self.is_encrypted {
            // Clean up temporary database file
            if let Some(temp_path) = &self.temp_db_path {
                let _ = std::fs::remove_file(temp_path); // Ignore errors during cleanup
            }
        }
    }
}

/// Wallet export format for encryption/backup
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct WalletExport {
    /// Export format version
    version: String,
    /// Wallet metadata (key-value pairs)
    metadata: std::collections::HashMap<String, String>,
    /// Unspent outputs (secret, amount, created_at, spent)
    outputs: Vec<(String, i64, String, i32)>,
    /// Spent hashes (hash, spent_at)
    spent_hashes: Vec<(Vec<u8>, String)>,
    /// When this export was created
    exported_at: String,
}

/// Wallet statistics
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletStats {
    /// Total number of webcash entries (including spent)
    pub total_webcash: u64,
    /// Number of unspent webcash
    pub unspent_webcash: u64,
    /// Number of spent webcash
    pub spent_webcash: u64,
    /// Total balance of unspent webcash
    pub total_balance: Amount,
}


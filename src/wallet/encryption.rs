//! Wallet encryption — database-level and seed-level encryption.

use std::fs;
use std::path::PathBuf;

use super::Wallet;
use crate::passkey::{decrypt_with_password, encrypt_with_password, EncryptedData, EncryptionConfig, PasskeyEncryption};
use crate::error::{Error, Result};

impl Wallet {
    /// Check if a database file is encrypted (not a valid SQLite header).
    pub fn is_database_encrypted(path: &PathBuf) -> Result<bool> {
        if !path.exists() {
            return Ok(false);
        }
        let bytes = fs::read(path)
            .map_err(|e| Error::wallet(format!("Failed to read database file: {}", e)))?;
        if bytes.len() < 16 {
            return Ok(false);
        }
        let sqlite_magic = b"SQLite format 3\0";
        Ok(!bytes.starts_with(sqlite_magic))
    }

    /// Decrypt database for runtime use (returns path to temp decrypted file).
    pub(crate) async fn decrypt_database_for_runtime(encrypted_path: &PathBuf) -> Result<PathBuf> {
        let encrypted_bytes = fs::read(encrypted_path)
            .map_err(|e| Error::wallet(format!("Failed to read encrypted database: {}", e)))?;
        let encrypted_data: EncryptedData = serde_json::from_slice(&encrypted_bytes)
            .map_err(|e| Error::wallet(format!("Invalid encrypted database format: {}", e)))?;

        let file_name = encrypted_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("default");
        let mut passkey = PasskeyEncryption::new(EncryptionConfig {
            app_identifier: "com.webycash.webylib".to_string(),
            service_name: format!("WalletEncryption_{}", file_name),
            ..EncryptionConfig::default()
        })?;
        let decrypted_bytes = passkey
            .decrypt_with_passkey(&encrypted_data)
            .await
            .map_err(|e| Error::wallet(format!("Failed to decrypt database: {}", e)))?;
        let temp_path = encrypted_path.with_extension("temp.db");
        fs::write(&temp_path, decrypted_bytes)
            .map_err(|e| Error::wallet(format!("Failed to write decrypted database: {}", e)))?;
        Ok(temp_path)
    }

    /// Encrypt the wallet database with a password.
    pub async fn encrypt_database_with_password(&self, password: &str) -> Result<()> {
        let db_bytes = fs::read(&self.path)
            .map_err(|e| Error::wallet(format!("Failed to read database: {}", e)))?;
        let encrypted_data = encrypt_with_password(&db_bytes, password)?;
        let encrypted_json = serde_json::to_vec_pretty(&encrypted_data)
            .map_err(|e| Error::wallet(format!("Failed to serialize encrypted data: {}", e)))?;
        fs::write(&self.path, encrypted_json)
            .map_err(|e| Error::wallet(format!("Failed to write encrypted database: {}", e)))?;
        Ok(())
    }

    /// Decrypt the wallet database with a password.
    pub async fn decrypt_database_with_password(&self, password: &str) -> Result<()> {
        let encrypted_bytes = fs::read(&self.path)
            .map_err(|e| Error::wallet(format!("Failed to read encrypted database: {}", e)))?;
        let encrypted_data: EncryptedData = serde_json::from_slice(&encrypted_bytes)
            .map_err(|e| Error::wallet(format!("Invalid encrypted database format: {}", e)))?;
        let decrypted_bytes = decrypt_with_password(&encrypted_data, password)
            .map_err(|e| Error::wallet(format!("Failed to decrypt database: {}", e)))?;
        fs::write(&self.path, decrypted_bytes)
            .map_err(|e| Error::wallet(format!("Failed to write decrypted database: {}", e)))?;
        Ok(())
    }

    /// Encrypt the wallet database using passkey keys.
    pub async fn encrypt_database(&self) -> Result<()> {
        if !self.is_encrypted {
            return Err(Error::wallet("Wallet is not configured for encryption"));
        }
        let mut passkey = self
            .passkey_encryption
            .as_ref()
            .ok_or_else(|| Error::wallet("Passkey encryption not available"))?
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire passkey lock"))?;

        let db_bytes = if let Some(temp_path) = &self.temp_db_path {
            fs::read(temp_path)
                .map_err(|e| Error::wallet(format!("Failed to read temp database: {}", e)))?
        } else {
            fs::read(&self.path)
                .map_err(|e| Error::wallet(format!("Failed to read database: {}", e)))?
        };

        let encrypted_data = passkey.encrypt_with_passkey(&db_bytes).await?;
        let encrypted_json = serde_json::to_vec_pretty(&encrypted_data)
            .map_err(|e| Error::wallet(format!("Failed to serialize encrypted data: {}", e)))?;
        fs::write(&self.path, encrypted_json)
            .map_err(|e| Error::wallet(format!("Failed to write encrypted database: {}", e)))?;

        if let Some(temp_path) = &self.temp_db_path {
            let _ = fs::remove_file(temp_path);
        }
        Ok(())
    }

    /// Decrypt the wallet database (must be done during open).
    pub async fn decrypt_database(&self) -> Result<()> {
        if !self.is_encrypted {
            return Err(Error::wallet("Wallet is not encrypted"));
        }
        if self.temp_db_path.is_some() {
            return Ok(());
        }
        Err(Error::wallet(
            "Database decryption must be done during wallet opening",
        ))
    }

    /// Encrypt the entire wallet using passkey authentication.
    /// Creates a PasskeyEncryption instance on the fly if not already present.
    pub async fn encrypt_with_passkey(&self) -> Result<EncryptedData> {
        use crate::passkey::{PasskeyEncryption, EncryptionConfig};

        let wallet_data = self.export_wallet_data().await?;

        if let Some(ref passkey_mutex) = self.passkey_encryption {
            let mut passkey = passkey_mutex
                .lock()
                .map_err(|_| Error::wallet("Failed to acquire passkey lock"))?;
            passkey
                .encrypt_with_passkey(&wallet_data)
                .await
                .map_err(|e| Error::wallet(format!("Passkey encryption failed: {}", e)))
        } else {
            // Create passkey on the fly for one-shot encryption
            let mut passkey = PasskeyEncryption::new(EncryptionConfig {
                app_identifier: "com.webycash.webylib".to_string(),
                service_name: format!(
                    "WalletEncryption_{}",
                    self.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("default")
                ),
                ..EncryptionConfig::default()
            })?;
            passkey
                .encrypt_with_passkey(&wallet_data)
                .await
                .map_err(|e| Error::wallet(format!("Passkey encryption failed: {}", e)))
        }
    }

    /// Decrypt and restore wallet data using passkey authentication.
    /// Creates a PasskeyEncryption instance on the fly if not already present.
    pub async fn decrypt_with_passkey(&self, encrypted_data: &EncryptedData) -> Result<()> {
        use crate::passkey::{PasskeyEncryption, EncryptionConfig};

        if let Some(ref passkey_mutex) = self.passkey_encryption {
            let mut passkey = passkey_mutex
                .lock()
                .map_err(|_| Error::wallet("Failed to acquire passkey lock"))?;
            let wallet_data = passkey
                .decrypt_with_passkey(encrypted_data)
                .await
                .map_err(|e| Error::wallet(format!("Passkey decryption failed: {}", e)))?;
            self.import_wallet_data(&wallet_data).await
        } else {
            let mut passkey = PasskeyEncryption::new(EncryptionConfig {
                app_identifier: "com.webycash.webylib".to_string(),
                service_name: format!(
                    "WalletEncryption_{}",
                    self.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("default")
                ),
                ..EncryptionConfig::default()
            })?;
            let wallet_data = passkey
                .decrypt_with_passkey(encrypted_data)
                .await
                .map_err(|e| Error::wallet(format!("Passkey decryption failed: {}", e)))?;
            self.import_wallet_data(&wallet_data).await
        }
    }

    /// Encrypt wallet with password (fallback when passkeys unavailable).
    pub async fn encrypt_with_password(&self, password: &str) -> Result<EncryptedData> {
        let wallet_data = self.export_wallet_data().await?;
        crate::passkey::encrypt_with_password(&wallet_data, password)
    }

    /// Decrypt wallet with password.
    pub async fn decrypt_with_password(
        &self,
        encrypted_data: &EncryptedData,
        password: &str,
    ) -> Result<()> {
        let wallet_data = crate::passkey::decrypt_with_password(encrypted_data, password)?;
        self.import_wallet_data(&wallet_data).await
    }

    /// Check if passkey encryption is enabled for this wallet.
    pub fn is_passkey_enabled(&self) -> bool {
        self.passkey_encryption.is_some()
    }

    /// Check if passkey authentication is available on the current device.
    pub async fn is_passkey_available(&self) -> Result<bool> {
        if let Some(ref passkey_mutex) = self.passkey_encryption {
            let passkey = passkey_mutex
                .lock()
                .map_err(|_| Error::wallet("Failed to acquire passkey lock"))?;
            passkey.is_passkey_available().await
        } else {
            Ok(false)
        }
    }
}

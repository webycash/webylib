//! Wallet snapshot export/import for backup and recovery.

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::Wallet;
use crate::error::{Error, Result};

/// Complete wallet state snapshot for backup/restore.
#[derive(Serialize, Deserialize, Debug)]
pub struct WalletSnapshot {
    pub master_secret: String,
    pub unspent_outputs: Vec<UnspentOutputSnapshot>,
    pub spent_hashes: Vec<SpentHashSnapshot>,
    pub depths: HashMap<String, i64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UnspentOutputSnapshot {
    pub secret: String,
    pub amount: i64,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SpentHashSnapshot {
    pub hash: String,
    pub spent_at: String,
}

/// Internal export format for encryption/backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WalletExport {
    pub version: String,
    pub metadata: HashMap<String, String>,
    pub outputs: Vec<(String, i64, String, i32)>,
    pub spent_hashes: Vec<(Vec<u8>, String)>,
    pub exported_at: String,
}

impl Wallet {
    /// Export wallet state to a snapshot.
    pub fn export_snapshot(&self) -> Result<WalletSnapshot> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire lock"))?;

        let master_secret: String = connection.query_row(
            "SELECT value FROM wallet_metadata WHERE key = 'master_secret'",
            [],
            |r| r.get(0),
        )?;

        let mut stmt = connection
            .prepare("SELECT secret, amount, created_at FROM unspent_outputs WHERE spent = 0")?;
        let unspent = stmt
            .query_map([], |row| {
                Ok(UnspentOutputSnapshot {
                    secret: row.get(0)?,
                    amount: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut stmt = connection.prepare("SELECT hash, spent_at FROM spent_hashes")?;
        let spent = stmt
            .query_map([], |row| {
                let hash_blob: Vec<u8> = row.get(0)?;
                Ok(SpentHashSnapshot {
                    hash: hex::encode(hash_blob),
                    spent_at: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut stmt = connection.prepare("SELECT chain_code, depth FROM walletdepths")?;
        let depths_map = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<HashMap<String, i64>, _>>()?;

        Ok(WalletSnapshot {
            master_secret,
            unspent_outputs: unspent,
            spent_hashes: spent,
            depths: depths_map,
        })
    }

    /// Import wallet state from a snapshot (overwrites current state).
    pub fn import_snapshot(&self, snapshot: &WalletSnapshot) -> Result<()> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire lock"))?;
        let tx = connection.transaction()?;

        tx.execute(
            "INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES ('master_secret', ?1)",
            params![snapshot.master_secret],
        )?;

        for (code, depth) in &snapshot.depths {
            tx.execute(
                "INSERT OR REPLACE INTO walletdepths (chain_code, depth) VALUES (?1, ?2)",
                params![code, depth],
            )?;
        }

        for item in &snapshot.unspent_outputs {
            let secret_hash = crate::crypto::sha256(item.secret.as_bytes());
            tx.execute(
                "INSERT OR REPLACE INTO unspent_outputs (secret_hash, secret, amount, created_at, spent) VALUES (?1, ?2, ?3, ?4, 0)",
                params![&secret_hash[..], item.secret, item.amount, item.created_at],
            )?;
        }

        for item in &snapshot.spent_hashes {
            let hash_bytes =
                hex::decode(&item.hash).map_err(|_| Error::wallet("Invalid hex in snapshot"))?;
            tx.execute(
                "INSERT OR REPLACE INTO spent_hashes (hash, spent_at) VALUES (?1, ?2)",
                params![hash_bytes, item.spent_at],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Export wallet data to bytes for encryption.
    pub(crate) async fn export_wallet_data(&self) -> Result<Vec<u8>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let mut stmt = connection.prepare("SELECT key, value FROM wallet_metadata ORDER BY key")?;
        let metadata = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<HashMap<String, String>, _>>()?;

        let mut stmt = connection
            .prepare("SELECT secret, amount, created_at, spent FROM unspent_outputs ORDER BY id")?;
        let outputs = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i32>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut stmt = connection.prepare("SELECT hash, spent_at FROM spent_hashes ORDER BY id")?;
        let spent_hashes = stmt
            .query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let wallet_export = WalletExport {
            version: "1.0".to_string(),
            metadata,
            outputs,
            spent_hashes,
            exported_at: chrono::Utc::now().to_rfc3339(),
        };

        serde_json::to_vec(&wallet_export)
            .map_err(|e| Error::wallet(format!("Failed to serialize wallet data: {}", e)))
    }

    /// Import wallet data from bytes after decryption.
    pub(crate) async fn import_wallet_data(&self, data: &[u8]) -> Result<()> {
        let wallet_export: WalletExport = serde_json::from_slice(data)
            .map_err(|e| Error::wallet(format!("Failed to deserialize wallet data: {}", e)))?;

        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let tx = connection.unchecked_transaction()?;

        tx.execute("DELETE FROM wallet_metadata", [])?;
        tx.execute("DELETE FROM unspent_outputs", [])?;
        tx.execute("DELETE FROM spent_hashes", [])?;

        for (key, value) in wallet_export.metadata {
            tx.execute(
                "INSERT INTO wallet_metadata (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
        }

        for (secret, amount, created_at, spent) in wallet_export.outputs {
            let secret_hash = crate::crypto::sha256(secret.as_bytes());
            tx.execute(
                "INSERT INTO unspent_outputs (secret_hash, secret, amount, created_at, spent) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![&secret_hash[..], secret, amount, created_at, spent],
            )?;
        }

        for (hash, spent_at) in wallet_export.spent_hashes {
            tx.execute(
                "INSERT INTO spent_hashes (hash, spent_at) VALUES (?1, ?2)",
                params![hash, spent_at],
            )?;
        }

        tx.commit()?;
        log::info!("Wallet data imported from encrypted backup");
        Ok(())
    }
}

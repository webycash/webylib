//! Wallet snapshot export/import for backup and recovery.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::Wallet;
use crate::error::Result;

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
    /// Asset flavor: `"webcash"` (default for V1 snapshots), `"rgb"`,
    /// or `"voucher"`. Optional for backward compatibility — V1
    /// snapshots without this field load as Webcash.
    #[serde(default)]
    pub asset_type: Option<String>,
    /// Contract id for issuer-namespaced flavors (RGB / Voucher).
    /// Always None for Webcash.
    #[serde(default)]
    pub contract_id: Option<String>,
    /// Issuer's PGP V4 fingerprint (40 hex chars) for namespaced
    /// flavors. Always None for Webcash.
    #[serde(default)]
    pub issuer_fp: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SpentHashSnapshot {
    pub hash: String,
    pub spent_at: String,
    #[serde(default)]
    pub asset_type: Option<String>,
    #[serde(default)]
    pub contract_id: Option<String>,
    #[serde(default)]
    pub issuer_fp: Option<String>,
}

/// Internal export format for encryption/backup (native only).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WalletExport {
    pub version: String,
    pub metadata: HashMap<String, String>,
    pub outputs: Vec<(String, i64, String, i32)>,
    pub spent_hashes: Vec<(Vec<u8>, String)>,
    pub exported_at: String,
}

impl Wallet {
    pub fn export_snapshot(&self) -> Result<WalletSnapshot> {
        let master_secret = self.store.get_meta("master_secret")?.unwrap_or_default();
        let unspent = self
            .store
            .get_unspent_full()?
            .into_iter()
            .map(|(secret, amount, created_at)| UnspentOutputSnapshot {
                secret,
                amount,
                created_at,
                // Legacy webylib only knows about Webcash. The new
                // wallet-rgb / wallet-voucher will populate the
                // namespace fields when they switch to webylib-storage.
                asset_type: None,
                contract_id: None,
                issuer_fp: None,
            })
            .collect();

        let spent = self
            .store
            .get_spent_hashes_with_time()?
            .into_iter()
            .map(|(hash_blob, spent_at)| SpentHashSnapshot {
                hash: hex::encode(hash_blob),
                spent_at,
                asset_type: None,
                contract_id: None,
                issuer_fp: None,
            })
            .collect();

        let depths = self
            .store
            .get_all_depths()?
            .into_iter()
            .map(|(k, v)| (k, v as i64))
            .collect();

        Ok(WalletSnapshot {
            master_secret,
            unspent_outputs: unspent,
            spent_hashes: spent,
            depths,
        })
    }

    pub fn import_snapshot(&self, snapshot: &WalletSnapshot) -> Result<()> {
        self.store.atomic(&mut |store| {
            store.clear_all()?;
            store.set_meta("master_secret", &snapshot.master_secret)?;

            for (code, depth) in &snapshot.depths {
                store.set_depth(code, *depth as u64)?;
            }

            for item in &snapshot.unspent_outputs {
                let secret_hash = crate::crypto::sha256(item.secret.as_bytes());
                store.insert_output(&secret_hash, &item.secret, item.amount)?;
            }

            for item in &snapshot.spent_hashes {
                let hash_bytes = hex::decode(&item.hash)
                    .map_err(|_| crate::error::Error::wallet("Invalid hex in snapshot"))?;
                store.insert_spent_hash(&hash_bytes)?;
            }

            Ok(())
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn export_wallet_data(&self) -> Result<Vec<u8>> {
        let metadata = self.store.get_all_meta()?;
        let outputs = self.store.get_all_outputs()?;
        let spent_hashes = self.store.get_spent_hashes_with_time()?;

        let wallet_export = WalletExport {
            version: "1.0".to_string(),
            metadata,
            outputs,
            spent_hashes,
            exported_at: chrono::Utc::now().to_rfc3339(),
        };

        serde_json::to_vec(&wallet_export).map_err(|e| {
            crate::error::Error::wallet(format!("Failed to serialize wallet data: {}", e))
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn import_wallet_data(&self, data: &[u8]) -> Result<()> {
        let wallet_export: WalletExport = serde_json::from_slice(data).map_err(|e| {
            crate::error::Error::wallet(format!("Failed to deserialize wallet data: {}", e))
        })?;

        self.store.clear_all()?;

        for (key, value) in wallet_export.metadata {
            self.store.set_meta(&key, &value)?;
        }

        for (secret, amount, _created_at, _spent) in wallet_export.outputs {
            let secret_hash = crate::crypto::sha256(secret.as_bytes());
            self.store.insert_output(&secret_hash, &secret, amount)?;
        }

        for (hash, _spent_at) in wallet_export.spent_hashes {
            self.store.insert_spent_hash(&hash)?;
        }

        log::info!("Wallet data imported from encrypted backup");
        Ok(())
    }
}

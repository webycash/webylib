//! Core wallet operations: insert, pay, merge, recover, check, balance.

use std::str::FromStr;

use rusqlite::{params, OptionalExtension};

use super::Wallet;
use crate::amount::Amount;
use crate::error::{Error, Result};
use crate::hd::HDWallet;
use crate::server::{Legalese, ReplaceRequest};
use crate::webcash::{PublicWebcash, SecretWebcash, SecureString};

/// Statistics about the wallet.
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

/// Result of a wallet check operation.
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub valid_count: usize,
    pub spent_count: usize,
    pub unknown_count: usize,
}

/// Result of a recovery operation.
#[derive(Debug, Clone)]
pub struct RecoveryResult {
    pub recovered_count: usize,
    pub total_amount: Amount,
}

impl std::fmt::Display for RecoveryResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Recovery completed! Webcash recovered: {}, Total amount: {}",
            self.recovered_count, self.total_amount
        )
    }
}

// ── Master secret management ────────────────────────────────────────

impl Wallet {
    /// Get master secret from wallet metadata, generating one if it doesn't exist.
    pub(crate) fn get_or_generate_master_secret(&self) -> Result<String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let master_secret_hex: Option<String> = connection
            .query_row(
                "SELECT value FROM wallet_metadata WHERE key = 'master_secret'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::Database(e).with_context("Failed to query master secret"))?;

        match master_secret_hex {
            Some(secret) => Ok(secret),
            None => {
                let master_secret = crate::crypto::CryptoSecret::generate().map_err(|e| {
                    Error::crypto(&format!("Failed to generate master secret: {}", e))
                })?;
                let hex = master_secret.to_hex();
                connection.execute(
                    "INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES ('master_secret', ?1)",
                    params![hex],
                )?;
                log::info!("Generated new master secret using hardware RNG");
                Ok(hex)
            }
        }
    }

    fn get_master_secret(&self) -> Result<String> {
        self.get_or_generate_master_secret()
    }

    /// Get master secret hex for backup/recovery.
    pub fn master_secret_hex(&self) -> Result<String> {
        self.get_master_secret()
    }

    fn validate_master_secret(&self, hex: &str) -> Result<[u8; 32]> {
        let bytes = hex::decode(hex).map_err(|_| Error::wallet("Invalid master secret format"))?;
        if bytes.len() != 32 {
            return Err(Error::wallet(format!(
                "Master secret must be 32 bytes (64 hex characters), got {} bytes",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }

    /// Store master secret in wallet metadata for recovery.
    pub async fn store_master_secret(&self, master_secret_hex: &str) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        connection.execute(
            "INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES ('master_secret', ?1)",
            params![master_secret_hex],
        )?;
        log::info!("Master secret stored in wallet for recovery purposes");
        Ok(())
    }
}

// ── Balance & listing ───────────────────────────────────────────────

impl Wallet {
    /// Get the current wallet balance as a display string.
    pub async fn balance(&self) -> Result<String> {
        Ok(self.balance_amount().await?.to_string())
    }

    /// Get the current wallet balance as Amount.
    pub async fn balance_amount(&self) -> Result<Amount> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let total_wats: Option<i64> = connection
            .query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM unspent_outputs WHERE spent = 0",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(Amount::from_wats(total_wats.unwrap_or(0)))
    }

    /// List all unspent webcash in the wallet.
    pub async fn list_webcash(&self) -> Result<Vec<SecretWebcash>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let mut stmt = connection.prepare(
            "SELECT secret, amount FROM unspent_outputs WHERE spent = 0 ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows
            .into_iter()
            .map(|(secret_str, wats)| {
                SecretWebcash::new(SecureString::new(secret_str), Amount::from_wats(wats))
            })
            .collect())
    }

    /// List all unspent webcash as public webcash.
    pub async fn list_public_webcash(&self) -> Result<Vec<PublicWebcash>> {
        Ok(self
            .list_webcash()
            .await?
            .iter()
            .map(|wc| wc.to_public())
            .collect())
    }

    /// Get wallet statistics.
    pub async fn stats(&self) -> Result<WalletStats> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let total_webcash: i64 =
            connection.query_row("SELECT COUNT(*) FROM unspent_outputs", [], |row| row.get(0))?;
        let unspent_webcash: i64 = connection.query_row(
            "SELECT COUNT(*) FROM unspent_outputs WHERE spent = 0",
            [],
            |row| row.get(0),
        )?;
        let spent_webcash: i64 =
            connection.query_row("SELECT COUNT(*) FROM spent_hashes", [], |row| row.get(0))?;
        let total_balance_wats: i64 = connection.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM unspent_outputs WHERE spent = 0",
            [],
            |row| row.get(0),
        )?;

        Ok(WalletStats {
            total_webcash: total_webcash as u64,
            unspent_webcash: unspent_webcash as u64,
            spent_webcash: spent_webcash as u64,
            total_balance: Amount::from_wats(total_balance_wats),
        })
    }
}

// ── Store / Insert ──────────────────────────────────────────────────

impl Wallet {
    /// Store webcash directly without ownership transfer (for change, recovery, etc.).
    pub async fn store_directly(&self, webcash: SecretWebcash) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let secret_str = webcash
            .secret
            .as_str()
            .map_err(|_| Error::wallet("Invalid secret encoding"))?;
        let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
        connection.execute(
            "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
            params![&secret_hash[..], secret_str, webcash.amount.wats],
        )?;
        log::debug!("Webcash stored directly: {}", webcash.amount);
        Ok(())
    }

    /// Insert webcash into the wallet with server validation and ownership transfer.
    pub async fn insert(&self, webcash: SecretWebcash) -> Result<()> {
        self.insert_with_validation(webcash, false).await
    }

    /// Insert webcash with optional pre-validation against server.
    pub async fn insert_with_validation(
        &self,
        webcash: SecretWebcash,
        validate_with_server: bool,
    ) -> Result<()> {
        log::debug!("Starting webcash insertion with ownership transfer");

        let master_secret_hex = self.get_master_secret()?;
        let master_secret_array = self.validate_master_secret(&master_secret_hex)?;
        let hd_wallet = HDWallet::from_master_secret(master_secret_array);

        // Get next depth for RECEIVE chain
        let depth = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
            let d: Option<i64> = connection
                .query_row("SELECT COUNT(*) FROM unspent_outputs", [], |row| row.get(0))
                .optional()?;
            d.unwrap_or(0) as u64
        };

        let new_secret_hex = hd_wallet
            .derive_secret(crate::hd::ChainCode::Receive, depth)
            .map_err(|e| Error::crypto(&format!("Failed to generate new secret: {}", e)))?;

        let new_webcash = SecretWebcash::new(SecureString::new(new_secret_hex), webcash.amount);

        // Optional pre-validation
        if validate_with_server {
            self.validate_input_webcash(&webcash).await?;
        }

        // Perform server replacement to transfer ownership
        let replace_request = ReplaceRequest {
            webcashes: vec![webcash.to_string()],
            new_webcashes: vec![new_webcash.to_string()],
            legalese: Legalese { terms: true },
        };

        let server = self
            .server_client
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire server client lock"))?;

        match server.replace(&replace_request).await {
            Ok(resp) if resp.status == "success" => {
                drop(server);
                log::info!("Server replacement successful — ownership transferred");
                // Store the NEW webcash (not the original)
                let connection = self
                    .connection
                    .lock()
                    .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
                let new_secret_str = new_webcash
                    .secret
                    .as_str()
                    .map_err(|_| Error::wallet("Invalid new secret encoding"))?;
                let new_secret_hash = crate::crypto::sha256(new_secret_str.as_bytes());
                connection.execute(
                    "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
                    params![&new_secret_hash[..], new_secret_str, new_webcash.amount.wats],
                )?;
                log::info!("Inserted amount: {}", new_webcash.amount);
                Ok(())
            }
            Err(Error::Server { ref message })
                if message.contains("can only be replaced by itself") =>
            {
                // Same-lineage token — validate unspent then store directly
                log::info!("Same-lineage token detected, storing directly without replace");
                let public_webcash = webcash.to_public();
                let health_response = server.health_check(&[public_webcash.clone()]).await?;
                drop(server);
                if health_response.status != "success" {
                    return Err(Error::server(
                        "Health check failed for same-lineage fallback",
                    ));
                }
                if let Some(hr) = health_response.results.get(&public_webcash.to_string()) {
                    if hr.spent == Some(true) {
                        return Err(Error::wallet("Input webcash has been spent"));
                    }
                }
                self.store_directly(webcash).await
            }
            Ok(_) => Err(Error::server(
                "Server replacement failed — ownership transfer unsuccessful",
            )),
            Err(e) => Err(e),
        }
    }

    /// Validate input webcash against the server (health check).
    async fn validate_input_webcash(&self, webcash: &SecretWebcash) -> Result<()> {
        let server = self
            .server_client
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire server client lock"))?;
        let public_webcash = webcash.to_public();
        let health = server.health_check(&[public_webcash.clone()]).await?;

        if health.status != "success" {
            return Err(Error::server("Server validation failed"));
        }
        if let Some(result) = health.results.get(&public_webcash.to_string()) {
            if let Some(true) = result.spent {
                return Err(Error::wallet("Input webcash has been spent"));
            }
            if let Some(ref server_amount) = result.amount {
                let expected = Amount::from_str(server_amount).map_err(|_| {
                    Error::wallet(&format!("Invalid amount from server: {}", server_amount))
                })?;
                if webcash.amount != expected {
                    return Err(Error::wallet(&format!(
                        "Amount mismatch: provided {}, server says {}",
                        webcash.amount, expected
                    )));
                }
            }
        } else {
            return Err(Error::server("Input webcash not found in server response"));
        }
        Ok(())
    }
}

// ── Pay ─────────────────────────────────────────────────────────────

impl Wallet {
    /// Pay amount using server-validated transaction.
    /// Returns the payment webcash string for the recipient.
    pub async fn pay(&self, amount: Amount, memo: &str) -> Result<String> {
        log::info!("Starting payment: amount={}, memo={}", amount, memo);

        let master_secret_hex = self.get_master_secret()?;
        let master_secret_array = self.validate_master_secret(&master_secret_hex)?;
        let hd_wallet = HDWallet::from_master_secret(master_secret_array);

        let inputs = self.select_inputs(amount).await?;
        if inputs.is_empty() {
            return Err(Error::wallet("Insufficient funds"));
        }

        let input_total: Amount = inputs.iter().fold(Amount::ZERO, |acc, wc| acc + wc.amount);
        let change_amount = input_total - amount;

        // Get and increment depths atomically
        let (pay_depth, change_depth) = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
            let pay_depth: u64 = connection
                .query_row(
                    "SELECT depth FROM walletdepths WHERE chain_code = 'PAY'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(0) as u64;
            connection.execute(
                "UPDATE walletdepths SET depth = ? WHERE chain_code = 'PAY'",
                params![(pay_depth + 1) as i64],
            )?;

            let change_depth: u64 = connection
                .query_row(
                    "SELECT depth FROM walletdepths WHERE chain_code = 'CHANGE'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(0) as u64;
            connection.execute(
                "UPDATE walletdepths SET depth = ? WHERE chain_code = 'CHANGE'",
                params![(change_depth + 1) as i64],
            )?;
            (pay_depth, change_depth)
        };

        // Generate payment output
        let pay_secret = hd_wallet
            .derive_secret(crate::hd::ChainCode::Pay, pay_depth)
            .map_err(|e| Error::crypto(&format!("Failed to generate payment secret: {}", e)))?;
        let payment_webcash = SecretWebcash::new(SecureString::new(pay_secret), amount);
        let mut new_webcashes = vec![payment_webcash.to_string()];

        // Generate change output if needed
        let change_webcash = if change_amount > Amount::ZERO {
            let change_secret = hd_wallet
                .derive_secret(crate::hd::ChainCode::Change, change_depth)
                .map_err(|e| Error::crypto(&format!("Failed to generate change secret: {}", e)))?;
            let cw = SecretWebcash::new(SecureString::new(change_secret), change_amount);
            new_webcashes.push(cw.to_string());
            Some(cw)
        } else {
            None
        };

        let replace_request = ReplaceRequest {
            webcashes: inputs.iter().map(|wc| wc.to_string()).collect(),
            new_webcashes,
            legalese: Legalese { terms: true },
        };

        let server = self
            .server_client
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire server client lock"))?;
        let response = server.replace(&replace_request).await?;
        drop(server);

        if response.status != "success" {
            return Err(Error::server(
                "Payment transaction failed — server rejected replacement",
            ));
        }

        self.mark_inputs_spent(&inputs).await?;

        if let Some(cw) = change_webcash {
            self.store_directly(cw).await?;
        }

        Ok(format!(
            "Payment completed! Send this webcash to recipient: {}",
            payment_webcash
        ))
    }

    /// Select unspent inputs that cover the required amount.
    async fn select_inputs(&self, amount: Amount) -> Result<Vec<SecretWebcash>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let mut stmt = connection.prepare(
            "SELECT secret, amount FROM unspent_outputs WHERE spent = 0 ORDER BY amount DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut selected = Vec::new();
        let mut total = Amount::ZERO;
        for (secret_str, wats) in rows {
            let wc_amount = Amount::from_wats(wats);
            selected.push(SecretWebcash::new(SecureString::new(secret_str), wc_amount));
            total += wc_amount;
            if total >= amount {
                break;
            }
        }
        if total < amount {
            return Err(Error::wallet("Insufficient funds"));
        }
        Ok(selected)
    }

    /// Mark inputs as spent in the wallet.
    pub async fn mark_inputs_spent(&self, inputs: &[SecretWebcash]) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        for input in inputs {
            let secret_str = input.secret.as_str().unwrap_or("");
            let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
            connection.execute(
                "UPDATE unspent_outputs SET spent = 1 WHERE secret_hash = ?1",
                params![&secret_hash[..]],
            )?;
            connection.execute(
                "INSERT OR IGNORE INTO spent_hashes (hash) VALUES (?1)",
                params![&secret_hash[..]],
            )?;
        }
        Ok(())
    }

    /// Update the stored amount of an unspent output to match the server's actual amount.
    pub async fn update_unspent_amount(
        &self,
        secret_webcash: &SecretWebcash,
        correct_amount: Amount,
    ) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let secret_str = secret_webcash.secret.as_str().unwrap_or("");
        let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
        let rows = connection.execute(
            "UPDATE unspent_outputs SET amount = ?1 WHERE secret_hash = ?2 AND spent = 0",
            params![correct_amount.wats, &secret_hash[..]],
        )?;
        if rows == 0 {
            return Err(Error::wallet(
                "No matching unspent output found to update amount",
            ));
        }
        Ok(())
    }
}

// ── Check ───────────────────────────────────────────────────────────

impl Wallet {
    /// Check wallet status against server.
    pub async fn check(&self) -> Result<CheckResult> {
        let public_webcash_list = self.list_public_webcash().await?;
        if public_webcash_list.is_empty() {
            return Ok(CheckResult {
                valid_count: 0,
                spent_count: 0,
                unknown_count: 0,
            });
        }

        let server = self
            .server_client
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire server client lock"))?;
        let health_response = server.health_check(&public_webcash_list).await?;
        drop(server);

        if health_response.status != "success" {
            return Err(Error::server("Server returned non-success status"));
        }

        let mut valid_count = 0;
        let mut spent_count = 0;
        for (_webcash_str, health_result) in &health_response.results {
            if let Some(true) = health_result.spent {
                spent_count += 1;
            } else {
                valid_count += 1;
            }
        }

        Ok(CheckResult {
            valid_count,
            spent_count,
            unknown_count: 0,
        })
    }
}

// ── Merge ───────────────────────────────────────────────────────────

impl Wallet {
    /// Merge small outputs to reduce wallet fragmentation.
    pub async fn merge(&self, max_outputs: usize) -> Result<String> {
        log::info!("Starting output consolidation");

        let all_webcash = self.list_webcash().await?;
        if all_webcash.len() <= 1 {
            return Ok("No consolidation needed — wallet has 1 or fewer outputs".to_string());
        }

        let webcash_to_merge = if all_webcash.len() > max_outputs {
            &all_webcash[..max_outputs]
        } else {
            &all_webcash
        };

        if webcash_to_merge.len() <= 1 {
            return Ok("No consolidation needed — insufficient outputs to merge".to_string());
        }

        let total_amount: Amount = webcash_to_merge
            .iter()
            .fold(Amount::ZERO, |acc, wc| acc + wc.amount);

        let consolidated_secret = crate::crypto::CryptoSecret::generate()
            .map_err(|e| Error::crypto(format!("Failed to generate consolidated secret: {}", e)))?;
        let consolidated_webcash = SecretWebcash::new(
            SecureString::new(consolidated_secret.to_hex()),
            total_amount,
        );

        let replace_request = ReplaceRequest {
            webcashes: webcash_to_merge.iter().map(|wc| wc.to_string()).collect(),
            new_webcashes: vec![consolidated_webcash.to_string()],
            legalese: Legalese { terms: true },
        };

        let server = self
            .server_client
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire server client lock"))?;
        let response = server.replace(&replace_request).await?;
        drop(server);

        if response.status != "success" {
            return Err(Error::server("Consolidation transaction failed"));
        }

        self.mark_inputs_spent(webcash_to_merge).await?;
        self.insert(consolidated_webcash).await?;

        Ok(format!(
            "Consolidation completed: {} outputs merged, total {} preserved",
            webcash_to_merge.len(),
            total_amount
        ))
    }
}

// ── Recover ─────────────────────────────────────────────────────────

impl Wallet {
    /// Recover wallet from stored master secret in the wallet.
    pub async fn recover_from_wallet(&self, gap_limit: usize) -> Result<RecoveryResult> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let master_secret_hex: Option<String> = connection
            .query_row(
                "SELECT value FROM wallet_metadata WHERE key = 'master_secret'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        drop(connection);

        match master_secret_hex {
            Some(secret) => {
                log::info!("Found stored master secret, proceeding with recovery");
                self.recover(&secret, gap_limit).await
            }
            None => Err(Error::wallet(
                "No master secret found in wallet. Use 'setup -p <master_secret>' to create a wallet with a master secret, or use 'recover <master_secret>' to recover from an external master secret.",
            )),
        }
    }

    /// Recover wallet from master secret using HD derivation.
    pub async fn recover(
        &self,
        master_secret_hex: &str,
        gap_limit: usize,
    ) -> Result<RecoveryResult> {
        use crate::hd::ChainCode;

        log::info!(
            "Starting wallet recovery, master_secret={}..., gap_limit={}",
            &master_secret_hex[..8],
            gap_limit
        );

        let master_secret_bytes = hex::decode(master_secret_hex)
            .map_err(|_| Error::wallet("Invalid master secret hex format"))?;
        if master_secret_bytes.len() != 32 {
            return Err(Error::wallet(
                "Master secret must be 32 bytes (64 hex characters)",
            ));
        }
        let mut master_secret_array = [0u8; 32];
        master_secret_array.copy_from_slice(&master_secret_bytes);
        let hd_wallet = HDWallet::from_master_secret(master_secret_array);

        let mut recovered_count = 0;
        let mut total_recovered_amount = Amount::ZERO;

        let chain_codes = [
            ("RECEIVE", ChainCode::Receive),
            ("CHANGE", ChainCode::Change),
            ("MINING", ChainCode::Mining),
        ];

        // Get reported wallet depths
        let reported_depths = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
            let mut depths = std::collections::HashMap::new();
            for (name, _) in &chain_codes {
                let depth: i64 = connection
                    .query_row(
                        "SELECT depth FROM walletdepths WHERE chain_code = ?",
                        params![name],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                depths.insert(name.to_string(), depth as u64);
            }
            depths
        };

        for (chain_name, chain_code) in &chain_codes {
            log::debug!("Scanning chain code: {}", chain_name);

            let reported_walletdepth = *reported_depths.get(*chain_name).unwrap_or(&0);
            let mut has_had_webcash = true;
            let mut current_depth = 0u64;
            let mut last_used_walletdepth = 0u64;
            let mut consecutive_empty = 0u64;

            while has_had_webcash {
                has_had_webcash = false;
                let mut check_webcashes = std::collections::HashMap::new();
                let mut batch_webcash = Vec::new();

                for offset in 0..gap_limit {
                    let depth = current_depth + offset as u64;
                    let derived_secret_hex = hd_wallet
                        .derive_secret(*chain_code, depth)
                        .map_err(|e| Error::crypto(format!("HD derivation failed: {}", e)))?;
                    let test_webcash = SecretWebcash::new(
                        SecureString::new(derived_secret_hex.clone()),
                        Amount::from_str("1").unwrap(),
                    );
                    let public_webcash = test_webcash.to_public();
                    let hash_hex = public_webcash.hash_hex();
                    check_webcashes.insert(hash_hex, (derived_secret_hex, depth));
                    batch_webcash.push(public_webcash);
                }

                let server = self
                    .server_client
                    .lock()
                    .map_err(|_| Error::wallet("Failed to acquire server client lock"))?;
                let health_result = server.health_check(&batch_webcash).await;
                drop(server);

                match health_result {
                    Ok(response) => {
                        for (public_webcash_str, health_result) in &response.results {
                            let hash_hex =
                                if let Some(hash_part) = public_webcash_str.split(':').nth(2) {
                                    hash_part.to_string()
                                } else {
                                    continue;
                                };

                            if let Some((secret_hex, depth)) = check_webcashes.get(&hash_hex) {
                                let depth = *depth;

                                if health_result.spent.is_some() {
                                    has_had_webcash = true;
                                    consecutive_empty = 0;
                                    if depth > last_used_walletdepth {
                                        last_used_walletdepth = depth;
                                    }
                                }

                                if health_result.spent == Some(false) {
                                    if let Some(actual_amount_str) = &health_result.amount {
                                        let amount =
                                            Amount::from_str(actual_amount_str).map_err(|_| {
                                                Error::wallet("Invalid amount from server")
                                            })?;
                                        let actual_webcash = SecretWebcash::new(
                                            SecureString::new(secret_hex.clone()),
                                            amount,
                                        );
                                        match self.store_directly(actual_webcash).await {
                                            Ok(()) => {
                                                recovered_count += 1;
                                                total_recovered_amount += amount;
                                                has_had_webcash = true;
                                                log::info!(
                                                    "Recovered: {} at {}/{}",
                                                    amount,
                                                    chain_name,
                                                    depth
                                                );
                                            }
                                            Err(e)
                                                if e.to_string().contains("UNIQUE constraint")
                                                    || e.to_string().contains("already exists") =>
                                            {
                                                has_had_webcash = true;
                                            }
                                            Err(e) => return Err(e),
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Server error during batch check: {}", e);
                        consecutive_empty += gap_limit as u64;
                    }
                }

                if current_depth < reported_walletdepth {
                    has_had_webcash = true;
                }
                if has_had_webcash {
                    current_depth += gap_limit as u64;
                }
                if !has_had_webcash && consecutive_empty >= gap_limit as u64 {
                    break;
                }
                if current_depth > 1000 {
                    log::warn!(
                        "Safety limit reached — stopping recovery at depth {}",
                        current_depth
                    );
                    break;
                }
            }

            // Update wallet depths after scanning
            if last_used_walletdepth > 0 {
                let connection = self
                    .connection
                    .lock()
                    .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
                if reported_walletdepth < last_used_walletdepth {
                    connection.execute(
                        "UPDATE walletdepths SET depth = ? WHERE chain_code = ?",
                        params![(last_used_walletdepth + 1) as i64, chain_name],
                    )?;
                }
            }
        }

        Ok(RecoveryResult {
            recovered_count,
            total_amount: total_recovered_amount,
        })
    }
}

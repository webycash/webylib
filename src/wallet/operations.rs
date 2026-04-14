//! Core wallet operations: insert, pay, merge, recover, check, balance.
//!
//! All operations that modify wallet state after a server call use SQLite
//! transactions for atomicity.  Crash recovery relies on HD determinism:
//! `recover` re-derives every chain and finds any outputs the server holds.

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
                    Error::crypto(format!("Failed to generate master secret: {}", e))
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

    /// Read the current depth for a chain code from walletdepths.
    fn read_chain_depth(&self, chain_name: &str) -> Result<u64> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let d: i64 = connection
            .query_row(
                "SELECT depth FROM walletdepths WHERE chain_code = ?1",
                params![chain_name],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(d as u64)
    }

    /// Build an HDWallet from the stored master secret.
    fn hd_wallet(&self) -> Result<HDWallet> {
        let hex = self.get_master_secret()?;
        let arr = self.validate_master_secret(&hex)?;
        Ok(HDWallet::from_master_secret(arr))
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
    ///
    /// Matches the Python reference: derives a new RECEIVE-chain secret,
    /// performs a server `/replace` to transfer ownership, then atomically
    /// stores the new output and increments the RECEIVE depth.
    pub async fn insert_with_validation(
        &self,
        webcash: SecretWebcash,
        validate_with_server: bool,
    ) -> Result<()> {
        log::debug!("Starting webcash insertion with ownership transfer");

        let hd_wallet = self.hd_wallet()?;

        // Read RECEIVE depth from walletdepths (matches Python generate_new_secret)
        let depth = self.read_chain_depth("RECEIVE")?;

        let new_secret_hex = hd_wallet
            .derive_secret(crate::hd::ChainCode::Receive, depth)
            .map_err(|e| Error::crypto(format!("Failed to generate new secret: {}", e)))?;

        let new_webcash = SecretWebcash::new(SecureString::new(new_secret_hex), webcash.amount);

        if validate_with_server {
            self.validate_input_webcash(&webcash).await?;
        }

        let replace_request = ReplaceRequest {
            webcashes: vec![webcash.to_string()],
            new_webcashes: vec![new_webcash.to_string()],
            legalese: Legalese { terms: true },
        };

        let server = self.server_client.lock().await;

        match server.replace(&replace_request).await {
            Ok(resp) if resp.status == "success" => {
                drop(server);
                log::info!("Server replacement successful — ownership transferred");
                // Atomically: store output + increment RECEIVE depth
                let mut connection = self
                    .connection
                    .lock()
                    .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
                let tx = connection.transaction()?;
                let new_secret_str = new_webcash
                    .secret
                    .as_str()
                    .map_err(|_| Error::wallet("Invalid new secret encoding"))?;
                let new_secret_hash = crate::crypto::sha256(new_secret_str.as_bytes());
                tx.execute(
                    "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
                    params![&new_secret_hash[..], new_secret_str, new_webcash.amount.wats],
                )?;
                tx.execute(
                    "INSERT INTO walletdepths (chain_code, depth) VALUES ('RECEIVE', ?1)
                     ON CONFLICT(chain_code) DO UPDATE SET depth = excluded.depth",
                    params![(depth + 1) as i64],
                )?;
                tx.commit()?;
                log::info!("Inserted webcash at RECEIVE/{}", depth);
                Ok(())
            }
            Err(Error::Server { ref message })
                if message.contains("can only be replaced by itself") =>
            {
                // Same-lineage token — validate unspent then store directly.
                // No RECEIVE depth increment: no HD derivation used.
                log::info!("Same-lineage token detected, storing directly without replace");
                let public_webcash = webcash.to_public();
                let health_response = server
                    .health_check(std::slice::from_ref(&public_webcash))
                    .await?;
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
        let server = self.server_client.lock().await;
        let public_webcash = webcash.to_public();
        let health = server
            .health_check(std::slice::from_ref(&public_webcash))
            .await?;

        if health.status != "success" {
            return Err(Error::server("Server validation failed"));
        }
        if let Some(result) = health.results.get(&public_webcash.to_string()) {
            if let Some(true) = result.spent {
                return Err(Error::wallet("Input webcash has been spent"));
            }
            if let Some(ref server_amount) = result.amount {
                let expected = Amount::from_str(server_amount).map_err(|_| {
                    Error::wallet(format!("Invalid amount from server: {}", server_amount))
                })?;
                if webcash.amount != expected {
                    return Err(Error::wallet(format!(
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
    ///
    /// Matches the Python reference: derives PAY + CHANGE secrets, performs
    /// server `/replace`, then atomically updates local DB (mark spent, store
    /// change, increment depths) in a single transaction.
    pub async fn pay(&self, amount: Amount, memo: &str) -> Result<String> {
        log::info!("Starting payment: amount={}, memo={}", amount, memo);

        let hd_wallet = self.hd_wallet()?;

        let inputs = self.select_inputs(amount).await?;
        if inputs.is_empty() {
            return Err(Error::wallet("Insufficient funds"));
        }

        let input_total: Amount = inputs.iter().fold(Amount::ZERO, |acc, wc| acc + wc.amount);
        let change_amount = input_total - amount;

        // Read depths before server call (do NOT increment yet)
        let pay_depth = self.read_chain_depth("PAY")?;
        let change_depth = self.read_chain_depth("CHANGE")?;

        // Generate payment output (PAY chain)
        let pay_secret = hd_wallet
            .derive_secret(crate::hd::ChainCode::Pay, pay_depth)
            .map_err(|e| Error::crypto(format!("Failed to generate payment secret: {}", e)))?;
        let payment_webcash = SecretWebcash::new(SecureString::new(pay_secret), amount);
        let mut new_webcashes = vec![payment_webcash.to_string()];

        // Generate change output (CHANGE chain) if needed
        let change_webcash = if change_amount > Amount::ZERO {
            let change_secret = hd_wallet
                .derive_secret(crate::hd::ChainCode::Change, change_depth)
                .map_err(|e| Error::crypto(format!("Failed to generate change secret: {}", e)))?;
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

        let server = self.server_client.lock().await;
        let response = server.replace(&replace_request).await?;
        drop(server);

        if response.status != "success" {
            return Err(Error::server(
                "Payment transaction failed — server rejected replacement",
            ));
        }

        // Server accepted — atomically update local DB
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let tx = connection.transaction()?;

        // Mark inputs as spent
        for input in &inputs {
            let secret_str = input.secret.as_str().unwrap_or("");
            let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
            tx.execute(
                "UPDATE unspent_outputs SET spent = 1 WHERE secret_hash = ?1",
                params![&secret_hash[..]],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO spent_hashes (hash) VALUES (?1)",
                params![&secret_hash[..]],
            )?;
        }

        // Store change output
        if let Some(ref cw) = change_webcash {
            let change_secret_str = cw
                .secret
                .as_str()
                .map_err(|_| Error::wallet("Invalid change secret encoding"))?;
            let change_hash = crate::crypto::sha256(change_secret_str.as_bytes());
            tx.execute(
                "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
                params![&change_hash[..], change_secret_str, cw.amount.wats],
            )?;
        }

        // Increment PAY depth (always)
        tx.execute(
            "INSERT INTO walletdepths (chain_code, depth) VALUES ('PAY', ?1)
             ON CONFLICT(chain_code) DO UPDATE SET depth = excluded.depth",
            params![(pay_depth + 1) as i64],
        )?;

        // Increment CHANGE depth only when a change output was derived
        if change_webcash.is_some() {
            tx.execute(
                "INSERT INTO walletdepths (chain_code, depth) VALUES ('CHANGE', ?1)
                 ON CONFLICT(chain_code) DO UPDATE SET depth = excluded.depth",
                params![(change_depth + 1) as i64],
            )?;
        }

        tx.commit()?;

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

    /// Mark inputs as spent in the wallet (transactional).
    pub async fn mark_inputs_spent(&self, inputs: &[SecretWebcash]) -> Result<()> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let tx = connection.transaction()?;
        for input in inputs {
            let secret_str = input.secret.as_str().unwrap_or("");
            let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
            tx.execute(
                "UPDATE unspent_outputs SET spent = 1 WHERE secret_hash = ?1",
                params![&secret_hash[..]],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO spent_hashes (hash) VALUES (?1)",
                params![&secret_hash[..]],
            )?;
        }
        tx.commit()?;
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

        let server = self.server_client.lock().await;
        let health_response = server.health_check(&public_webcash_list).await?;
        drop(server);

        if health_response.status != "success" {
            return Err(Error::server("Server returned non-success status"));
        }

        let mut valid_count = 0;
        let mut spent_count = 0;
        for health_result in health_response.results.values() {
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
    ///
    /// Matches the Python reference: uses CHANGE chain code for the merged
    /// output (HD-recoverable), performs a single server `/replace`, then
    /// atomically updates local DB.
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

        // Read CHANGE depth (do NOT increment yet — wait for server success)
        let hd_wallet = self.hd_wallet()?;
        let change_depth = self.read_chain_depth("CHANGE")?;

        let change_secret_hex = hd_wallet
            .derive_secret(crate::hd::ChainCode::Change, change_depth)
            .map_err(|e| Error::crypto(format!("Failed to generate change secret: {}", e)))?;
        let consolidated_webcash =
            SecretWebcash::new(SecureString::new(change_secret_hex), total_amount);

        // Single server replace (inputs -> change-derived output)
        let replace_request = ReplaceRequest {
            webcashes: webcash_to_merge.iter().map(|wc| wc.to_string()).collect(),
            new_webcashes: vec![consolidated_webcash.to_string()],
            legalese: Legalese { terms: true },
        };

        let server = self.server_client.lock().await;
        let response = server.replace(&replace_request).await?;
        drop(server);

        if response.status != "success" {
            return Err(Error::server("Consolidation transaction failed"));
        }

        // Server accepted — atomically update local DB
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;
        let tx = connection.transaction()?;

        for input in webcash_to_merge {
            let secret_str = input.secret.as_str().unwrap_or("");
            let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
            tx.execute(
                "UPDATE unspent_outputs SET spent = 1 WHERE secret_hash = ?1",
                params![&secret_hash[..]],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO spent_hashes (hash) VALUES (?1)",
                params![&secret_hash[..]],
            )?;
        }

        let consolidated_secret_str = consolidated_webcash
            .secret
            .as_str()
            .map_err(|_| Error::wallet("Invalid consolidated secret encoding"))?;
        let consolidated_hash = crate::crypto::sha256(consolidated_secret_str.as_bytes());
        tx.execute(
            "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
            params![&consolidated_hash[..], consolidated_secret_str, consolidated_webcash.amount.wats],
        )?;

        // Increment CHANGE depth
        tx.execute(
            "INSERT INTO walletdepths (chain_code, depth) VALUES ('CHANGE', ?1)
             ON CONFLICT(chain_code) DO UPDATE SET depth = excluded.depth",
            params![(change_depth + 1) as i64],
        )?;

        tx.commit()?;

        log::info!(
            "Consolidation completed: {} outputs merged at CHANGE/{}, total {} preserved",
            webcash_to_merge.len(),
            change_depth,
            total_amount
        );

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

        log::info!("Starting wallet recovery with gap_limit={}", gap_limit);

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

                let server = self.server_client.lock().await;
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

// ── Mining helpers ──────────────────────────────────────────────────

impl Wallet {
    /// Derive the next HD secret for the given chain code and increment the depth.
    /// Returns (secret_hex, depth_used).
    pub fn derive_next_secret(&self, chain_code: crate::hd::ChainCode) -> Result<(String, u64)> {
        let master_secret_hex = self.get_master_secret()?;
        let master_secret_array = self.validate_master_secret(&master_secret_hex)?;
        let hd_wallet = HDWallet::from_master_secret(master_secret_array);

        let chain_name = match chain_code {
            crate::hd::ChainCode::Receive => "RECEIVE",
            crate::hd::ChainCode::Pay => "PAY",
            crate::hd::ChainCode::Change => "CHANGE",
            crate::hd::ChainCode::Mining => "MINING",
        };

        let mut connection = self
            .connection
            .lock()
            .map_err(|_| Error::wallet("Failed to acquire database lock"))?;

        let tx = connection.transaction()?;

        // Get current depth (or 0 if not set)
        let depth: i64 = tx
            .query_row(
                "SELECT depth FROM walletdepths WHERE chain_code = ?",
                params![chain_name],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);

        let depth_u64 = depth as u64;

        // Derive secret at this depth
        let secret_hex = hd_wallet
            .derive_secret(chain_code, depth_u64)
            .map_err(|e| Error::crypto(format!("HD derivation failed: {}", e)))?;

        // Increment depth
        tx.execute(
            "INSERT INTO walletdepths (chain_code, depth) VALUES (?, ?)
             ON CONFLICT(chain_code) DO UPDATE SET depth = excluded.depth",
            params![chain_name, depth + 1],
        )?;

        tx.commit()?;

        Ok((secret_hex, depth_u64))
    }

    /// Convenience method: mine webcash using the light CPU miner.
    /// See [`crate::miner::mine`] for details.
    pub async fn mine(&self) -> Result<crate::miner::MineResult> {
        crate::miner::mine(self).await
    }
}

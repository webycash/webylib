//! Core wallet operations: insert, pay, merge, recover, check, balance.
//!
//! All operations use the `Store` trait for storage access.
//! Business logic is identical across SQLite and in-memory backends.

use std::str::FromStr;

use super::Wallet;
use crate::amount::Amount;
use crate::error::{Error, Result};
use crate::hd::HDWallet;
use crate::webcash::{PublicWebcash, SecretWebcash, SecureString};

use crate::server::{Legalese, ReplaceRequest};

/// Statistics about the wallet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletStats {
    pub total_webcash: u64,
    pub unspent_webcash: u64,
    pub spent_webcash: u64,
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
        write!(f, "Recovery completed! Webcash recovered: {}, Total amount: {}", self.recovered_count, self.total_amount)
    }
}

// ── Master secret management ────────────────────────────────────────

impl Wallet {
    pub(crate) fn get_or_generate_master_secret(&self) -> Result<String> {
        match self.store.get_meta("master_secret")? {
            Some(secret) => Ok(secret),
            None => {
                let master_secret = crate::crypto::CryptoSecret::generate()
                    .map_err(|e| Error::crypto(format!("Failed to generate master secret: {}", e)))?;
                let hex = master_secret.to_hex();
                self.store.set_meta("master_secret", &hex)?;
                log::info!("Generated new master secret using hardware RNG");
                Ok(hex)
            }
        }
    }

    fn get_master_secret(&self) -> Result<String> {
        self.get_or_generate_master_secret()
    }

    pub fn master_secret_hex(&self) -> Result<String> {
        self.get_master_secret()
    }

    fn validate_master_secret(&self, hex: &str) -> Result<[u8; 32]> {
        let bytes = hex::decode(hex).map_err(|_| Error::wallet("Invalid master secret format"))?;
        if bytes.len() != 32 {
            return Err(Error::wallet(format!("Master secret must be 32 bytes, got {}", bytes.len())));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }

    pub async fn store_master_secret(&self, master_secret_hex: &str) -> Result<()> {
        self.store.set_meta("master_secret", master_secret_hex)?;
        log::info!("Master secret stored in wallet for recovery purposes");
        Ok(())
    }

    fn read_chain_depth(&self, chain_name: &str) -> Result<u64> {
        self.store.get_depth(chain_name)
    }

    fn hd_wallet(&self) -> Result<HDWallet> {
        let hex = self.get_master_secret()?;
        let arr = self.validate_master_secret(&hex)?;
        Ok(HDWallet::from_master_secret(arr))
    }
}

// ── Balance & listing ───────────────────────────────────────────────

impl Wallet {
    pub async fn balance(&self) -> Result<String> {
        Ok(self.balance_amount().await?.to_string())
    }

    pub async fn balance_amount(&self) -> Result<Amount> {
        Ok(Amount::from_wats(self.store.sum_unspent()?))
    }

    pub async fn list_webcash(&self) -> Result<Vec<SecretWebcash>> {
        let rows = self.store.get_unspent()?;
        Ok(rows.into_iter().map(|(secret, wats)| {
            SecretWebcash::new(SecureString::new(secret), Amount::from_wats(wats))
        }).collect())
    }

    pub async fn list_public_webcash(&self) -> Result<Vec<PublicWebcash>> {
        Ok(self.list_webcash().await?.iter().map(|wc| wc.to_public()).collect())
    }

    pub async fn stats(&self) -> Result<WalletStats> {
        Ok(WalletStats {
            total_webcash: self.store.count_outputs()?,
            unspent_webcash: self.store.count_unspent()?,
            spent_webcash: self.store.count_spent_hashes()?,
            total_balance: Amount::from_wats(self.store.sum_unspent()?),
        })
    }
}

// ── Store / Insert ──────────────────────────────────────────────────

impl Wallet {
    pub async fn store_directly(&self, webcash: SecretWebcash) -> Result<()> {
        let secret_str = webcash.secret.as_str().map_err(|_| Error::wallet("Invalid secret encoding"))?;
        let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
        self.store.insert_output(&secret_hash, secret_str, webcash.amount.wats)?;
        log::debug!("Webcash stored directly: {}", webcash.amount);
        Ok(())
    }

    pub async fn insert(&self, webcash: SecretWebcash) -> Result<()> {
        self.insert_with_validation(webcash, false).await
    }

    pub async fn insert_with_validation(&self, webcash: SecretWebcash, validate_with_server: bool) -> Result<()> {
        log::debug!("Starting webcash insertion with ownership transfer");

        let hd_wallet = self.hd_wallet()?;
        let depth = self.read_chain_depth("RECEIVE")?;
        let new_secret_hex = hd_wallet.derive_secret(crate::hd::ChainCode::Receive, depth)
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

        match self.server_replace(&replace_request).await {
            Ok(resp) if resp.status == "success" => {
                log::info!("Server replacement successful — ownership transferred");
                let new_secret_str = new_webcash.secret.as_str()
                    .map_err(|_| Error::wallet("Invalid new secret encoding"))?;
                let new_secret_hash = crate::crypto::sha256(new_secret_str.as_bytes());
                self.store.insert_output(&new_secret_hash, new_secret_str, new_webcash.amount.wats)?;
                self.store.set_depth("RECEIVE", depth + 1)?;
                log::info!("Inserted webcash at RECEIVE/{}", depth);
                Ok(())
            }
            Err(Error::Server { ref message }) if message.contains("can only be replaced by itself") => {
                log::info!("Same-lineage secret webcash detected, storing directly without replace");
                let public_webcash = webcash.to_public();
                let health_response = self.server_health_check(std::slice::from_ref(&public_webcash)).await?;
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
            Ok(_) => Err(Error::server("Server replacement failed")),
            Err(e) => Err(e),
        }
    }

    async fn validate_input_webcash(&self, webcash: &SecretWebcash) -> Result<()> {
        let public_webcash = webcash.to_public();
        let health = self.server_health_check(std::slice::from_ref(&public_webcash)).await?;
        if health.status != "success" { return Err(Error::server("Server validation failed")); }
        if let Some(result) = health.results.get(&public_webcash.to_string()) {
            if let Some(true) = result.spent { return Err(Error::wallet("Input webcash has been spent")); }
            if let Some(ref server_amount) = result.amount {
                let expected = Amount::from_str(server_amount)
                    .map_err(|_| Error::wallet(format!("Invalid amount from server: {}", server_amount)))?;
                if webcash.amount != expected {
                    return Err(Error::wallet(format!("Amount mismatch: provided {}, server says {}", webcash.amount, expected)));
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
    pub async fn pay(&self, amount: Amount, memo: &str) -> Result<String> {
        log::info!("Starting payment: amount={}, memo={}", amount, memo);

        let hd_wallet = self.hd_wallet()?;
        let inputs = self.select_inputs(amount).await?;
        if inputs.is_empty() { return Err(Error::wallet("Insufficient funds")); }

        let input_total: Amount = inputs.iter().fold(Amount::ZERO, |acc, wc| acc + wc.amount);
        let change_amount = input_total - amount;

        let pay_depth = self.read_chain_depth("PAY")?;
        let change_depth = self.read_chain_depth("CHANGE")?;

        let pay_secret = hd_wallet.derive_secret(crate::hd::ChainCode::Pay, pay_depth)
            .map_err(|e| Error::crypto(format!("Failed to generate payment secret: {}", e)))?;
        let payment_webcash = SecretWebcash::new(SecureString::new(pay_secret), amount);
        let mut new_webcashes = vec![payment_webcash.to_string()];

        let change_webcash = if change_amount > Amount::ZERO {
            let change_secret = hd_wallet.derive_secret(crate::hd::ChainCode::Change, change_depth)
                .map_err(|e| Error::crypto(format!("Failed to generate change secret: {}", e)))?;
            let cw = SecretWebcash::new(SecureString::new(change_secret), change_amount);
            new_webcashes.push(cw.to_string());
            Some(cw)
        } else { None };

        let replace_request = ReplaceRequest {
            webcashes: inputs.iter().map(|wc| wc.to_string()).collect(),
            new_webcashes,
            legalese: Legalese { terms: true },
        };

        let response = self.server_replace(&replace_request).await?;

        if response.status != "success" {
            return Err(Error::server("Payment transaction failed"));
        }

        for input in &inputs {
            let secret_str = input.secret.as_str().unwrap_or("");
            let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
            self.store.mark_spent(&secret_hash)?;
            self.store.insert_spent_hash(&secret_hash)?;
        }

        if let Some(ref cw) = change_webcash {
            let s = cw.secret.as_str().map_err(|_| Error::wallet("Invalid change secret"))?;
            let h = crate::crypto::sha256(s.as_bytes());
            self.store.insert_output(&h, s, cw.amount.wats)?;
        }

        self.store.set_depth("PAY", pay_depth + 1)?;
        if change_webcash.is_some() {
            self.store.set_depth("CHANGE", change_depth + 1)?;
        }

        Ok(format!("Payment completed! Send this webcash to recipient: {}", payment_webcash))
    }

    async fn select_inputs(&self, amount: Amount) -> Result<Vec<SecretWebcash>> {
        let rows = self.store.get_unspent()?;
        let mut selected = Vec::new();
        let mut total = Amount::ZERO;
        for (secret_str, wats) in rows {
            let wc_amount = Amount::from_wats(wats);
            selected.push(SecretWebcash::new(SecureString::new(secret_str), wc_amount));
            total += wc_amount;
            if total >= amount { break; }
        }
        if total < amount { return Err(Error::wallet("Insufficient funds")); }
        Ok(selected)
    }

    pub async fn mark_inputs_spent(&self, inputs: &[SecretWebcash]) -> Result<()> {
        for input in inputs {
            let secret_str = input.secret.as_str().unwrap_or("");
            let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
            self.store.mark_spent(&secret_hash)?;
            self.store.insert_spent_hash(&secret_hash)?;
        }
        Ok(())
    }

    pub async fn update_unspent_amount(&self, secret_webcash: &SecretWebcash, correct_amount: Amount) -> Result<()> {
        let secret_str = secret_webcash.secret.as_str().unwrap_or("");
        let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
        self.store.update_output_amount(&secret_hash, correct_amount.wats)?;
        Ok(())
    }
}

// ── Check ───────────────────────────────────────────────────────────

impl Wallet {
    pub async fn check(&self) -> Result<CheckResult> {
        let public_webcash_list = self.list_public_webcash().await?;
        if public_webcash_list.is_empty() {
            return Ok(CheckResult { valid_count: 0, spent_count: 0, unknown_count: 0 });
        }

        let health_response = self.server_health_check(&public_webcash_list).await?;

        if health_response.status != "success" {
            return Err(Error::server("Server returned non-success status"));
        }

        let mut valid_count = 0;
        let mut spent_count = 0;
        for health_result in health_response.results.values() {
            if let Some(true) = health_result.spent { spent_count += 1; }
            else { valid_count += 1; }
        }
        Ok(CheckResult { valid_count, spent_count, unknown_count: 0 })
    }
}

// ── Merge ───────────────────────────────────────────────────────────

impl Wallet {
    pub async fn merge(&self, max_outputs: usize) -> Result<String> {
        log::info!("Starting output consolidation");

        let all_webcash = self.list_webcash().await?;
        if all_webcash.len() <= 1 { return Ok("No consolidation needed".to_string()); }

        let webcash_to_merge = if all_webcash.len() > max_outputs { &all_webcash[..max_outputs] } else { &all_webcash };
        if webcash_to_merge.len() <= 1 { return Ok("Insufficient outputs to merge".to_string()); }

        let total_amount: Amount = webcash_to_merge.iter().fold(Amount::ZERO, |acc, wc| acc + wc.amount);

        let hd_wallet = self.hd_wallet()?;
        let change_depth = self.read_chain_depth("CHANGE")?;
        let change_secret_hex = hd_wallet.derive_secret(crate::hd::ChainCode::Change, change_depth)
            .map_err(|e| Error::crypto(format!("Failed to generate change secret: {}", e)))?;
        let consolidated_webcash = SecretWebcash::new(SecureString::new(change_secret_hex), total_amount);

        let replace_request = ReplaceRequest {
            webcashes: webcash_to_merge.iter().map(|wc| wc.to_string()).collect(),
            new_webcashes: vec![consolidated_webcash.to_string()],
            legalese: Legalese { terms: true },
        };

        let response = self.server_replace(&replace_request).await?;

        if response.status != "success" { return Err(Error::server("Consolidation transaction failed")); }

        for input in webcash_to_merge {
            let s = input.secret.as_str().unwrap_or("");
            let h = crate::crypto::sha256(s.as_bytes());
            self.store.mark_spent(&h)?;
            self.store.insert_spent_hash(&h)?;
        }

        let cs = consolidated_webcash.secret.as_str().map_err(|_| Error::wallet("Invalid consolidated secret"))?;
        let ch = crate::crypto::sha256(cs.as_bytes());
        self.store.insert_output(&ch, cs, consolidated_webcash.amount.wats)?;
        self.store.set_depth("CHANGE", change_depth + 1)?;

        Ok(format!("Consolidation completed: {} outputs merged, total {} preserved", webcash_to_merge.len(), total_amount))
    }
}

// ── Recover ─────────────────────────────────────────────────────────

impl Wallet {
    pub async fn recover_from_wallet(&self, gap_limit: usize) -> Result<RecoveryResult> {
        match self.store.get_meta("master_secret")? {
            Some(secret) => {
                log::info!("Found stored master secret, proceeding with recovery");
                self.recover(&secret, gap_limit).await
            }
            None => Err(Error::wallet("No master secret found in wallet")),
        }
    }

    pub async fn recover(&self, master_secret_hex: &str, gap_limit: usize) -> Result<RecoveryResult> {
        use crate::hd::ChainCode;

        log::info!("Starting wallet recovery with gap_limit={}", gap_limit);

        let master_secret_bytes = hex::decode(master_secret_hex)
            .map_err(|_| Error::wallet("Invalid master secret hex format"))?;
        if master_secret_bytes.len() != 32 {
            return Err(Error::wallet("Master secret must be 32 bytes"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&master_secret_bytes);
        let hd_wallet = HDWallet::from_master_secret(arr);

        let mut recovered_count = 0;
        let mut total_recovered_amount = Amount::ZERO;

        let chain_codes = [
            ("RECEIVE", ChainCode::Receive),
            ("CHANGE", ChainCode::Change),
            ("MINING", ChainCode::Mining),
        ];

        let reported_depths = {
            let mut depths = std::collections::HashMap::new();
            for (name, _) in &chain_codes {
                depths.insert(name.to_string(), self.store.get_depth(name)?);
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
                    let derived_secret_hex = hd_wallet.derive_secret(*chain_code, depth)
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

                let health_result = self.server_health_check(&batch_webcash).await;

                match health_result {
                    Ok(response) => {
                        for (public_webcash_str, health_result) in &response.results {
                            let hash_hex = if let Some(hash_part) = public_webcash_str.split(':').nth(2) {
                                hash_part.to_string()
                            } else { continue; };

                            if let Some((secret_hex, depth)) = check_webcashes.get(&hash_hex) {
                                let depth = *depth;
                                if health_result.spent.is_some() {
                                    has_had_webcash = true;
                                    consecutive_empty = 0;
                                    if depth > last_used_walletdepth { last_used_walletdepth = depth; }
                                }
                                if health_result.spent == Some(false) {
                                    if let Some(actual_amount_str) = &health_result.amount {
                                        let amount = Amount::from_str(actual_amount_str)
                                            .map_err(|_| Error::wallet("Invalid amount from server"))?;
                                        let actual_webcash = SecretWebcash::new(SecureString::new(secret_hex.clone()), amount);
                                        match self.store_directly(actual_webcash).await {
                                            Ok(()) => {
                                                recovered_count += 1;
                                                total_recovered_amount += amount;
                                                has_had_webcash = true;
                                                log::info!("Recovered: {} at {}/{}", amount, chain_name, depth);
                                            }
                                            Err(e) if e.to_string().contains("UNIQUE constraint") || e.to_string().contains("already exists") => {
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

                if current_depth < reported_walletdepth { has_had_webcash = true; }
                if has_had_webcash { current_depth += gap_limit as u64; }
                if !has_had_webcash && consecutive_empty >= gap_limit as u64 { break; }
                if current_depth > 1000 { log::warn!("Safety limit reached at depth {}", current_depth); break; }
            }

            if last_used_walletdepth > 0 && reported_walletdepth < last_used_walletdepth {
                self.store.set_depth(chain_name, last_used_walletdepth + 1)?;
            }
        }

        Ok(RecoveryResult { recovered_count, total_amount: total_recovered_amount })
    }
}

// ── Mining helpers ──────────────────────────────────────────────────

impl Wallet {
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

        let depth = self.store.get_depth(chain_name)?;
        let secret_hex = hd_wallet.derive_secret(chain_code, depth)
            .map_err(|e| Error::crypto(format!("HD derivation failed: {}", e)))?;
        self.store.set_depth(chain_name, depth + 1)?;

        Ok((secret_hex, depth))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn mine(&self) -> Result<crate::miner::MineResult> {
        crate::miner::mine(self).await
    }
}

// ── Server access helpers ──────────────────────────────────────────
//
// Abstract the Mutex (native) vs direct access (WASM) difference.

use crate::server::{HealthResponse, ReplaceResponse};

impl Wallet {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn server_replace(&self, req: &ReplaceRequest) -> Result<ReplaceResponse> {
        self.server_client.lock().await.replace(req).await
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn server_replace(&self, req: &ReplaceRequest) -> Result<ReplaceResponse> {
        self.server_client.replace(req).await
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn server_health_check(&self, webcash: &[PublicWebcash]) -> Result<HealthResponse> {
        self.server_client.lock().await.health_check(webcash).await
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn server_health_check(&self, webcash: &[PublicWebcash]) -> Result<HealthResponse> {
        self.server_client.health_check(webcash).await
    }

    pub async fn server_get_target(&self) -> Result<crate::server::TargetResponse> {
        #[cfg(not(target_arch = "wasm32"))]
        { self.server_client.lock().await.get_target().await }
        #[cfg(target_arch = "wasm32")]
        { self.server_client.get_target().await }
    }

    pub async fn server_submit_mining_report(&self, report: &crate::server::MiningReportRequest) -> Result<crate::server::MiningReportResponse> {
        #[cfg(not(target_arch = "wasm32"))]
        { self.server_client.lock().await.submit_mining_report(report).await }
        #[cfg(target_arch = "wasm32")]
        { self.server_client.submit_mining_report(report).await }
    }
}

//! Core wallet operations: insert, pay, merge, recover, check, balance.
//!
//! All operations use the `Store` trait for storage access.
//! Business logic is identical across SQLite and in-memory backends.

use std::str::FromStr;

use super::Wallet;
use crate::amount::Amount;
use crate::error::{Error, Result};
use crate::hd::HdWallet as HDWallet;
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
        write!(
            f,
            "Recovery completed! Webcash recovered: {}, Total amount: {}",
            self.recovered_count, self.total_amount
        )
    }
}

// ── Master secret management ────────────────────────────────────────

impl Wallet {
    pub(crate) fn get_or_generate_master_secret(&self) -> Result<String> {
        match self.store.get_meta("master_secret")? {
            Some(secret) => Ok(secret),
            None => {
                let master_secret = crate::crypto::CryptoSecret::generate().map_err(|e| {
                    Error::crypto(format!("Failed to generate master secret: {}", e))
                })?;
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
            return Err(Error::wallet(format!(
                "Master secret must be 32 bytes, got {}",
                bytes.len()
            )));
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
        Ok(rows
            .into_iter()
            .map(|(secret, wats)| {
                SecretWebcash::new(SecureString::new(secret), Amount::from_wats(wats))
            })
            .collect())
    }

    pub async fn list_public_webcash(&self) -> Result<Vec<PublicWebcash>> {
        Ok(self
            .list_webcash()
            .await?
            .iter()
            .map(|wc| wc.to_public())
            .collect())
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
        let secret_str = webcash
            .secret
            .as_str()
            .map_err(|_| Error::wallet("Invalid secret encoding"))?;
        let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
        self.store
            .insert_output(&secret_hash, secret_str, webcash.amount.wats)?;
        log::debug!("Webcash stored directly: {}", webcash.amount);
        Ok(())
    }

    pub async fn insert(&self, webcash: SecretWebcash) -> Result<()> {
        self.insert_with_validation(webcash, false).await
    }

    pub async fn insert_with_validation(
        &self,
        webcash: SecretWebcash,
        validate_with_server: bool,
    ) -> Result<()> {
        log::debug!("Starting webcash insertion with ownership transfer");

        let hd_wallet = self.hd_wallet()?;
        let depth = self.read_chain_depth("RECEIVE")?;
        let new_secret_hex = hd_wallet.derive_secret(crate::hd::ChainCode::Receive, depth);
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
            Ok(()) => {
                log::info!("Server replacement successful — ownership transferred");
                let new_secret_str = new_webcash
                    .secret
                    .as_str()
                    .map_err(|_| Error::wallet("Invalid new secret encoding"))?;
                let new_secret_hash = crate::crypto::sha256(new_secret_str.as_bytes());
                self.store.insert_output(
                    &new_secret_hash,
                    new_secret_str,
                    new_webcash.amount.wats,
                )?;
                self.store.set_depth("RECEIVE", depth + 1)?;
                log::info!("Inserted webcash at RECEIVE/{}", depth);
                Ok(())
            }
            Err(Error::Server { ref message })
                if message.contains("can only be replaced by itself") =>
            {
                log::info!(
                    "Same-lineage secret webcash detected, storing directly without replace"
                );
                // Fail closed: store only what the server positively reports unspent.
                match self.server_token_status(&webcash.to_public()).await? {
                    TokenStatus::Unspent { .. } => self.store_directly(webcash).await,
                    TokenStatus::Spent { .. } => Err(Error::wallet("Input webcash has been spent")),
                    TokenStatus::Unknown => {
                        Err(Error::wallet("Input webcash is unknown to the server"))
                    }
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn validate_input_webcash(&self, webcash: &SecretWebcash) -> Result<()> {
        match self.server_token_status(&webcash.to_public()).await? {
            TokenStatus::Spent { .. } => Err(Error::wallet("Input webcash has been spent")),
            TokenStatus::Unknown => Err(Error::wallet("Input webcash is unknown to the server")),
            TokenStatus::Unspent { amount } => {
                if let Some(server_amount) = amount {
                    let expected = Amount::from_str(&server_amount).map_err(|_| {
                        Error::wallet(format!("Invalid amount from server: {}", server_amount))
                    })?;
                    if webcash.amount != expected {
                        return Err(Error::wallet(format!(
                            "Amount mismatch: provided {}, server says {}",
                            webcash.amount, expected
                        )));
                    }
                }
                Ok(())
            }
        }
    }
}

// ── Pay ─────────────────────────────────────────────────────────────

impl Wallet {
    /// Pay `amount`; returns the human-readable receipt carrying the
    /// payment webcash. Callers that need the token use [`Self::pay_webcash`].
    pub async fn pay(&self, amount: Amount, memo: &str) -> Result<String> {
        let payment_webcash = self.pay_webcash(amount, memo).await?;
        Ok(format!(
            "Payment completed! Send this webcash to recipient: {}",
            payment_webcash
        ))
    }

    /// Pay `amount` and return the payment webcash (HD PAY chain secret) for
    /// the recipient. Inputs are marked spent and change is stored only after
    /// the server's `/replace` succeeds.
    pub async fn pay_webcash(&self, amount: Amount, memo: &str) -> Result<SecretWebcash> {
        log::info!("Starting payment: amount={}, memo={}", amount, memo);

        let hd_wallet = self.hd_wallet()?;
        let inputs = self.select_inputs(amount).await?;
        if inputs.is_empty() {
            return Err(Error::wallet("Insufficient funds"));
        }

        let input_total: Amount = inputs.iter().fold(Amount::ZERO, |acc, wc| acc + wc.amount);
        let change_amount = input_total - amount;

        let pay_depth = self.read_chain_depth("PAY")?;
        let change_depth = self.read_chain_depth("CHANGE")?;

        let pay_secret = hd_wallet.derive_secret(crate::hd::ChainCode::Pay, pay_depth);
        let payment_webcash = SecretWebcash::new(SecureString::new(pay_secret), amount);
        let mut new_webcashes = vec![payment_webcash.to_string()];

        let change_webcash = if change_amount > Amount::ZERO {
            let change_secret = hd_wallet.derive_secret(crate::hd::ChainCode::Change, change_depth);
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

        self.server_replace(&replace_request).await?;

        // Atomic: mark inputs spent + store change + update depths
        let change_data = change_webcash.as_ref().map(|cw| {
            let s = cw.secret.as_str().unwrap_or("");
            let h = crate::crypto::sha256(s.as_bytes());
            (h, s.to_string(), cw.amount.wats)
        });
        let has_change = change_webcash.is_some();
        self.store.atomic(&mut |store| {
            for input in &inputs {
                let secret_str = input.secret.as_str().unwrap_or("");
                let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
                store.mark_spent(&secret_hash)?;
                store.insert_spent_hash(&secret_hash)?;
            }
            if let Some((ref h, ref s, wats)) = change_data {
                store.insert_output(h, s, wats)?;
            }
            store.set_depth("PAY", pay_depth + 1)?;
            if has_change {
                store.set_depth("CHANGE", change_depth + 1)?;
            }
            Ok(())
        })?;

        Ok(payment_webcash)
    }

    async fn select_inputs(&self, amount: Amount) -> Result<Vec<SecretWebcash>> {
        let rows = self.store.get_unspent()?;
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

    pub async fn mark_inputs_spent(&self, inputs: &[SecretWebcash]) -> Result<()> {
        for input in inputs {
            let secret_str = input.secret.as_str().unwrap_or("");
            let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
            self.store.mark_spent(&secret_hash)?;
            self.store.insert_spent_hash(&secret_hash)?;
        }
        Ok(())
    }

    pub async fn update_unspent_amount(
        &self,
        secret_webcash: &SecretWebcash,
        correct_amount: Amount,
    ) -> Result<()> {
        let secret_str = secret_webcash.secret.as_str().unwrap_or("");
        let secret_hash = crate::crypto::sha256(secret_str.as_bytes());
        self.store
            .update_output_amount(&secret_hash, correct_amount.wats)?;
        Ok(())
    }
}

// ── Check ───────────────────────────────────────────────────────────

impl Wallet {
    pub async fn check(&self) -> Result<CheckResult> {
        let public_webcash_list = self.list_public_webcash().await?;
        if public_webcash_list.is_empty() {
            return Ok(CheckResult {
                valid_count: 0,
                spent_count: 0,
                unknown_count: 0,
            });
        }

        let tokens: Vec<String> = public_webcash_list.iter().map(|w| w.to_string()).collect();
        let statuses = self.server_health_check(&tokens).await?;

        let mut valid_count = 0;
        let mut spent_count = 0;
        let mut unknown_count = 0;
        for status in &statuses {
            match status {
                TokenStatus::Unspent { .. } => valid_count += 1,
                TokenStatus::Spent { .. } => spent_count += 1,
                TokenStatus::Unknown => unknown_count += 1,
            }
        }
        Ok(CheckResult {
            valid_count,
            spent_count,
            unknown_count,
        })
    }
}

// ── Merge ───────────────────────────────────────────────────────────

impl Wallet {
    pub async fn merge(&self, max_outputs: usize) -> Result<String> {
        log::info!("Starting output consolidation");

        let all_webcash = self.list_webcash().await?;
        if all_webcash.len() <= 1 {
            return Ok("No consolidation needed".to_string());
        }

        let webcash_to_merge = if all_webcash.len() > max_outputs {
            &all_webcash[..max_outputs]
        } else {
            &all_webcash
        };
        if webcash_to_merge.len() <= 1 {
            return Ok("Insufficient outputs to merge".to_string());
        }

        let total_amount: Amount = webcash_to_merge
            .iter()
            .fold(Amount::ZERO, |acc, wc| acc + wc.amount);

        let hd_wallet = self.hd_wallet()?;
        let change_depth = self.read_chain_depth("CHANGE")?;
        let change_secret_hex = hd_wallet.derive_secret(crate::hd::ChainCode::Change, change_depth);
        let consolidated_webcash =
            SecretWebcash::new(SecureString::new(change_secret_hex), total_amount);

        let replace_request = ReplaceRequest {
            webcashes: webcash_to_merge.iter().map(|wc| wc.to_string()).collect(),
            new_webcashes: vec![consolidated_webcash.to_string()],
            legalese: Legalese { terms: true },
        };

        self.server_replace(&replace_request).await?;

        let cs = consolidated_webcash
            .secret
            .as_str()
            .map_err(|_| Error::wallet("Invalid consolidated secret"))?
            .to_string();
        let ch = crate::crypto::sha256(cs.as_bytes());
        let cons_wats = consolidated_webcash.amount.wats;
        let merge_count = webcash_to_merge.len();

        self.store.atomic(&mut |store| {
            for input in webcash_to_merge {
                let s = input.secret.as_str().unwrap_or("");
                let h = crate::crypto::sha256(s.as_bytes());
                store.mark_spent(&h)?;
                store.insert_spent_hash(&h)?;
            }
            store.insert_output(&ch, &cs, cons_wats)?;
            store.set_depth("CHANGE", change_depth + 1)?;
            Ok(())
        })?;

        Ok(format!(
            "Consolidation completed: {} outputs merged, total {} preserved",
            merge_count, total_amount
        ))
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

    /// Recover unspent outputs derived from `master_secret_hex` by
    /// scanning the four HD chains and persisting hits the server
    /// reports as `spent: false`.
    ///
    /// Delegates to the asset-generic [`webylib_core::recover`]; the
    /// only webcash-specific work this wrapper does is wiring the
    /// store (insert + chain-depth advance) once `webylib-core` returns
    /// the `RecoveryReport`.
    pub async fn recover(
        &self,
        master_secret_hex: &str,
        gap_limit: usize,
    ) -> Result<RecoveryResult> {
        use crate::core::{ChainCode as CoreChain, RecoveryScan};
        use crate::hd::HdWallet as CoreHd;
        use crate::wallet_webcash::Webcash;
        use std::collections::HashMap;

        log::info!("Starting wallet recovery with gap_limit={}", gap_limit);

        let hd = CoreHd::from_hex(master_secret_hex)
            .map_err(|e| Error::wallet(format!("Invalid master secret: {e}")))?;

        // The legacy chain *names* are the keys our store persists.
        let chain_codes: [(&str, CoreChain); 4] = [
            ("RECEIVE", CoreChain::Receive),
            ("PAY", CoreChain::Pay),
            ("CHANGE", CoreChain::Change),
            ("MINING", CoreChain::Mining),
        ];
        let mut reported_depths: HashMap<CoreChain, u64> = HashMap::new();
        for (name, code) in &chain_codes {
            reported_depths.insert(*code, self.store.get_depth(name)?);
        }

        // The asset-generic, sans-IO scan (shared with the native blocking
        // driver); this wallet's async server client is the transport.
        let mut scan = RecoveryScan::<Webcash>::new(&hd, &(), gap_limit as u64, &reported_depths)
            .map_err(|e| Error::wallet(format!("recovery: {e}")))?;
        while let Some(lookup) = scan.next_lookup() {
            let statuses = self
                .server_health_check(&lookup)
                .await
                .map_err(|e| e.with_context("recovery health_check"))?;
            scan.apply(&statuses)
                .map_err(|e| Error::server(format!("recovery: {e}")))?;
        }
        let report = scan.finish();

        // Persist what came back. `store_directly` handles the SQLite
        // single-use-seal "already exists" case by surfacing it as an
        // error; we tolerate it here so a re-run of `recover()` on a
        // wallet that already holds some recovered tokens is idempotent.
        let mut recovered_count: usize = 0;
        let mut total_recovered_amount = Amount::ZERO;
        for output in &report.recovered {
            // Webcash always carries amount; the trait's
            // SERVER_REPORTS_AMOUNT contract guarantees Some here.
            // `RecoveryScan::apply` fails closed on an unspent hit without an
            // amount for `SERVER_REPORTS_AMOUNT` assets, so this is `Some`.
            let amount_wats = output
                .amount_wats
                .ok_or_else(|| Error::server("recovery: unspent output without amount"))?;
            let amount = Amount::from_wats(amount_wats);
            let webcash = SecretWebcash::new(SecureString::new(output.secret_hex.clone()), amount);
            match self.store_directly(webcash).await {
                Ok(()) => {
                    recovered_count += 1;
                    total_recovered_amount += amount;
                    log::info!(
                        "Recovered: {} at {:?}/{}",
                        amount,
                        output.chain,
                        output.depth
                    );
                }
                Err(e)
                    if e.to_string().contains("UNIQUE constraint")
                        || e.to_string().contains("already exists") =>
                {
                    // Already-stored output — recovery is idempotent.
                }
                Err(e) => return Err(e),
            }
        }

        // Advance per-chain depth markers if the scan saw further-out
        // usage than the wallet's record.
        for (name, code) in &chain_codes {
            let reported = self.store.get_depth(name)?;
            if let Some(seen) = report.last_used_depth.get(code).copied() {
                if seen + 1 > reported {
                    self.store.set_depth(name, seen + 1)?;
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
        let secret_hex = hd_wallet.derive_secret(chain_code, depth);
        self.store.set_depth(chain_name, depth + 1)?;

        Ok((secret_hex, depth))
    }

    #[cfg(feature = "native")]
    pub async fn mine(&self) -> Result<crate::miner::MineResult> {
        crate::miner::mine(self).await
    }

    /// One bounded proof-of-work search against `target` (from
    /// [`Self::server_get_target`]), on every target including wasm32.
    ///
    /// The webcash (and subsidy) secrets come from the HD MINING chain at
    /// the current depth; the depth is only advanced once a solution is
    /// found, and before the report is sent, so a secret is never offered to
    /// the server twice. The mined webcash is stored only after the server
    /// accepts the report. A batch without a solution leaves the wallet
    /// unchanged.
    pub async fn mine_batch(
        &self,
        target: &crate::server::TargetResponse,
        timestamp: u64,
        start_nonce: u64,
        max_attempts: u64,
    ) -> Result<crate::miner::MineBatch> {
        let mining_amount = Amount::from_str(&target.mining_amount)
            .map_err(|e| Error::wallet(format!("invalid mining amount: {e}")))?;
        let subsidy_amount = Amount::from_str(&target.mining_subsidy_amount)
            .map_err(|e| Error::wallet(format!("invalid subsidy amount: {e}")))?;
        let hd_wallet = self.hd_wallet()?;
        let depth = self.read_chain_depth("MINING")?;
        let webcash = SecretWebcash::new(
            SecureString::new(hd_wallet.derive_secret(crate::hd::ChainCode::Mining, depth)),
            mining_amount,
        );
        let subsidy = (!subsidy_amount.is_zero()).then(|| {
            SecretWebcash::new(
                SecureString::new(hd_wallet.derive_secret(crate::hd::ChainCode::Mining, depth + 1)),
                subsidy_amount,
            )
        });
        let used = if subsidy.is_some() { 2 } else { 1 };
        let work = crate::miner::Work {
            webcash: webcash.to_string(),
            subsidy: subsidy.as_ref().map(|s| s.to_string()),
            timestamp,
            difficulty: target.difficulty_target_bits,
        };
        let (attempted, solution) = work.search(start_nonce, max_attempts);
        let batch = |result| crate::miner::MineBatch {
            attempted,
            difficulty: target.difficulty_target_bits,
            mining_amount,
            result,
        };
        let Some(solution) = solution else {
            return Ok(batch(None));
        };

        // Reserve the secrets before they leave the wallet.
        self.store.set_depth("MINING", depth + used)?;
        let report = crate::server::MiningReportRequest {
            preimage: solution.preimage.clone(),
            legalese: Legalese { terms: true },
        };
        self.server_submit_mining_report(&report).await?;
        // We own the HD-derived secret, so no /replace is needed.
        self.store_directly(webcash).await?;

        Ok(batch(Some(crate::miner::MineResult {
            webcash: work.webcash,
            subsidy: work.subsidy,
            preimage: solution.preimage,
            hash: solution.hash_hex,
            difficulty: target.difficulty_target_bits,
            amount: mining_amount,
        })))
    }
}

// ── Server access helpers ──────────────────────────────────────────
//
// Abstract the Mutex (native) vs direct access (WASM) difference.

use crate::server_client::TokenStatus;

impl Wallet {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn server_replace(&self, req: &ReplaceRequest) -> Result<()> {
        self.server_client.lock().await.replace(req).await
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn server_replace(&self, req: &ReplaceRequest) -> Result<()> {
        self.server_client.replace(req).await
    }

    /// Per-token server state of `public_tokens`, in request order.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn server_health_check(
        &self,
        public_tokens: &[String],
    ) -> Result<Vec<TokenStatus>> {
        self.server_client
            .lock()
            .await
            .health_check(public_tokens)
            .await
    }

    /// Per-token server state of `public_tokens`, in request order.
    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn server_health_check(
        &self,
        public_tokens: &[String],
    ) -> Result<Vec<TokenStatus>> {
        self.server_client.health_check(public_tokens).await
    }

    /// Server state of one public webcash.
    async fn server_token_status(&self, public: &PublicWebcash) -> Result<TokenStatus> {
        let mut statuses = self.server_health_check(&[public.to_string()]).await?;
        // `parse_health_check` guarantees one status per requested token.
        statuses
            .pop()
            .ok_or_else(|| Error::server("health_check returned no status"))
    }

    pub async fn server_get_target(&self) -> Result<crate::server::TargetResponse> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.server_client.lock().await.get_target().await
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.server_client.get_target().await
        }
    }

    pub async fn server_submit_mining_report(
        &self,
        report: &crate::server::MiningReportRequest,
    ) -> Result<crate::server::MiningReportResponse> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.server_client
                .lock()
                .await
                .submit_mining_report(report)
                .await
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.server_client.submit_mining_report(report).await
        }
    }
}

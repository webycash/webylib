//! Wallet operations with JSON state — single source of truth.
//!
//! The JS side handles: IndexedDB persistence, fetch() for API, UI.
//! WASM handles: all business logic, crypto, state transitions.
//!
//! Pattern: operation(state, inputs) → (new_state, effects)
//! Effects are instructions for JS to execute (API calls, etc).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

use crate::{derive_secret, format_amount_i64, parse_amount_str, sha256_hex};

// ── Wallet State ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct WalletState {
    pub master_secret: String,
    pub outputs: Vec<Output>,
    pub spent_hashes: Vec<String>,
    pub depths: HashMap<String, u64>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Output {
    pub secret: String,
    pub public_hash: String,
    pub amount: i64,
    pub created_at: String,
    pub spent: bool,
}

// ── Effects (instructions for JS) ────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct WalletResult {
    pub state: WalletState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Effect {
    /// JS must POST to /replace then call wallet_apply_replace with the response
    Replace {
        webcashes: Vec<String>,
        new_webcashes: Vec<String>,
        /// The new output to store on success
        pending_output: Option<Output>,
        /// Chain depths to increment on success
        depth_updates: HashMap<String, u64>,
        /// Secrets to mark spent on success
        mark_spent: Vec<String>,
    },
    /// JS must POST to /health_check with these webcash strings
    HealthCheck { webcashes: Vec<String> },
    /// JS must POST to /mining_report with this preimage
    MiningReport {
        preimage: String,
        /// Output to store on success
        pending_output: Output,
        depth_updates: HashMap<String, u64>,
    },
}

// ── Helpers ──────────────────────────────────────────────────────

fn secret_hash(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn now_iso() -> String {
    // JS will override this with actual timestamp
    "".to_string()
}

impl WalletState {
    fn balance(&self) -> i64 {
        self.outputs
            .iter()
            .filter(|o| !o.spent)
            .map(|o| o.amount)
            .sum()
    }

    fn unspent(&self) -> Vec<&Output> {
        self.outputs.iter().filter(|o| !o.spent).collect()
    }

    fn get_depth(&self, chain: &str) -> u64 {
        self.depths.get(chain).copied().unwrap_or(0)
    }

    fn derive(&self, chain_code: u32, depth: u64) -> Result<String, String> {
        derive_secret(&self.master_secret, chain_code, depth).map_err(|e| format!("{:?}", e))
    }
}

// ── Wallet Operations ────────────────────────────────────────────

#[wasm_bindgen]
pub fn wallet_create(master_secret_hex: Option<String>) -> Result<String, JsError> {
    let master = match master_secret_hex {
        Some(hex) => {
            if hex.len() != 64 || hex::decode(&hex).is_err() {
                return Err(JsError::new("master secret must be 64 hex chars"));
            }
            hex
        }
        None => crate::generate_master_secret()?,
    };

    let state = WalletState {
        master_secret: master,
        outputs: Vec::new(),
        spent_hashes: Vec::new(),
        depths: [
            ("RECEIVE".into(), 0),
            ("PAY".into(), 0),
            ("CHANGE".into(), 0),
            ("MINING".into(), 0),
        ]
        .into(),
    };

    let result = WalletResult {
        state,
        effect: None,
        message: Some("Wallet created".into()),
        error: None,
    };

    serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
}

#[wasm_bindgen]
pub fn wallet_balance(state_json: &str) -> Result<i64, JsError> {
    let state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(state.balance())
}

#[wasm_bindgen]
pub fn wallet_info(state_json: &str) -> Result<String, JsError> {
    let state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    let unspent: Vec<_> = state.outputs.iter().filter(|o| !o.spent).collect();
    let spent_count = state.spent_hashes.len();

    #[derive(Serialize)]
    struct Info {
        balance: String,
        balance_wats: i64,
        unspent_count: usize,
        spent_count: usize,
        total_outputs: usize,
        master_secret: String,
    }

    let balance_wats = state.balance();
    let info = Info {
        balance: format_amount_i64(balance_wats),
        balance_wats,
        unspent_count: unspent.len(),
        spent_count,
        total_outputs: state.outputs.len(),
        master_secret: state.master_secret.clone(),
    };

    serde_json::to_string(&info).map_err(|e| JsError::new(&e.to_string()))
}

/// Prepare an insert (receive) — returns effect for JS to POST /replace
#[wasm_bindgen]
pub fn wallet_prepare_insert(
    state_json: &str,
    webcash_str: &str,
    timestamp: &str,
) -> Result<String, JsError> {
    let state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    let (input_secret, amount_wats) =
        crate::parse_webcash_internal(webcash_str).map_err(|e| JsError::new(&e))?;

    let depth = state.get_depth("RECEIVE");
    let new_secret = state.derive(0, depth).map_err(|e| JsError::new(&e))?;
    let new_hash = secret_hash(&new_secret);

    let input_str = crate::format_webcash(&input_secret, amount_wats);
    let output_str = crate::format_webcash(&new_secret, amount_wats);

    let result = WalletResult {
        state: state.clone(),
        effect: Some(Effect::Replace {
            webcashes: vec![input_str],
            new_webcashes: vec![output_str],
            pending_output: Some(Output {
                secret: new_secret,
                public_hash: new_hash,
                amount: amount_wats,
                created_at: timestamp.to_string(),
                spent: false,
            }),
            depth_updates: [("RECEIVE".into(), depth + 1)].into(),
            mark_spent: Vec::new(),
        }),
        message: None,
        error: None,
    };

    serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// After server confirms replace, apply the state change
#[wasm_bindgen]
pub fn wallet_apply_replace(
    state_json: &str,
    effect_json: &str,
    success: bool,
) -> Result<String, JsError> {
    let mut state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    if !success {
        return Ok(serde_json::to_string(&WalletResult {
            state,
            effect: None,
            message: None,
            error: Some("Server rejected the replace".into()),
        })
        .unwrap());
    }

    let effect: Effect =
        serde_json::from_str(effect_json).map_err(|e| JsError::new(&e.to_string()))?;

    if let Effect::Replace {
        pending_output,
        depth_updates,
        mark_spent,
        ..
    } = effect
    {
        // Mark spent
        for secret in &mark_spent {
            if let Some(out) = state.outputs.iter_mut().find(|o| o.secret == *secret) {
                out.spent = true;
            }
            let hash = secret_hash(secret);
            if !state.spent_hashes.contains(&hash) {
                state.spent_hashes.push(hash);
            }
        }
        // Store new output
        if let Some(output) = pending_output {
            state.outputs.push(output);
        }
        // Update depths
        for (chain, depth) in depth_updates {
            state.depths.insert(chain, depth);
        }
    }

    Ok(serde_json::to_string(&WalletResult {
        state,
        effect: None,
        message: Some("Success".into()),
        error: None,
    })
    .unwrap())
}

/// Prepare a payment — returns effect for JS to POST /replace
#[wasm_bindgen]
pub fn wallet_prepare_pay(
    state_json: &str,
    amount_wats: i64,
    timestamp: &str,
) -> Result<String, JsError> {
    let state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    // Select inputs (largest first)
    let mut unspent: Vec<_> = state.outputs.iter().filter(|o| !o.spent).collect();
    unspent.sort_by(|a, b| b.amount.cmp(&a.amount));

    let mut selected = Vec::new();
    let mut total: i64 = 0;
    for o in &unspent {
        selected.push(o.secret.clone());
        total += o.amount;
        if total >= amount_wats {
            break;
        }
    }

    if total < amount_wats {
        return Err(JsError::new("Insufficient funds"));
    }

    let change = total - amount_wats;
    let pay_depth = state.get_depth("PAY");
    let change_depth = state.get_depth("CHANGE");

    let pay_secret = state.derive(1, pay_depth).map_err(|e| JsError::new(&e))?;
    let pay_str = crate::format_webcash(&pay_secret, amount_wats);
    let mut new_webcashes = vec![pay_str];
    let mut depth_updates: HashMap<String, u64> = [("PAY".into(), pay_depth + 1)].into();

    let mut pending_change: Option<Output> = None;
    if change > 0 {
        let change_secret = state
            .derive(2, change_depth)
            .map_err(|e| JsError::new(&e))?;
        let change_hash = secret_hash(&change_secret);
        new_webcashes.push(crate::format_webcash(&change_secret, change));
        depth_updates.insert("CHANGE".into(), change_depth + 1);
        pending_change = Some(Output {
            secret: change_secret,
            public_hash: change_hash,
            amount: change,
            created_at: timestamp.to_string(),
            spent: false,
        });
    }

    let webcashes: Vec<String> = selected
        .iter()
        .filter_map(|s| {
            state
                .outputs
                .iter()
                .find(|o| o.secret == *s)
                .map(|o| crate::format_webcash(s, o.amount))
        })
        .collect();

    let result = WalletResult {
        state: state.clone(),
        effect: Some(Effect::Replace {
            webcashes,
            new_webcashes,
            pending_output: pending_change,
            depth_updates,
            mark_spent: selected,
        }),
        message: Some(crate::format_webcash(&pay_secret, amount_wats)),
        error: None,
    };

    serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// Prepare merge — returns effect for JS to POST /replace
#[wasm_bindgen]
pub fn wallet_prepare_merge(
    state_json: &str,
    max_outputs: usize,
    timestamp: &str,
) -> Result<String, JsError> {
    let state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    let unspent: Vec<_> = state.outputs.iter().filter(|o| !o.spent).collect();
    if unspent.len() <= 1 {
        return Err(JsError::new("Nothing to merge"));
    }

    let to_merge: Vec<_> = unspent.into_iter().take(max_outputs).collect();
    let total: i64 = to_merge.iter().map(|o| o.amount).sum();

    let change_depth = state.get_depth("CHANGE");
    let change_secret = state
        .derive(2, change_depth)
        .map_err(|e| JsError::new(&e))?;
    let change_hash = secret_hash(&change_secret);

    let webcashes: Vec<String> = to_merge
        .iter()
        .map(|o| crate::format_webcash(&o.secret, o.amount))
        .collect();
    let mark_spent: Vec<String> = to_merge.iter().map(|o| o.secret.clone()).collect();

    let result = WalletResult {
        state: state.clone(),
        effect: Some(Effect::Replace {
            webcashes,
            new_webcashes: vec![crate::format_webcash(&change_secret, total)],
            pending_output: Some(Output {
                secret: change_secret,
                public_hash: change_hash,
                amount: total,
                created_at: timestamp.to_string(),
                spent: false,
            }),
            depth_updates: [("CHANGE".into(), change_depth + 1)].into(),
            mark_spent,
        }),
        message: Some(format!("{} outputs merged", to_merge.len())),
        error: None,
    };

    serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// Build mining preimage — returns JSON preimage for JS to hash and submit
#[wasm_bindgen]
pub fn wallet_prepare_mine(
    state_json: &str,
    difficulty: u32,
    mining_amount: &str,
) -> Result<String, JsError> {
    let state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    let depth = state.get_depth("MINING");
    let secret = state.derive(3, depth).map_err(|e| JsError::new(&e))?;
    let webcash_str = format!("e{}:secret:{}", mining_amount, secret);
    let public_hash = secret_hash(&secret);
    let amount_wats = parse_amount_str(mining_amount)?;

    #[derive(Serialize)]
    struct MineParams {
        secret: String,
        webcash_str: String,
        public_hash: String,
        amount_wats: i64,
        difficulty: u32,
        mining_depth: u64,
    }

    let params = MineParams {
        secret,
        webcash_str,
        public_hash,
        amount_wats,
        difficulty,
        mining_depth: depth,
    };
    serde_json::to_string(&params).map_err(|e| JsError::new(&e.to_string()))
}

/// Store mined webcash directly (no replace needed)
#[wasm_bindgen]
pub fn wallet_store_mined(
    state_json: &str,
    secret: &str,
    amount_wats: i64,
    timestamp: &str,
) -> Result<String, JsError> {
    let mut state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    let public_hash = secret_hash(secret);
    let depth = state.get_depth("MINING");

    state.outputs.push(Output {
        secret: secret.to_string(),
        public_hash,
        amount: amount_wats,
        created_at: timestamp.to_string(),
        spent: false,
    });
    state.depths.insert("MINING".into(), depth + 1);

    let result = WalletResult {
        state,
        effect: None,
        message: Some(format!("Mined {}", format_amount_i64(amount_wats))),
        error: None,
    };

    serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// Build health_check requests for recovery
#[wasm_bindgen]
pub fn wallet_prepare_recover_batch(
    master_secret_hex: &str,
    _chain_name: &str,
    chain_code: u32,
    start_depth: u64,
    batch_size: u32,
) -> Result<String, JsError> {
    #[derive(Serialize)]
    struct RecoverBatch {
        webcash_strings: Vec<String>,
        secrets: Vec<String>,
        hashes: Vec<String>,
        depths: Vec<u64>,
    }

    let mut batch = RecoverBatch {
        webcash_strings: Vec::new(),
        secrets: Vec::new(),
        hashes: Vec::new(),
        depths: Vec::new(),
    };

    for i in 0..batch_size as u64 {
        let depth = start_depth + i;
        let secret = derive_secret(master_secret_hex, chain_code, depth)?;
        let hash = sha256_hex(&secret);
        let public_str = format!("e1:public:{}", hash);
        batch.webcash_strings.push(public_str);
        batch.secrets.push(secret);
        batch.hashes.push(hash);
        batch.depths.push(depth);
    }

    serde_json::to_string(&batch).map_err(|e| JsError::new(&e.to_string()))
}

/// Apply recovery results from health_check response
#[wasm_bindgen]
pub fn wallet_apply_recover(
    state_json: &str,
    batch_json: &str,
    health_results_json: &str,
    timestamp: &str,
) -> Result<String, JsError> {
    let mut state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    #[derive(Deserialize)]
    struct RecoverBatch {
        secrets: Vec<String>,
        hashes: Vec<String>,
        depths: Vec<u64>,
    }

    #[derive(Deserialize)]
    struct HealthResults {
        results: HashMap<String, HealthEntry>,
    }

    #[derive(Deserialize)]
    struct HealthEntry {
        amount: Option<String>,
        spent: Option<bool>,
    }

    let batch: RecoverBatch =
        serde_json::from_str(batch_json).map_err(|e| JsError::new(&e.to_string()))?;
    let health: HealthResults =
        serde_json::from_str(health_results_json).map_err(|e| JsError::new(&e.to_string()))?;

    let mut recovered = 0;
    let mut found_any = false;

    // Build lookup: hash → (secret, depth)
    let lookup: HashMap<&str, (&str, u64)> = batch
        .hashes
        .iter()
        .zip(batch.secrets.iter().zip(batch.depths.iter()))
        .map(|(h, (s, d))| (h.as_str(), (s.as_str(), *d)))
        .collect();

    for (key, entry) in &health.results {
        // Extract hash from "e1:public:<hash>"
        let hash = key.split(':').nth(2).unwrap_or("");
        if let Some((secret, _depth)) = lookup.get(hash) {
            if entry.spent.is_some() {
                found_any = true;
            }
            if entry.spent == Some(false) {
                if let Some(amount_str) = &entry.amount {
                    let amount = parse_amount_str(amount_str).unwrap_or(0);
                    if amount > 0 {
                        // Check not already stored
                        let pub_hash = secret_hash(secret);
                        if !state.outputs.iter().any(|o| o.public_hash == pub_hash) {
                            state.outputs.push(Output {
                                secret: secret.to_string(),
                                public_hash: pub_hash,
                                amount,
                                created_at: timestamp.to_string(),
                                spent: false,
                            });
                            recovered += 1;
                        }
                    }
                }
            }
        }
    }

    #[derive(Serialize)]
    struct RecoverResult {
        state: WalletState,
        recovered: usize,
        found_any: bool,
        balance: i64,
    }

    let result = RecoverResult {
        balance: state.balance(),
        state,
        recovered,
        found_any,
    };

    serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// Export state as snapshot (for backup)
#[wasm_bindgen]
pub fn wallet_export_snapshot(state_json: &str) -> Result<String, JsError> {
    let state: WalletState =
        serde_json::from_str(state_json).map_err(|e| JsError::new(&e.to_string()))?;

    #[derive(Serialize)]
    struct Snapshot {
        master_secret: String,
        unspent_outputs: Vec<SnapshotOutput>,
        spent_hashes: Vec<SnapshotSpent>,
        depths: HashMap<String, u64>,
    }

    #[derive(Serialize)]
    struct SnapshotOutput {
        secret: String,
        amount: i64,
        created_at: String,
    }

    #[derive(Serialize)]
    struct SnapshotSpent {
        hash: String,
        spent_at: String,
    }

    let snapshot = Snapshot {
        master_secret: state.master_secret,
        unspent_outputs: state
            .outputs
            .iter()
            .filter(|o| !o.spent)
            .map(|o| SnapshotOutput {
                secret: o.secret.clone(),
                amount: o.amount,
                created_at: o.created_at.clone(),
            })
            .collect(),
        spent_hashes: state
            .spent_hashes
            .iter()
            .map(|h| SnapshotSpent {
                hash: h.clone(),
                spent_at: String::new(),
            })
            .collect(),
        depths: state.depths,
    };

    serde_json::to_string_pretty(&snapshot).map_err(|e| JsError::new(&e.to_string()))
}

/// Import from snapshot
#[wasm_bindgen]
pub fn wallet_import_snapshot(snapshot_json: &str) -> Result<String, JsError> {
    #[derive(Deserialize)]
    struct Snapshot {
        master_secret: String,
        unspent_outputs: Vec<SnapshotOutput>,
        spent_hashes: Vec<SnapshotSpent>,
        depths: HashMap<String, u64>,
    }

    #[derive(Deserialize)]
    struct SnapshotOutput {
        secret: String,
        amount: i64,
        created_at: String,
    }
    #[derive(Deserialize)]
    struct SnapshotSpent {
        hash: String,
    }

    let snapshot: Snapshot =
        serde_json::from_str(snapshot_json).map_err(|e| JsError::new(&e.to_string()))?;

    let state = WalletState {
        master_secret: snapshot.master_secret,
        outputs: snapshot
            .unspent_outputs
            .into_iter()
            .map(|o| {
                let public_hash = secret_hash(&o.secret);
                Output {
                    secret: o.secret,
                    public_hash,
                    amount: o.amount,
                    created_at: o.created_at,
                    spent: false,
                }
            })
            .collect(),
        spent_hashes: snapshot.spent_hashes.into_iter().map(|s| s.hash).collect(),
        depths: snapshot.depths,
    };

    let result = WalletResult {
        state,
        effect: None,
        message: Some("Snapshot imported".into()),
        error: None,
    };

    serde_json::to_string(&result).map_err(|e| JsError::new(&e.to_string()))
}

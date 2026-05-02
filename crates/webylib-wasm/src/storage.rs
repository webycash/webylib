//! WASM bindings for the wallet-side storage (`webylib-storage`).
//!
//! Exposes `WeyMemWallet` (in-memory) and `WeyJsonWallet` (in-memory +
//! caller-managed JSON snapshot). Browser consumers persist the JSON
//! blob via `localStorage` / `IndexedDB` / `chrome.storage`.
//!
//! Crypto + HD-derivation primitives live in the parent `lib.rs`; this
//! module focuses on the storage trait surface so a JS-side wallet can
//! track unspent outputs, spent hashes, HD chain depth, and metadata
//! without importing the entire native webylib.

use sha2::{Digest, Sha256};
use wasm_bindgen::prelude::*;
use webylib_storage::{JsonStore, MemStore, Store};

// ─────────────────────────────────────────────────────────────────────────────
// In-memory wallet
// ─────────────────────────────────────────────────────────────────────────────

#[wasm_bindgen]
pub struct WeyMemWallet {
    store: MemStore,
}

#[wasm_bindgen]
impl WeyMemWallet {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            store: MemStore::new(),
        }
    }

    /// Insert an unspent output. `secret_hex` is the 64-char hex secret;
    /// `amount_wats` is the atomic-unit amount (1 webcash = 1e8 wats).
    /// Returns the public hash JS can show in UI.
    #[wasm_bindgen(js_name = insertOutput)]
    pub fn insert_output(&self, secret_hex: &str, amount_wats: i64) -> Result<String, JsValue> {
        let public_hash_hex = hex::encode(Sha256::digest(secret_hex.as_bytes()));
        let public_hash_bytes = hex::decode(&public_hash_hex)
            .map_err(|e| JsValue::from_str(&format!("hex: {e}")))?;
        self.store
            .insert_output(&public_hash_bytes, secret_hex, amount_wats)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))?;
        Ok(public_hash_hex)
    }

    /// Mark a secret spent by its public hash (40-hex SHA256(secret_hex)).
    #[wasm_bindgen(js_name = markSpent)]
    pub fn mark_spent(&self, public_hash_hex: &str) -> Result<(), JsValue> {
        let bytes = hex::decode(public_hash_hex)
            .map_err(|e| JsValue::from_str(&format!("hex: {e}")))?;
        self.store
            .mark_spent(&bytes)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    /// Total unspent value in atomic units.
    #[wasm_bindgen(js_name = balanceWats)]
    pub fn balance_wats(&self) -> Result<i64, JsValue> {
        self.store
            .sum_unspent()
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    /// Number of unspent outputs.
    #[wasm_bindgen(js_name = unspentCount)]
    pub fn unspent_count(&self) -> Result<u32, JsValue> {
        self.store
            .count_unspent()
            .map(|c| c as u32)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    /// HD chain depth getter (Receive / Pay / Change / Mining).
    #[wasm_bindgen(js_name = getDepth)]
    pub fn get_depth(&self, chain: &str) -> Result<u32, JsValue> {
        self.store
            .get_depth(chain)
            .map(|d| d as u32)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    /// HD chain depth setter.
    #[wasm_bindgen(js_name = setDepth)]
    pub fn set_depth(&self, chain: &str, depth: u32) -> Result<(), JsValue> {
        self.store
            .set_depth(chain, depth as u64)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    /// Wallet metadata get/set.
    #[wasm_bindgen(js_name = getMeta)]
    pub fn get_meta(&self, key: &str) -> Result<Option<String>, JsValue> {
        self.store
            .get_meta(key)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = setMeta)]
    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), JsValue> {
        self.store
            .set_meta(key, value)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }
}

impl Default for WeyMemWallet {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// JSON-snapshot wallet
// ─────────────────────────────────────────────────────────────────────────────

#[wasm_bindgen]
pub struct WeyJsonWallet {
    store: JsonStore,
}

#[wasm_bindgen]
impl WeyJsonWallet {
    /// Construct an empty wallet. The JSON snapshot lives on the JS side;
    /// call `toJson` to retrieve it for persistence.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            store: JsonStore::new(None),
        }
    }

    /// Hydrate from a previously-serialised JSON snapshot.
    #[wasm_bindgen(js_name = fromJson)]
    pub fn from_json(json: &str) -> Result<WeyJsonWallet, JsValue> {
        JsonStore::from_json(json, None)
            .map(|store| WeyJsonWallet { store })
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    /// Serialise the current wallet state. JS can store this in
    /// localStorage/IndexedDB.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self) -> Result<String, JsValue> {
        self.store
            .to_json()
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = insertOutput)]
    pub fn insert_output(&self, secret_hex: &str, amount_wats: i64) -> Result<String, JsValue> {
        let public_hash_hex = hex::encode(Sha256::digest(secret_hex.as_bytes()));
        let public_hash_bytes = hex::decode(&public_hash_hex)
            .map_err(|e| JsValue::from_str(&format!("hex: {e}")))?;
        self.store
            .insert_output(&public_hash_bytes, secret_hex, amount_wats)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))?;
        Ok(public_hash_hex)
    }

    #[wasm_bindgen(js_name = balanceWats)]
    pub fn balance_wats(&self) -> Result<i64, JsValue> {
        self.store
            .sum_unspent()
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = unspentCount)]
    pub fn unspent_count(&self) -> Result<u32, JsValue> {
        self.store
            .count_unspent()
            .map(|c| c as u32)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = markSpent)]
    pub fn mark_spent(&self, public_hash_hex: &str) -> Result<(), JsValue> {
        let bytes = hex::decode(public_hash_hex)
            .map_err(|e| JsValue::from_str(&format!("hex: {e}")))?;
        self.store
            .mark_spent(&bytes)
            .map_err(|e: webylib_storage::StoreError| JsValue::from_str(&e.to_string()))
    }
}

impl Default for WeyJsonWallet {
    fn default() -> Self {
        Self::new()
    }
}

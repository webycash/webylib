//! `JsonStore` — `MemStore` plus persistence to a JSON file.
//!
//! Useful for browser wallets (in-memory + Web Storage) and for
//! deterministic snapshot tests. Saves on every mutation; reads from the
//! in-memory state.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::mem::{MemState, MemStore};
use crate::{Store, StoreError, StoreResult};

/// `MemStore` plus persistence to a JSON file. Every mutation
/// flushes the entire state to the configured path; reads come from
/// the in-memory working set. Suitable for small wallets and as a
/// transparent on-disk format. WASM consumers can build with
/// `path = None` and manage the JSON blob via `to_json` / `from_json`.
pub struct JsonStore {
    inner: MemStore,
    path: Mutex<Option<PathBuf>>,
}

impl JsonStore {
    /// Open an in-memory JSON store with no on-disk persistence. Use
    /// `to_json` to retrieve the state.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            inner: MemStore::new(),
            path: Mutex::new(path),
        }
    }

    /// Open from a file path. If the file exists, hydrate from it;
    /// otherwise start empty and persist on first write.
    pub fn open(path: PathBuf) -> StoreResult<Self> {
        let inner = if path.exists() {
            let bytes =
                std::fs::read(&path).map_err(|e| StoreError::Backend(format!("read: {e}")))?;
            let state: MemState = serde_json::from_slice(&bytes)
                .map_err(|e| StoreError::Backend(format!("parse: {e}")))?;
            MemStore::from_state(state)
        } else {
            MemStore::new()
        };
        Ok(Self {
            inner,
            path: Mutex::new(Some(path)),
        })
    }

    /// Hydrate a JsonStore from a string. Useful in WASM where the
    /// caller manages the JSON blob themselves.
    pub fn from_json(json: &str, path: Option<PathBuf>) -> StoreResult<Self> {
        let state: MemState =
            serde_json::from_str(json).map_err(|e| StoreError::Backend(format!("parse: {e}")))?;
        Ok(Self {
            inner: MemStore::from_state(state),
            path: Mutex::new(path),
        })
    }

    /// Serialise the current state to a JSON string. Useful for
    /// WASM consumers that handle persistence externally, and as
    /// the underlying primitive `flush` builds on.
    pub fn to_json(&self) -> StoreResult<String> {
        let snap = self.inner.snapshot();
        serde_json::to_string(&snap).map_err(|e| StoreError::Backend(format!("encode: {e}")))
    }

    fn flush(&self) -> StoreResult<()> {
        let path = match self.path.lock().map_err(|e| {
            StoreError::Backend(format!("path lock: {e}"))
        })?.clone() {
            Some(p) => p,
            None => return Ok(()),
        };
        let json = self.to_json()?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    StoreError::Backend(format!("mkdir: {e}"))
                })?;
            }
        }
        std::fs::write(&path, json.as_bytes())
            .map_err(|e| StoreError::Backend(format!("write: {e}")))?;
        Ok(())
    }
}

impl Store for JsonStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get_meta(&self, key: &str) -> StoreResult<Option<String>> {
        self.inner.get_meta(key)
    }
    fn set_meta(&self, key: &str, value: &str) -> StoreResult<()> {
        self.inner.set_meta(key, value)?;
        self.flush()
    }
    fn get_all_meta(&self) -> StoreResult<std::collections::HashMap<String, String>> {
        self.inner.get_all_meta()
    }
    fn insert_output(
        &self,
        secret_hash: &[u8],
        secret: &str,
        amount: i64,
    ) -> StoreResult<()> {
        self.inner.insert_output(secret_hash, secret, amount)?;
        self.flush()
    }
    fn mark_spent(&self, secret_hash: &[u8]) -> StoreResult<()> {
        self.inner.mark_spent(secret_hash)?;
        self.flush()
    }
    fn insert_spent_hash(&self, hash: &[u8]) -> StoreResult<()> {
        self.inner.insert_spent_hash(hash)?;
        self.flush()
    }
    fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> StoreResult<()> {
        self.inner.update_output_amount(secret_hash, new_amount)?;
        self.flush()
    }
    fn get_unspent(&self) -> StoreResult<Vec<(String, i64)>> {
        self.inner.get_unspent()
    }
    fn get_unspent_full(&self) -> StoreResult<Vec<(String, i64, String)>> {
        self.inner.get_unspent_full()
    }
    fn get_all_outputs(&self) -> StoreResult<Vec<(String, i64, String, i32)>> {
        self.inner.get_all_outputs()
    }
    fn get_spent_hashes_with_time(&self) -> StoreResult<Vec<(Vec<u8>, String)>> {
        self.inner.get_spent_hashes_with_time()
    }
    fn count_outputs(&self) -> StoreResult<u64> {
        self.inner.count_outputs()
    }
    fn count_unspent(&self) -> StoreResult<u64> {
        self.inner.count_unspent()
    }
    fn count_spent_hashes(&self) -> StoreResult<u64> {
        self.inner.count_spent_hashes()
    }
    fn sum_unspent(&self) -> StoreResult<i64> {
        self.inner.sum_unspent()
    }
    fn get_depth(&self, chain: &str) -> StoreResult<u64> {
        self.inner.get_depth(chain)
    }
    fn set_depth(&self, chain: &str, depth: u64) -> StoreResult<()> {
        self.inner.set_depth(chain, depth)?;
        self.flush()
    }
    fn get_all_depths(&self) -> StoreResult<std::collections::HashMap<String, u64>> {
        self.inner.get_all_depths()
    }
    fn clear_all(&self) -> StoreResult<()> {
        self.inner.clear_all()?;
        self.flush()
    }
    fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> StoreResult<()>) -> StoreResult<()> {
        // Defer flush until after the closure completes successfully — so a
        // multi-step transaction lands on disk in one write.
        let snapshot = self.inner.snapshot();
        match f(&self.inner) {
            Ok(()) => self.flush(),
            Err(e) => {
                // Restore inner state before propagating the error.
                let mut g = self
                    .inner
                    .state
                    .lock()
                    .map_err(|le| StoreError::Backend(format!("lock: {le}")))?;
                *g = snapshot;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn open_persists_and_rehydrates() {
        let dir = std::env::temp_dir().join(format!(
            "webylib-jsonstore-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("wallet.json");
        let s = JsonStore::open(path.clone()).unwrap();
        s.set_meta("master_secret", "deadbeef").unwrap();
        s.insert_output(&[1, 2], "secret-1", 1000).unwrap();
        s.set_depth("Receive", 7).unwrap();

        // Drop, re-open, verify state persisted.
        drop(s);
        let s2 = JsonStore::open(path.clone()).unwrap();
        assert_eq!(
            s2.get_meta("master_secret").unwrap(),
            Some("deadbeef".into())
        );
        assert_eq!(s2.count_outputs().unwrap(), 1);
        assert_eq!(s2.get_depth("Receive").unwrap(), 7);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_json_string_roundtrip() {
        let s = JsonStore::new(None);
        s.set_meta("k", "v").unwrap();
        let dump = s.to_json().unwrap();
        let s2 = JsonStore::from_json(&dump, None).unwrap();
        assert_eq!(s2.get_meta("k").unwrap(), Some("v".into()));
    }
}

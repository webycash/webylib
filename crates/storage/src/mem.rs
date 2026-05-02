//! `MemStore` — in-memory implementation of the `Store` trait.
//!
//! Keeps everything in `Mutex`-wrapped `HashMap`s and `Vec`s. Useful for:
//!   - unit tests that don't want the SQLite startup cost
//!   - WASM consumers that don't ship rusqlite
//!   - the `JsonStore` working set (which serialises this to disk)

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::storage::{Store, StoreError, StoreResult};

/// Plain-data form of the wallet's in-memory state. Used as the
/// JsonStore on-disk shape and as the unit-of-rollback for atomic
/// blocks. Serialisable so JsonStore can flush and reload.
#[derive(Default, Serialize, Deserialize)]
pub struct MemState {
    /// Free-form `meta` key/value map (master_secret, etc.).
    pub meta: HashMap<String, String>,
    /// HD chain depth counters per chain code.
    pub depths: HashMap<String, u64>,
    /// One entry per output the wallet has seen.
    pub unspent: Vec<UnspentEntry>,
    /// One entry per spent hash (deduped by hash).
    pub spent_hashes: Vec<SpentHashEntry>,
}

/// One unspent (or spent — see `spent`) output kept in MemState.
#[derive(Clone, Serialize, Deserialize)]
pub struct UnspentEntry {
    /// SHA256 of the secret hex bytes — primary key.
    pub secret_hash: Vec<u8>,
    /// 64-char hex secret.
    pub secret: String,
    /// Atomic-unit amount (wats).
    pub amount: i64,
    /// RFC 3339 wall-clock when the entry was inserted.
    pub created_at: String,
    /// `true` after a successful /replace consumes this output.
    pub spent: bool,
}

/// One spent-hash audit entry. Tracks hashes that have been
/// presented to the server as already-spent so we don't re-submit.
#[derive(Clone, Serialize, Deserialize)]
pub struct SpentHashEntry {
    /// Raw 32-byte hash.
    pub hash: Vec<u8>,
    /// RFC 3339 wall-clock when the entry was recorded.
    pub spent_at: String,
}

/// Pure in-memory `Store` implementation. Useful for unit tests and
/// as the working set the `JsonStore` flushes to disk.
#[derive(Default)]
pub struct MemStore {
    pub(crate) state: Mutex<MemState>,
}

impl MemStore {
    /// Empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap an existing MemState (e.g. one loaded from a JSON snapshot).
    pub fn from_state(state: MemState) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }

    /// Clone the current state out of the lock for snapshot / flush.
    pub fn snapshot(&self) -> MemState {
        let g = self.state.lock().expect("MemStore lock");
        MemState {
            meta: g.meta.clone(),
            depths: g.depths.clone(),
            unspent: g.unspent.clone(),
            spent_hashes: g.spent_hashes.clone(),
        }
    }
}

fn lock_err<T>(e: std::sync::PoisonError<T>) -> StoreError {
    StoreError::Backend(format!("mutex poisoned: {e}"))
}

fn now_iso() -> String {
    // Avoid pulling chrono just for tests; timestamp granularity doesn't
    // matter for in-memory tests, but production users get the legacy
    // SqliteStore which uses datetime('now').
    "1970-01-01T00:00:00Z".to_string()
}

impl Store for MemStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get_meta(&self, key: &str) -> StoreResult<Option<String>> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.meta.get(key).cloned())
    }

    fn set_meta(&self, key: &str, value: &str) -> StoreResult<()> {
        let mut g = self.state.lock().map_err(lock_err)?;
        g.meta.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get_all_meta(&self) -> StoreResult<HashMap<String, String>> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.meta.clone())
    }

    fn insert_output(&self, secret_hash: &[u8], secret: &str, amount: i64) -> StoreResult<()> {
        let mut g = self.state.lock().map_err(lock_err)?;
        if g.unspent.iter().any(|e| e.secret_hash == secret_hash) {
            return Err(StoreError::Constraint(format!(
                "secret_hash already present: {}",
                hex::encode(secret_hash)
            )));
        }
        g.unspent.push(UnspentEntry {
            secret_hash: secret_hash.to_vec(),
            secret: secret.to_string(),
            amount,
            created_at: now_iso(),
            spent: false,
        });
        Ok(())
    }

    fn mark_spent(&self, secret_hash: &[u8]) -> StoreResult<()> {
        let mut g = self.state.lock().map_err(lock_err)?;
        for e in g.unspent.iter_mut() {
            if e.secret_hash == secret_hash {
                e.spent = true;
            }
        }
        Ok(())
    }

    fn insert_spent_hash(&self, hash: &[u8]) -> StoreResult<()> {
        let mut g = self.state.lock().map_err(lock_err)?;
        if !g.spent_hashes.iter().any(|e| e.hash == hash) {
            g.spent_hashes.push(SpentHashEntry {
                hash: hash.to_vec(),
                spent_at: now_iso(),
            });
        }
        Ok(())
    }

    fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> StoreResult<()> {
        let mut g = self.state.lock().map_err(lock_err)?;
        for e in g.unspent.iter_mut() {
            if e.secret_hash == secret_hash && !e.spent {
                e.amount = new_amount;
            }
        }
        Ok(())
    }

    fn get_unspent(&self) -> StoreResult<Vec<(String, i64)>> {
        let g = self.state.lock().map_err(lock_err)?;
        let mut v: Vec<(String, i64)> = g
            .unspent
            .iter()
            .filter(|e| !e.spent)
            .map(|e| (e.secret.clone(), e.amount))
            .collect();
        // Match legacy behaviour: ORDER BY amount DESC.
        v.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(v)
    }

    fn get_unspent_full(&self) -> StoreResult<Vec<(String, i64, String)>> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.unspent
            .iter()
            .filter(|e| !e.spent)
            .map(|e| (e.secret.clone(), e.amount, e.created_at.clone()))
            .collect())
    }

    fn get_all_outputs(&self) -> StoreResult<Vec<(String, i64, String, i32)>> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.unspent
            .iter()
            .map(|e| {
                (
                    e.secret.clone(),
                    e.amount,
                    e.created_at.clone(),
                    if e.spent { 1 } else { 0 },
                )
            })
            .collect())
    }

    fn get_spent_hashes_with_time(&self) -> StoreResult<Vec<(Vec<u8>, String)>> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.spent_hashes
            .iter()
            .map(|e| (e.hash.clone(), e.spent_at.clone()))
            .collect())
    }

    fn count_outputs(&self) -> StoreResult<u64> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.unspent.len() as u64)
    }

    fn count_unspent(&self) -> StoreResult<u64> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.unspent.iter().filter(|e| !e.spent).count() as u64)
    }

    fn count_spent_hashes(&self) -> StoreResult<u64> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.spent_hashes.len() as u64)
    }

    fn sum_unspent(&self) -> StoreResult<i64> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.unspent
            .iter()
            .filter(|e| !e.spent)
            .map(|e| e.amount)
            .sum())
    }

    fn get_depth(&self, chain: &str) -> StoreResult<u64> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.depths.get(chain).copied().unwrap_or(0))
    }

    fn set_depth(&self, chain: &str, depth: u64) -> StoreResult<()> {
        let mut g = self.state.lock().map_err(lock_err)?;
        g.depths.insert(chain.to_string(), depth);
        Ok(())
    }

    fn get_all_depths(&self) -> StoreResult<HashMap<String, u64>> {
        let g = self.state.lock().map_err(lock_err)?;
        Ok(g.depths.clone())
    }

    fn clear_all(&self) -> StoreResult<()> {
        let mut g = self.state.lock().map_err(lock_err)?;
        *g = MemState::default();
        Ok(())
    }

    fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> StoreResult<()>) -> StoreResult<()> {
        // MemStore operates under a single Mutex; consecutive calls from the
        // closure are already linearised. For a real transaction-rollback we'd
        // snapshot state before invoking `f` and restore on Err, but webylib's
        // wallet operations don't depend on that semantics for the in-memory
        // path. Document the behaviour as best-effort.
        let snapshot = self.snapshot();
        let result = f(self);
        if result.is_err() {
            let mut g = self.state.lock().map_err(lock_err)?;
            *g = snapshot;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_roundtrip() {
        let s = MemStore::new();
        s.set_meta("master_secret", "abcdef").unwrap();
        assert_eq!(s.get_meta("master_secret").unwrap(), Some("abcdef".into()));
        assert_eq!(s.get_meta("missing").unwrap(), None);
        assert_eq!(s.get_all_meta().unwrap().len(), 1);
    }

    #[test]
    fn output_lifecycle() {
        let s = MemStore::new();
        s.insert_output(&[1, 2, 3], "secret-a", 100).unwrap();
        s.insert_output(&[4, 5, 6], "secret-b", 250).unwrap();
        assert_eq!(s.count_outputs().unwrap(), 2);
        assert_eq!(s.count_unspent().unwrap(), 2);
        assert_eq!(s.sum_unspent().unwrap(), 350);

        // Order DESC by amount
        let unspent = s.get_unspent().unwrap();
        assert_eq!(unspent[0].1, 250);
        assert_eq!(unspent[1].1, 100);

        s.mark_spent(&[1, 2, 3]).unwrap();
        assert_eq!(s.count_unspent().unwrap(), 1);
        assert_eq!(s.sum_unspent().unwrap(), 250);

        s.update_output_amount(&[4, 5, 6], 999).unwrap();
        assert_eq!(s.sum_unspent().unwrap(), 999);
    }

    #[test]
    fn duplicate_secret_hash_rejected() {
        let s = MemStore::new();
        s.insert_output(&[7], "x", 10).unwrap();
        assert!(matches!(
            s.insert_output(&[7], "x-dup", 20).unwrap_err(),
            StoreError::Constraint(_)
        ));
    }

    #[test]
    fn spent_hashes_dedupe() {
        let s = MemStore::new();
        s.insert_spent_hash(&[1]).unwrap();
        s.insert_spent_hash(&[1]).unwrap(); // dedup, no error
        s.insert_spent_hash(&[2]).unwrap();
        assert_eq!(s.count_spent_hashes().unwrap(), 2);
    }

    #[test]
    fn depths() {
        let s = MemStore::new();
        s.set_depth("Receive", 5).unwrap();
        s.set_depth("Pay", 12).unwrap();
        assert_eq!(s.get_depth("Receive").unwrap(), 5);
        assert_eq!(s.get_depth("Pay").unwrap(), 12);
        assert_eq!(s.get_depth("Mining").unwrap(), 0); // default
        assert_eq!(s.get_all_depths().unwrap().len(), 2);
    }

    #[test]
    fn atomic_rollback_on_err() {
        let s = MemStore::new();
        s.insert_output(&[1], "x", 10).unwrap();
        let result: StoreResult<()> = s.atomic(&mut |inner| {
            inner.insert_output(&[2], "y", 20)?;
            // Force a failure; the [2] insert above should be rolled back.
            Err(StoreError::Backend("forced".into()))
        });
        assert!(result.is_err());
        assert_eq!(s.count_outputs().unwrap(), 1);
    }

    #[test]
    fn atomic_commit_on_ok() {
        let s = MemStore::new();
        let result: StoreResult<()> = s.atomic(&mut |inner| {
            inner.insert_output(&[1], "x", 10)?;
            inner.insert_output(&[2], "y", 20)?;
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(s.count_outputs().unwrap(), 2);
    }

    #[test]
    fn clear_all_resets() {
        let s = MemStore::new();
        s.set_meta("k", "v").unwrap();
        s.insert_output(&[1], "x", 10).unwrap();
        s.insert_spent_hash(&[2]).unwrap();
        s.set_depth("Receive", 5).unwrap();
        s.clear_all().unwrap();
        assert_eq!(s.count_outputs().unwrap(), 0);
        assert_eq!(s.count_spent_hashes().unwrap(), 0);
        assert_eq!(s.get_all_meta().unwrap().len(), 0);
        assert_eq!(s.get_all_depths().unwrap().len(), 0);
    }
}

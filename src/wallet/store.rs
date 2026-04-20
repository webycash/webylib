//! Storage abstraction for the wallet engine.
//!
//! `Store` defines the minimal interface the wallet needs from its backend.
//! - **Native**: `SqliteStore` wraps `rusqlite::Connection` (existing behavior).
//! - **WASM**: `MemStore` uses in-memory `Vec`/`HashMap`, serializable to JSON.
//!
//! The wallet operations in `operations.rs` and `snapshot.rs` call `Store`
//! methods instead of raw SQL. Business logic is unchanged.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Store trait ──────────────────────────────────────────────────

/// Minimal storage interface for the wallet engine.
pub trait Store {
    fn as_any(&self) -> &dyn std::any::Any;
    fn get_meta(&self, key: &str) -> Result<Option<String>>;
    fn set_meta(&self, key: &str, value: &str) -> Result<()>;

    fn insert_output(&self, secret_hash: &[u8], secret: &str, amount: i64) -> Result<()>;
    fn mark_spent(&self, secret_hash: &[u8]) -> Result<()>;
    fn insert_spent_hash(&self, hash: &[u8]) -> Result<()>;
    fn get_unspent(&self) -> Result<Vec<(String, i64)>>; // (secret, amount)
    fn get_all_outputs(&self) -> Result<Vec<(String, i64, String, i32)>>; // (secret, amount, created_at, spent)
    fn count_outputs(&self) -> Result<u64>;
    fn count_unspent(&self) -> Result<u64>;
    fn count_spent_hashes(&self) -> Result<u64>;
    fn sum_unspent(&self) -> Result<i64>;
    fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> Result<()>;

    fn get_depth(&self, chain: &str) -> Result<u64>;
    fn set_depth(&self, chain: &str, depth: u64) -> Result<()>;
    fn get_all_depths(&self) -> Result<HashMap<String, u64>>;

    fn get_all_meta(&self) -> Result<HashMap<String, String>>;
    fn get_spent_hashes_with_time(&self) -> Result<Vec<(Vec<u8>, String)>>;
    fn get_unspent_full(&self) -> Result<Vec<(String, i64, String)>>; // (secret, amount, created_at)

    /// Clear all data (for import)
    fn clear_all(&self) -> Result<()>;

    /// Run a batch of operations atomically.
    /// Default: no transaction (MemStore is single-threaded, always consistent).
    /// SqliteStore overrides with a real SQLite transaction.
    fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> Result<()>) -> Result<()>;
}

// ── SQLite implementation ────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod sqlite {
    use super::*;
    use crate::error::{Error, Result};
    use rusqlite::{params, Connection, OptionalExtension};
    use std::sync::Mutex;

    pub struct SqliteStore(pub Mutex<Connection>);

    impl SqliteStore {
        pub fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
            self.0
                .lock()
                .map_err(|_| Error::wallet("Failed to acquire database lock"))
        }
    }

    impl Store for SqliteStore {
        fn as_any(&self) -> &dyn std::any::Any { self }

        fn get_meta(&self, key: &str) -> Result<Option<String>> {
            let conn = self.connection()?;
            conn.query_row(
                "SELECT value FROM wallet_metadata WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::Database(e).with_context("get_meta"))
        }

        fn set_meta(&self, key: &str, value: &str) -> Result<()> {
            let conn = self.connection()?;
            conn.execute(
                "INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
            Ok(())
        }

        fn insert_output(&self, secret_hash: &[u8], secret: &str, amount: i64) -> Result<()> {
            let conn = self.connection()?;
            conn.execute(
                "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
                params![secret_hash, secret, amount],
            )?;
            Ok(())
        }

        fn mark_spent(&self, secret_hash: &[u8]) -> Result<()> {
            let conn = self.connection()?;
            conn.execute(
                "UPDATE unspent_outputs SET spent = 1 WHERE secret_hash = ?1",
                params![secret_hash],
            )?;
            Ok(())
        }

        fn insert_spent_hash(&self, hash: &[u8]) -> Result<()> {
            let conn = self.connection()?;
            conn.execute(
                "INSERT OR IGNORE INTO spent_hashes (hash) VALUES (?1)",
                params![hash],
            )?;
            Ok(())
        }

        fn get_unspent(&self) -> Result<Vec<(String, i64)>> {
            let conn = self.connection()?;
            let mut stmt = conn.prepare(
                "SELECT secret, amount FROM unspent_outputs WHERE spent = 0 ORDER BY amount DESC",
            )?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }

        fn get_all_outputs(&self) -> Result<Vec<(String, i64, String, i32)>> {
            let conn = self.connection()?;
            let mut stmt = conn.prepare(
                "SELECT secret, amount, created_at, spent FROM unspent_outputs ORDER BY id",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i32>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }

        fn count_outputs(&self) -> Result<u64> {
            let conn = self.connection()?;
            let n: i64 =
                conn.query_row("SELECT COUNT(*) FROM unspent_outputs", [], |r| r.get(0))?;
            Ok(n as u64)
        }

        fn count_unspent(&self) -> Result<u64> {
            let conn = self.connection()?;
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM unspent_outputs WHERE spent = 0",
                [],
                |r| r.get(0),
            )?;
            Ok(n as u64)
        }

        fn count_spent_hashes(&self) -> Result<u64> {
            let conn = self.connection()?;
            let n: i64 =
                conn.query_row("SELECT COUNT(*) FROM spent_hashes", [], |r| r.get(0))?;
            Ok(n as u64)
        }

        fn sum_unspent(&self) -> Result<i64> {
            let conn = self.connection()?;
            let total: Option<i64> = conn
                .query_row(
                    "SELECT COALESCE(SUM(amount), 0) FROM unspent_outputs WHERE spent = 0",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(total.unwrap_or(0))
        }

        fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> Result<()> {
            let conn = self.connection()?;
            conn.execute(
                "UPDATE unspent_outputs SET amount = ?1 WHERE secret_hash = ?2 AND spent = 0",
                params![new_amount, secret_hash],
            )?;
            Ok(())
        }

        fn get_depth(&self, chain: &str) -> Result<u64> {
            let conn = self.connection()?;
            let d: i64 = conn
                .query_row(
                    "SELECT depth FROM walletdepths WHERE chain_code = ?1",
                    params![chain],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(0);
            Ok(d as u64)
        }

        fn set_depth(&self, chain: &str, depth: u64) -> Result<()> {
            let conn = self.connection()?;
            conn.execute(
                "INSERT INTO walletdepths (chain_code, depth) VALUES (?1, ?2)
                 ON CONFLICT(chain_code) DO UPDATE SET depth = excluded.depth",
                params![chain, depth as i64],
            )?;
            Ok(())
        }

        fn get_all_depths(&self) -> Result<HashMap<String, u64>> {
            let conn = self.connection()?;
            let mut stmt = conn.prepare("SELECT chain_code, depth FROM walletdepths")?;
            let map = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
                })?
                .collect::<std::result::Result<HashMap<_, _>, _>>()?;
            Ok(map)
        }

        fn get_all_meta(&self) -> Result<HashMap<String, String>> {
            let conn = self.connection()?;
            let mut stmt =
                conn.prepare("SELECT key, value FROM wallet_metadata ORDER BY key")?;
            let map = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<std::result::Result<HashMap<_, _>, _>>()?;
            Ok(map)
        }

        fn get_spent_hashes_with_time(&self) -> Result<Vec<(Vec<u8>, String)>> {
            let conn = self.connection()?;
            let mut stmt =
                conn.prepare("SELECT hash, spent_at FROM spent_hashes ORDER BY id")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }

        fn get_unspent_full(&self) -> Result<Vec<(String, i64, String)>> {
            let conn = self.connection()?;
            let mut stmt = conn.prepare(
                "SELECT secret, amount, created_at FROM unspent_outputs WHERE spent = 0",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }

        fn clear_all(&self) -> Result<()> {
            let conn = self.connection()?;
            conn.execute("DELETE FROM wallet_metadata", [])?;
            conn.execute("DELETE FROM unspent_outputs", [])?;
            conn.execute("DELETE FROM spent_hashes", [])?;
            Ok(())
        }

        fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> Result<()>) -> Result<()> {
            let mut conn = self.connection()?;
            let tx = conn.transaction().map_err(|e| Error::Database(e).with_context("begin transaction"))?;
            let tx_store = SqliteTxStore(&tx);
            f(&tx_store)?;
            tx.commit().map_err(|e| Error::Database(e).with_context("commit transaction"))?;
            Ok(())
        }

    }

    // ── Transaction-scoped store ────────────────────────────────

    /// Transaction-scoped store — all operations within `SqliteStore::atomic`
    /// run inside a single SQLite transaction. If the closure returns `Err`,
    /// the transaction is rolled back; on `Ok`, it is committed.
    struct SqliteTxStore<'a>(&'a rusqlite::Transaction<'a>);

    impl<'a> Store for SqliteTxStore<'a> {
        fn as_any(&self) -> &dyn std::any::Any { unimplemented!("TxStore is not downcastable") }

        fn get_meta(&self, key: &str) -> Result<Option<String>> {
            self.0
                .query_row(
                    "SELECT value FROM wallet_metadata WHERE key = ?1",
                    params![key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| Error::Database(e).with_context("get_meta"))
        }

        fn set_meta(&self, key: &str, value: &str) -> Result<()> {
            self.0.execute(
                "INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
            Ok(())
        }

        fn insert_output(&self, secret_hash: &[u8], secret: &str, amount: i64) -> Result<()> {
            self.0.execute(
                "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) VALUES (?1, ?2, ?3, 0)",
                params![secret_hash, secret, amount],
            )?;
            Ok(())
        }

        fn mark_spent(&self, secret_hash: &[u8]) -> Result<()> {
            self.0.execute(
                "UPDATE unspent_outputs SET spent = 1 WHERE secret_hash = ?1",
                params![secret_hash],
            )?;
            Ok(())
        }

        fn insert_spent_hash(&self, hash: &[u8]) -> Result<()> {
            self.0.execute(
                "INSERT OR IGNORE INTO spent_hashes (hash) VALUES (?1)",
                params![hash],
            )?;
            Ok(())
        }

        fn get_unspent(&self) -> Result<Vec<(String, i64)>> {
            let mut stmt = self.0.prepare(
                "SELECT secret, amount FROM unspent_outputs WHERE spent = 0 ORDER BY amount DESC",
            )?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }

        fn get_all_outputs(&self) -> Result<Vec<(String, i64, String, i32)>> {
            let mut stmt = self.0.prepare(
                "SELECT secret, amount, created_at, spent FROM unspent_outputs ORDER BY id",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i32>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }

        fn count_outputs(&self) -> Result<u64> {
            let n: i64 =
                self.0
                    .query_row("SELECT COUNT(*) FROM unspent_outputs", [], |r| r.get(0))?;
            Ok(n as u64)
        }

        fn count_unspent(&self) -> Result<u64> {
            let n: i64 = self.0.query_row(
                "SELECT COUNT(*) FROM unspent_outputs WHERE spent = 0",
                [],
                |r| r.get(0),
            )?;
            Ok(n as u64)
        }

        fn count_spent_hashes(&self) -> Result<u64> {
            let n: i64 =
                self.0
                    .query_row("SELECT COUNT(*) FROM spent_hashes", [], |r| r.get(0))?;
            Ok(n as u64)
        }

        fn sum_unspent(&self) -> Result<i64> {
            let total: Option<i64> = self
                .0
                .query_row(
                    "SELECT COALESCE(SUM(amount), 0) FROM unspent_outputs WHERE spent = 0",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(total.unwrap_or(0))
        }

        fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> Result<()> {
            self.0.execute(
                "UPDATE unspent_outputs SET amount = ?1 WHERE secret_hash = ?2 AND spent = 0",
                params![new_amount, secret_hash],
            )?;
            Ok(())
        }

        fn get_depth(&self, chain: &str) -> Result<u64> {
            let d: i64 = self
                .0
                .query_row(
                    "SELECT depth FROM walletdepths WHERE chain_code = ?1",
                    params![chain],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(0);
            Ok(d as u64)
        }

        fn set_depth(&self, chain: &str, depth: u64) -> Result<()> {
            self.0.execute(
                "INSERT INTO walletdepths (chain_code, depth) VALUES (?1, ?2)
                 ON CONFLICT(chain_code) DO UPDATE SET depth = excluded.depth",
                params![chain, depth as i64],
            )?;
            Ok(())
        }

        fn get_all_depths(&self) -> Result<HashMap<String, u64>> {
            let mut stmt = self.0.prepare("SELECT chain_code, depth FROM walletdepths")?;
            let map = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
                })?
                .collect::<std::result::Result<HashMap<_, _>, _>>()?;
            Ok(map)
        }

        fn get_all_meta(&self) -> Result<HashMap<String, String>> {
            let mut stmt =
                self.0
                    .prepare("SELECT key, value FROM wallet_metadata ORDER BY key")?;
            let map = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<std::result::Result<HashMap<_, _>, _>>()?;
            Ok(map)
        }

        fn get_spent_hashes_with_time(&self) -> Result<Vec<(Vec<u8>, String)>> {
            let mut stmt =
                self.0
                    .prepare("SELECT hash, spent_at FROM spent_hashes ORDER BY id")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }

        fn get_unspent_full(&self) -> Result<Vec<(String, i64, String)>> {
            let mut stmt = self.0.prepare(
                "SELECT secret, amount, created_at FROM unspent_outputs WHERE spent = 0",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        }

        fn clear_all(&self) -> Result<()> {
            self.0.execute("DELETE FROM wallet_metadata", [])?;
            self.0.execute("DELETE FROM unspent_outputs", [])?;
            self.0.execute("DELETE FROM spent_hashes", [])?;
            Ok(())
        }

        fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> Result<()>) -> Result<()> {
            // Already inside a transaction — run directly.
            f(self)
        }
    }

}

// ── Shared JSON-serializable state ──────────────────────────────

/// Wallet state as plain data — used by MemStore (WASM) and JsonStore (native).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct MemState {
    pub meta: HashMap<String, String>,
    pub outputs: Vec<MemOutput>,
    pub spent_hashes: Vec<(Vec<u8>, String)>,
    pub depths: HashMap<String, u64>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MemOutput {
    pub secret_hash: Vec<u8>,
    pub secret: String,
    pub amount: i64,
    pub created_at: String,
    pub spent: bool,
}

fn default_depths() -> HashMap<String, u64> {
    let mut depths = HashMap::new();
    for chain in &["RECEIVE", "PAY", "CHANGE", "MINING"] {
        depths.insert(chain.to_string(), 0);
    }
    depths
}

fn new_mem_state() -> MemState {
    MemState {
        meta: HashMap::new(),
        outputs: Vec::new(),
        spent_hashes: Vec::new(),
        depths: default_depths(),
    }
}

// ── In-memory implementation (for WASM) ──────────────────────────

#[cfg(target_arch = "wasm32")]
pub(crate) mod mem {
    use super::*;
    use crate::error::{Error, Result};
    use std::cell::RefCell;

    pub struct MemStore(pub RefCell<MemState>);

    impl MemStore {
        pub fn new() -> Self {
            Self(RefCell::new(new_mem_state()))
        }

        pub fn from_json(json: &str) -> Result<Self> {
            let state: MemState =
                serde_json::from_str(json).map_err(|e| Error::wallet(e.to_string()))?;
            Ok(Self(RefCell::new(state)))
        }

        pub fn to_json(&self) -> Result<String> {
            let state = self.0.borrow();
            serde_json::to_string(&*state).map_err(|e| Error::wallet(e.to_string()))
        }
    }

    impl Store for MemStore {
        fn as_any(&self) -> &dyn std::any::Any { self }

        fn get_meta(&self, key: &str) -> Result<Option<String>> {
            Ok(self.0.borrow().meta.get(key).cloned())
        }

        fn set_meta(&self, key: &str, value: &str) -> Result<()> {
            self.0
                .borrow_mut()
                .meta
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn insert_output(&self, secret_hash: &[u8], secret: &str, amount: i64) -> Result<()> {
            let mut state = self.0.borrow_mut();
            if state.outputs.iter().any(|o| o.secret_hash == secret_hash) {
                return Err(Error::wallet("UNIQUE constraint: output already exists"));
            }
            state.outputs.push(MemOutput {
                secret_hash: secret_hash.to_vec(),
                secret: secret.to_string(),
                amount,
                created_at: String::new(),
                spent: false,
            });
            Ok(())
        }

        fn mark_spent(&self, secret_hash: &[u8]) -> Result<()> {
            let mut state = self.0.borrow_mut();
            if let Some(out) = state
                .outputs
                .iter_mut()
                .find(|o| o.secret_hash == secret_hash)
            {
                out.spent = true;
            }
            Ok(())
        }

        fn insert_spent_hash(&self, hash: &[u8]) -> Result<()> {
            let mut state = self.0.borrow_mut();
            if !state.spent_hashes.iter().any(|(h, _)| h == hash) {
                state
                    .spent_hashes
                    .push((hash.to_vec(), String::new()));
            }
            Ok(())
        }

        fn get_unspent(&self) -> Result<Vec<(String, i64)>> {
            let state = self.0.borrow();
            let mut v: Vec<_> = state
                .outputs
                .iter()
                .filter(|o| !o.spent)
                .map(|o| (o.secret.clone(), o.amount))
                .collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            Ok(v)
        }

        fn get_all_outputs(&self) -> Result<Vec<(String, i64, String, i32)>> {
            let state = self.0.borrow();
            Ok(state
                .outputs
                .iter()
                .map(|o| {
                    (
                        o.secret.clone(),
                        o.amount,
                        o.created_at.clone(),
                        if o.spent { 1 } else { 0 },
                    )
                })
                .collect())
        }

        fn count_outputs(&self) -> Result<u64> {
            Ok(self.0.borrow().outputs.len() as u64)
        }

        fn count_unspent(&self) -> Result<u64> {
            Ok(self.0.borrow().outputs.iter().filter(|o| !o.spent).count() as u64)
        }

        fn count_spent_hashes(&self) -> Result<u64> {
            Ok(self.0.borrow().spent_hashes.len() as u64)
        }

        fn sum_unspent(&self) -> Result<i64> {
            Ok(self
                .0
                .borrow()
                .outputs
                .iter()
                .filter(|o| !o.spent)
                .map(|o| o.amount)
                .sum())
        }

        fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> Result<()> {
            let mut state = self.0.borrow_mut();
            if let Some(out) = state
                .outputs
                .iter_mut()
                .find(|o| o.secret_hash == secret_hash && !o.spent)
            {
                out.amount = new_amount;
            }
            Ok(())
        }

        fn get_depth(&self, chain: &str) -> Result<u64> {
            Ok(self.0.borrow().depths.get(chain).copied().unwrap_or(0))
        }

        fn set_depth(&self, chain: &str, depth: u64) -> Result<()> {
            self.0
                .borrow_mut()
                .depths
                .insert(chain.to_string(), depth);
            Ok(())
        }

        fn get_all_depths(&self) -> Result<HashMap<String, u64>> {
            Ok(self.0.borrow().depths.clone())
        }

        fn get_all_meta(&self) -> Result<HashMap<String, String>> {
            Ok(self.0.borrow().meta.clone())
        }

        fn get_spent_hashes_with_time(&self) -> Result<Vec<(Vec<u8>, String)>> {
            Ok(self.0.borrow().spent_hashes.clone())
        }

        fn get_unspent_full(&self) -> Result<Vec<(String, i64, String)>> {
            let state = self.0.borrow();
            Ok(state
                .outputs
                .iter()
                .filter(|o| !o.spent)
                .map(|o| (o.secret.clone(), o.amount, o.created_at.clone()))
                .collect())
        }

        fn clear_all(&self) -> Result<()> {
            let mut state = self.0.borrow_mut();
            state.meta.clear();
            state.outputs.clear();
            state.spent_hashes.clear();
            Ok(())
        }

        fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> Result<()>) -> Result<()> {
            // Single-threaded WASM — no transaction needed, always consistent.
            f(self)
        }
    }
}

// ── JSON file store (native) ────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub mod json {
    use super::*;
    use crate::error::{Error, Result};
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// JSON-backed store for native targets.
    ///
    /// State lives in memory (behind a `Mutex`) and is auto-flushed to a JSON
    /// file on every mutation when a `path` is set. Without a path, it behaves
    /// as a pure in-memory store — use `to_json()` to retrieve the state.
    pub struct JsonStore {
        state: Mutex<MemState>,
        path: Option<PathBuf>,
    }

    impl JsonStore {
        /// Create an empty store. If `path` is `Some`, state is persisted to that file.
        pub fn new(path: Option<PathBuf>) -> Self {
            Self { state: Mutex::new(new_mem_state()), path }
        }

        /// Create from an existing JSON string.
        pub fn from_json(json: &str, path: Option<PathBuf>) -> Result<Self> {
            let state: MemState =
                serde_json::from_str(json).map_err(|e| Error::wallet(e.to_string()))?;
            Ok(Self { state: Mutex::new(state), path })
        }

        /// Open from a JSON file on disk. Creates with defaults if the file doesn't exist.
        pub fn open(path: PathBuf) -> Result<Self> {
            if path.exists() {
                let json = std::fs::read_to_string(&path)
                    .map_err(|e| Error::wallet(format!("read {}: {}", path.display(), e)))?;
                Self::from_json(&json, Some(path))
            } else {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| Error::wallet(format!("mkdir {}: {}", parent.display(), e)))?;
                }
                let store = Self::new(Some(path));
                store.flush()?;
                Ok(store)
            }
        }

        /// Serialize state to JSON.
        pub fn to_json(&self) -> Result<String> {
            let state = self.state.lock()
                .map_err(|_| Error::wallet("lock poisoned"))?;
            serde_json::to_string_pretty(&*state).map_err(|e| Error::wallet(e.to_string()))
        }

        /// Write state to disk (if a path is set).
        fn flush(&self) -> Result<()> {
            if let Some(ref path) = self.path {
                let json = self.to_json()?;
                std::fs::write(path, json.as_bytes())
                    .map_err(|e| Error::wallet(format!("write {}: {}", path.display(), e)))?;
            }
            Ok(())
        }

        fn with_state<R>(&self, f: impl FnOnce(&MemState) -> R) -> Result<R> {
            let state = self.state.lock()
                .map_err(|_| Error::wallet("lock poisoned"))?;
            Ok(f(&state))
        }

        fn mutate<R>(&self, f: impl FnOnce(&mut MemState) -> R) -> Result<R> {
            let mut state = self.state.lock()
                .map_err(|_| Error::wallet("lock poisoned"))?;
            let result = f(&mut state);
            drop(state);
            self.flush()?;
            Ok(result)
        }
    }

    impl Store for JsonStore {
        fn as_any(&self) -> &dyn std::any::Any { self }

        fn get_meta(&self, key: &str) -> Result<Option<String>> {
            self.with_state(|s| s.meta.get(key).cloned())
        }

        fn set_meta(&self, key: &str, value: &str) -> Result<()> {
            self.mutate(|s| { s.meta.insert(key.to_string(), value.to_string()); })
        }

        fn insert_output(&self, secret_hash: &[u8], secret: &str, amount: i64) -> Result<()> {
            self.mutate(|s| {
                s.outputs.push(MemOutput {
                    secret_hash: secret_hash.to_vec(),
                    secret: secret.to_string(),
                    amount,
                    created_at: String::new(),
                    spent: false,
                });
            })
        }

        fn mark_spent(&self, secret_hash: &[u8]) -> Result<()> {
            self.mutate(|s| {
                if let Some(o) = s.outputs.iter_mut().find(|o| o.secret_hash == secret_hash) {
                    o.spent = true;
                }
            })
        }

        fn insert_spent_hash(&self, hash: &[u8]) -> Result<()> {
            self.mutate(|s| {
                if !s.spent_hashes.iter().any(|(h, _)| h == hash) {
                    s.spent_hashes.push((hash.to_vec(), String::new()));
                }
            })
        }

        fn get_unspent(&self) -> Result<Vec<(String, i64)>> {
            self.with_state(|s| {
                let mut v: Vec<_> = s.outputs.iter()
                    .filter(|o| !o.spent)
                    .map(|o| (o.secret.clone(), o.amount))
                    .collect();
                v.sort_by(|a, b| b.1.cmp(&a.1));
                v
            })
        }

        fn get_all_outputs(&self) -> Result<Vec<(String, i64, String, i32)>> {
            self.with_state(|s| {
                s.outputs.iter()
                    .map(|o| (o.secret.clone(), o.amount, o.created_at.clone(), if o.spent { 1 } else { 0 }))
                    .collect()
            })
        }

        fn count_outputs(&self) -> Result<u64> {
            self.with_state(|s| s.outputs.len() as u64)
        }

        fn count_unspent(&self) -> Result<u64> {
            self.with_state(|s| s.outputs.iter().filter(|o| !o.spent).count() as u64)
        }

        fn count_spent_hashes(&self) -> Result<u64> {
            self.with_state(|s| s.spent_hashes.len() as u64)
        }

        fn sum_unspent(&self) -> Result<i64> {
            self.with_state(|s| s.outputs.iter().filter(|o| !o.spent).map(|o| o.amount).sum())
        }

        fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> Result<()> {
            self.mutate(|s| {
                if let Some(o) = s.outputs.iter_mut().find(|o| o.secret_hash == secret_hash && !o.spent) {
                    o.amount = new_amount;
                }
            })
        }

        fn get_depth(&self, chain: &str) -> Result<u64> {
            self.with_state(|s| s.depths.get(chain).copied().unwrap_or(0))
        }

        fn set_depth(&self, chain: &str, depth: u64) -> Result<()> {
            self.mutate(|s| { s.depths.insert(chain.to_string(), depth); })
        }

        fn get_all_depths(&self) -> Result<HashMap<String, u64>> {
            self.with_state(|s| s.depths.clone())
        }

        fn get_all_meta(&self) -> Result<HashMap<String, String>> {
            self.with_state(|s| s.meta.clone())
        }

        fn get_spent_hashes_with_time(&self) -> Result<Vec<(Vec<u8>, String)>> {
            self.with_state(|s| s.spent_hashes.clone())
        }

        fn get_unspent_full(&self) -> Result<Vec<(String, i64, String)>> {
            self.with_state(|s| {
                s.outputs.iter()
                    .filter(|o| !o.spent)
                    .map(|o| (o.secret.clone(), o.amount, o.created_at.clone()))
                    .collect()
            })
        }

        fn clear_all(&self) -> Result<()> {
            self.mutate(|s| {
                s.meta.clear();
                s.outputs.clear();
                s.spent_hashes.clear();
            })
        }

        fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> Result<()>) -> Result<()> {
            // Hold the lock for the entire batch, flush once at the end.
            let mut state = self.state.lock()
                .map_err(|_| Error::wallet("lock poisoned"))?;
            // Create a temporary in-memory store from current state for the batch
            let batch_state = std::cell::RefCell::new(state.clone());
            struct BatchStore<'a>(&'a std::cell::RefCell<MemState>);

            // Run on a snapshot — if f fails, original state is untouched
            impl<'a> Store for BatchStore<'a> {
                fn as_any(&self) -> &dyn std::any::Any { unimplemented!() }
                fn get_meta(&self, key: &str) -> Result<Option<String>> {
                    Ok(self.0.borrow().meta.get(key).cloned())
                }
                fn set_meta(&self, key: &str, value: &str) -> Result<()> {
                    self.0.borrow_mut().meta.insert(key.to_string(), value.to_string()); Ok(())
                }
                fn insert_output(&self, secret_hash: &[u8], secret: &str, amount: i64) -> Result<()> {
                    self.0.borrow_mut().outputs.push(MemOutput {
                        secret_hash: secret_hash.to_vec(), secret: secret.to_string(),
                        amount, created_at: String::new(), spent: false,
                    }); Ok(())
                }
                fn mark_spent(&self, secret_hash: &[u8]) -> Result<()> {
                    if let Some(o) = self.0.borrow_mut().outputs.iter_mut().find(|o| o.secret_hash == secret_hash) { o.spent = true; } Ok(())
                }
                fn insert_spent_hash(&self, hash: &[u8]) -> Result<()> {
                    let mut s = self.0.borrow_mut();
                    if !s.spent_hashes.iter().any(|(h,_)| h == hash) { s.spent_hashes.push((hash.to_vec(), String::new())); } Ok(())
                }
                fn get_unspent(&self) -> Result<Vec<(String, i64)>> {
                    let s = self.0.borrow(); let mut v: Vec<_> = s.outputs.iter().filter(|o| !o.spent).map(|o| (o.secret.clone(), o.amount)).collect(); v.sort_by(|a,b| b.1.cmp(&a.1)); Ok(v)
                }
                fn get_all_outputs(&self) -> Result<Vec<(String, i64, String, i32)>> {
                    Ok(self.0.borrow().outputs.iter().map(|o| (o.secret.clone(), o.amount, o.created_at.clone(), if o.spent {1} else {0})).collect())
                }
                fn count_outputs(&self) -> Result<u64> { Ok(self.0.borrow().outputs.len() as u64) }
                fn count_unspent(&self) -> Result<u64> { Ok(self.0.borrow().outputs.iter().filter(|o| !o.spent).count() as u64) }
                fn count_spent_hashes(&self) -> Result<u64> { Ok(self.0.borrow().spent_hashes.len() as u64) }
                fn sum_unspent(&self) -> Result<i64> { Ok(self.0.borrow().outputs.iter().filter(|o| !o.spent).map(|o| o.amount).sum()) }
                fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> Result<()> {
                    if let Some(o) = self.0.borrow_mut().outputs.iter_mut().find(|o| o.secret_hash == secret_hash && !o.spent) { o.amount = new_amount; } Ok(())
                }
                fn get_depth(&self, chain: &str) -> Result<u64> { Ok(self.0.borrow().depths.get(chain).copied().unwrap_or(0)) }
                fn set_depth(&self, chain: &str, depth: u64) -> Result<()> { self.0.borrow_mut().depths.insert(chain.to_string(), depth); Ok(()) }
                fn get_all_depths(&self) -> Result<HashMap<String, u64>> { Ok(self.0.borrow().depths.clone()) }
                fn get_all_meta(&self) -> Result<HashMap<String, String>> { Ok(self.0.borrow().meta.clone()) }
                fn get_spent_hashes_with_time(&self) -> Result<Vec<(Vec<u8>, String)>> { Ok(self.0.borrow().spent_hashes.clone()) }
                fn get_unspent_full(&self) -> Result<Vec<(String, i64, String)>> {
                    Ok(self.0.borrow().outputs.iter().filter(|o| !o.spent).map(|o| (o.secret.clone(), o.amount, o.created_at.clone())).collect())
                }
                fn clear_all(&self) -> Result<()> { let mut s = self.0.borrow_mut(); s.meta.clear(); s.outputs.clear(); s.spent_hashes.clear(); Ok(()) }
                fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> Result<()>) -> Result<()> { f(self) }
            }

            let batch = BatchStore(&batch_state);
            f(&batch)?;
            // Commit: replace state with batch result
            *state = batch_state.into_inner();
            drop(state);
            self.flush()?;
            Ok(())
        }
    }
}

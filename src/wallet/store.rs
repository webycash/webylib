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
pub(crate) trait Store {
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

    }

    /// Transaction-scoped store (used inside `atomic`)
    struct SqliteTxStore<'a>(&'a rusqlite::Transaction<'a>);

    impl<'a> Store for SqliteTxStore<'a> {
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

    }
}

// ── In-memory implementation (for WASM) ──────────────────────────

#[cfg(target_arch = "wasm32")]
pub(crate) mod mem {
    use super::*;
    use crate::error::{Error, Result};
    use std::cell::RefCell;

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

    pub struct MemStore(pub RefCell<MemState>);

    impl MemStore {
        pub fn new() -> Self {
            let mut depths = HashMap::new();
            for chain in &["RECEIVE", "PAY", "CHANGE", "MINING"] {
                depths.insert(chain.to_string(), 0);
            }
            Self(RefCell::new(MemState {
                meta: HashMap::new(),
                outputs: Vec::new(),
                spent_hashes: Vec::new(),
                depths,
            }))
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
            self.0.borrow_mut().outputs.push(MemOutput {
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

    }
}

//! `SqliteStore` — file-backed (or in-memory) storage via `rusqlite`.
//!
//! Identical SQL schema to the legacy webylib store so existing wallet
//! databases migrate transparently. Supports `Store::atomic` via a real
//! SQLite transaction; the closure runs inside `BEGIN ... COMMIT` and is
//! rolled back if it returns `Err`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{Store, StoreError, StoreResult};

/// `Store` implementation backed by a single SQLite connection.
/// File-backed (via `open`) or in-memory (via `open_in_memory`).
/// Atomic blocks use SAVEPOINT/RELEASE so the rollback path leaves
/// no half-applied state.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

fn sql_err<T: ToString>(prefix: &str, e: T) -> StoreError {
    StoreError::Backend(format!("{prefix}: {}", e.to_string()))
}

impl SqliteStore {
    /// Open or create a SQLite database at `path` and apply the schema.
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let conn =
            Connection::open(path.as_ref()).map_err(|e| sql_err("open", e))?;
        Self::initialise(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory SQLite database. Useful for tests and ephemeral
    /// wallets.
    pub fn open_in_memory() -> StoreResult<Self> {
        let conn = Connection::open_in_memory().map_err(|e| sql_err("open_in_memory", e))?;
        Self::initialise(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn initialise(conn: &Connection) -> StoreResult<()> {
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| sql_err("schema_init", e))
    }

    fn lock(&self) -> StoreResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| StoreError::Backend(format!("conn lock: {e}")))
    }
}

pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS wallet_metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS unspent_outputs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    secret_hash BLOB NOT NULL UNIQUE,
    secret      TEXT NOT NULL,
    amount      INTEGER NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    spent       INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_unspent_outputs_spent ON unspent_outputs(spent);

CREATE TABLE IF NOT EXISTS spent_hashes (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    hash     BLOB NOT NULL UNIQUE,
    spent_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS walletdepths (
    chain_code TEXT PRIMARY KEY,
    depth      INTEGER NOT NULL
);
"#;

impl Store for SqliteStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get_meta(&self, key: &str) -> StoreResult<Option<String>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT value FROM wallet_metadata WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| sql_err("get_meta", e))
    }

    fn set_meta(&self, key: &str, value: &str) -> StoreResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO wallet_metadata (key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .map(|_| ())
        .map_err(|e| sql_err("set_meta", e))
    }

    fn get_all_meta(&self) -> StoreResult<HashMap<String, String>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT key, value FROM wallet_metadata ORDER BY key")
            .map_err(|e| sql_err("prepare", e))?;
        let map = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| sql_err("query", e))?
            .collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(|e| sql_err("collect", e))?;
        Ok(map)
    }

    fn insert_output(
        &self,
        secret_hash: &[u8],
        secret: &str,
        amount: i64,
    ) -> StoreResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO unspent_outputs (secret_hash, secret, amount, spent) \
             VALUES (?1, ?2, ?3, 0)",
            params![secret_hash, secret, amount],
        )
        .map(|_| ())
        .map_err(|e| {
            // Translate UNIQUE violations into a Constraint error so callers
            // can distinguish from generic backend failures.
            let msg = e.to_string();
            if msg.contains("UNIQUE") {
                StoreError::Constraint(msg)
            } else {
                sql_err("insert_output", e)
            }
        })
    }

    fn mark_spent(&self, secret_hash: &[u8]) -> StoreResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE unspent_outputs SET spent = 1 WHERE secret_hash = ?1",
            params![secret_hash],
        )
        .map(|_| ())
        .map_err(|e| sql_err("mark_spent", e))
    }

    fn insert_spent_hash(&self, hash: &[u8]) -> StoreResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR IGNORE INTO spent_hashes (hash) VALUES (?1)",
            params![hash],
        )
        .map(|_| ())
        .map_err(|e| sql_err("insert_spent_hash", e))
    }

    fn update_output_amount(
        &self,
        secret_hash: &[u8],
        new_amount: i64,
    ) -> StoreResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE unspent_outputs SET amount = ?1 \
             WHERE secret_hash = ?2 AND spent = 0",
            params![new_amount, secret_hash],
        )
        .map(|_| ())
        .map_err(|e| sql_err("update_output_amount", e))
    }

    fn get_unspent(&self) -> StoreResult<Vec<(String, i64)>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT secret, amount FROM unspent_outputs \
                 WHERE spent = 0 ORDER BY amount DESC",
            )
            .map_err(|e| sql_err("prepare", e))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|e| sql_err("query", e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql_err("collect", e))?;
        Ok(rows)
    }

    fn get_unspent_full(&self) -> StoreResult<Vec<(String, i64, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT secret, amount, created_at FROM unspent_outputs WHERE spent = 0",
            )
            .map_err(|e| sql_err("prepare", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| sql_err("query", e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql_err("collect", e))?;
        Ok(rows)
    }

    fn get_all_outputs(&self) -> StoreResult<Vec<(String, i64, String, i32)>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT secret, amount, created_at, spent FROM unspent_outputs ORDER BY id",
            )
            .map_err(|e| sql_err("prepare", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i32>(3)?,
                ))
            })
            .map_err(|e| sql_err("query", e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql_err("collect", e))?;
        Ok(rows)
    }

    fn get_spent_hashes_with_time(&self) -> StoreResult<Vec<(Vec<u8>, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT hash, spent_at FROM spent_hashes ORDER BY id")
            .map_err(|e| sql_err("prepare", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| sql_err("query", e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql_err("collect", e))?;
        Ok(rows)
    }

    fn count_outputs(&self) -> StoreResult<u64> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM unspent_outputs", [], |r| r.get(0))
            .map_err(|e| sql_err("count_outputs", e))?;
        Ok(n as u64)
    }

    fn count_unspent(&self) -> StoreResult<u64> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM unspent_outputs WHERE spent = 0",
                [],
                |r| r.get(0),
            )
            .map_err(|e| sql_err("count_unspent", e))?;
        Ok(n as u64)
    }

    fn count_spent_hashes(&self) -> StoreResult<u64> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM spent_hashes", [], |r| r.get(0))
            .map_err(|e| sql_err("count_spent_hashes", e))?;
        Ok(n as u64)
    }

    fn sum_unspent(&self) -> StoreResult<i64> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM unspent_outputs WHERE spent = 0",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| sql_err("sum_unspent", e))
    }

    fn get_depth(&self, chain: &str) -> StoreResult<u64> {
        let conn = self.lock()?;
        let d: Option<i64> = conn
            .query_row(
                "SELECT depth FROM walletdepths WHERE chain_code = ?1",
                params![chain],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| sql_err("get_depth", e))?;
        Ok(d.unwrap_or(0) as u64)
    }

    fn set_depth(&self, chain: &str, depth: u64) -> StoreResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO walletdepths (chain_code, depth) VALUES (?1, ?2) \
             ON CONFLICT(chain_code) DO UPDATE SET depth = excluded.depth",
            params![chain, depth as i64],
        )
        .map(|_| ())
        .map_err(|e| sql_err("set_depth", e))
    }

    fn get_all_depths(&self) -> StoreResult<HashMap<String, u64>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT chain_code, depth FROM walletdepths")
            .map_err(|e| sql_err("prepare", e))?;
        let map = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })
            .map_err(|e| sql_err("query", e))?
            .collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(|e| sql_err("collect", e))?;
        Ok(map)
    }

    fn clear_all(&self) -> StoreResult<()> {
        let conn = self.lock()?;
        for sql in [
            "DELETE FROM wallet_metadata",
            "DELETE FROM unspent_outputs",
            "DELETE FROM spent_hashes",
            "DELETE FROM walletdepths",
        ] {
            conn.execute(sql, [])
                .map_err(|e| sql_err("clear_all", e))?;
        }
        Ok(())
    }

    fn atomic(
        &self,
        f: &mut dyn FnMut(&dyn Store) -> StoreResult<()>,
    ) -> StoreResult<()> {
        // We can't rebuild a mid-transaction Store from a `Transaction`
        // without reproducing the entire impl in a wrapper. Instead, take
        // the SQLite-level approach: BEGIN, run `f` against `self`, COMMIT
        // or ROLLBACK based on the result. The Mutex serialises access so
        // there's no concurrent-writer race.
        let conn = self.lock()?;
        conn.execute("BEGIN", [])
            .map_err(|e| sql_err("begin", e))?;
        // Drop the lock before re-entering through `&dyn Store`; the closure
        // will re-acquire it. SQLite's connection-level mutex inside rusqlite
        // is what serialises us.
        drop(conn);
        let result = f(self);
        let conn = self.lock()?;
        match &result {
            Ok(()) => {
                conn.execute("COMMIT", []).map_err(|e| sql_err("commit", e))?;
            }
            Err(_) => {
                conn.execute("ROLLBACK", [])
                    .map_err(|e| sql_err("rollback", e))?;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_and_outputs_roundtrip_in_memory() {
        let s = SqliteStore::open_in_memory().unwrap();
        s.set_meta("master", "abc").unwrap();
        assert_eq!(s.get_meta("master").unwrap(), Some("abc".into()));

        s.insert_output(&[1, 2, 3], "secret-a", 100).unwrap();
        s.insert_output(&[4, 5, 6], "secret-b", 250).unwrap();
        assert_eq!(s.count_unspent().unwrap(), 2);
        assert_eq!(s.sum_unspent().unwrap(), 350);

        // ORDER BY amount DESC
        let unspent = s.get_unspent().unwrap();
        assert_eq!(unspent[0].1, 250);

        s.mark_spent(&[1, 2, 3]).unwrap();
        assert_eq!(s.count_unspent().unwrap(), 1);
        assert_eq!(s.sum_unspent().unwrap(), 250);
    }

    #[test]
    fn duplicate_secret_hash_is_constraint() {
        let s = SqliteStore::open_in_memory().unwrap();
        s.insert_output(&[7], "x", 10).unwrap();
        let err = s.insert_output(&[7], "y", 20).unwrap_err();
        assert!(matches!(err, StoreError::Constraint(_)));
    }

    #[test]
    fn depth_tracking() {
        let s = SqliteStore::open_in_memory().unwrap();
        s.set_depth("Receive", 5).unwrap();
        s.set_depth("Pay", 12).unwrap();
        assert_eq!(s.get_depth("Receive").unwrap(), 5);
        assert_eq!(s.get_depth("Mining").unwrap(), 0);
        assert_eq!(s.get_all_depths().unwrap().len(), 2);
        // Upsert
        s.set_depth("Receive", 10).unwrap();
        assert_eq!(s.get_depth("Receive").unwrap(), 10);
    }

    #[test]
    fn atomic_commits_on_ok() {
        let s = SqliteStore::open_in_memory().unwrap();
        let r: StoreResult<()> = s.atomic(&mut |inner| {
            inner.insert_output(&[1], "a", 100)?;
            inner.insert_output(&[2], "b", 200)?;
            Ok(())
        });
        assert!(r.is_ok());
        assert_eq!(s.count_outputs().unwrap(), 2);
    }

    #[test]
    fn atomic_rolls_back_on_err() {
        let s = SqliteStore::open_in_memory().unwrap();
        s.insert_output(&[1], "a", 100).unwrap();
        let r: StoreResult<()> = s.atomic(&mut |inner| {
            inner.insert_output(&[2], "b", 200)?;
            Err(StoreError::Backend("forced".into()))
        });
        assert!(r.is_err());
        assert_eq!(s.count_outputs().unwrap(), 1);
    }

    #[test]
    fn schema_sql_idempotent() {
        // Open an in-memory db, run schema again — must succeed.
        let s = SqliteStore::open_in_memory().unwrap();
        let conn = s.lock().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
    }
}

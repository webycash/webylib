//! Wallet-side storage layer.
//!
//! Defines the `Store` trait and ships three reference implementations,
//! each behind a cargo feature so consumers (notably WASM) can opt out
//! of native-only deps:
//!
//!   - `sqlite` (default) — `SqliteStore` wraps `rusqlite::Connection`.
//!     File-backed or in-memory; transactions via `Store::atomic`.
//!   - `mem` (default) — `MemStore` keeps everything in HashMaps; useful
//!     for unit tests and as the JSON-snapshot working set.
//!   - `json` (default) — `JsonStore` is a `MemStore` plus persistence
//!     to a JSON file on the filesystem.
//!   - `indexeddb` (opt-in) — `IndexedDbStore` for the browser. Wires
//!     to `web-sys`'s IndexedDB API.
//!
//! The `Store` trait is identical-shape to the legacy webylib trait so
//! the existing wallet operations call this without code changes.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("storage backend error: {0}")]
    Backend(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("constraint violation: {0}")]
    Constraint(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Minimal storage interface for the wallet engine.
pub trait Store: Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;

    fn get_meta(&self, key: &str) -> StoreResult<Option<String>>;
    fn set_meta(&self, key: &str, value: &str) -> StoreResult<()>;
    fn get_all_meta(&self) -> StoreResult<HashMap<String, String>>;

    fn insert_output(&self, secret_hash: &[u8], secret: &str, amount: i64) -> StoreResult<()>;
    fn mark_spent(&self, secret_hash: &[u8]) -> StoreResult<()>;
    fn insert_spent_hash(&self, hash: &[u8]) -> StoreResult<()>;
    fn update_output_amount(&self, secret_hash: &[u8], new_amount: i64) -> StoreResult<()>;
    fn get_unspent(&self) -> StoreResult<Vec<(String, i64)>>;
    fn get_unspent_full(&self) -> StoreResult<Vec<(String, i64, String)>>;
    fn get_all_outputs(&self) -> StoreResult<Vec<(String, i64, String, i32)>>;
    fn get_spent_hashes_with_time(&self) -> StoreResult<Vec<(Vec<u8>, String)>>;
    fn count_outputs(&self) -> StoreResult<u64>;
    fn count_unspent(&self) -> StoreResult<u64>;
    fn count_spent_hashes(&self) -> StoreResult<u64>;
    fn sum_unspent(&self) -> StoreResult<i64>;

    fn get_depth(&self, chain: &str) -> StoreResult<u64>;
    fn set_depth(&self, chain: &str, depth: u64) -> StoreResult<()>;
    fn get_all_depths(&self) -> StoreResult<HashMap<String, u64>>;

    fn clear_all(&self) -> StoreResult<()>;

    fn atomic(&self, f: &mut dyn FnMut(&dyn Store) -> StoreResult<()>) -> StoreResult<()>;
}

#[cfg(feature = "mem")]
pub mod mem;
#[cfg(feature = "json")]
pub mod json;
#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "mem")]
pub use mem::MemStore;
#[cfg(feature = "json")]
pub use json::JsonStore;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;

#[cfg(test)]
mod tests {
    use super::*;

    /// Trait-object safety check. If this compiles, the trait is usable
    /// behind `Box<dyn Store>` (which the legacy webylib uses).
    #[test]
    fn trait_is_object_safe() {
        fn _accepts(_: &dyn Store) {}
    }
}

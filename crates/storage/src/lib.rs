//! Wallet-side Store trait + SqliteStore + MemStore + IndexedDB.
//!
//! Migrated from webylib `src/wallet/store.rs` (1134 LOC) in M2, split into
//! per-backend modules each <300 LOC. Generic over `A: Asset` from
//! `webycash-asset-core` (server-side) — the wallet imports the same trait
//! definitions to keep wire-format invariants in sync.

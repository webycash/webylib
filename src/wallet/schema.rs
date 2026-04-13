//! Database schema initialization and migrations for the wallet.

use crate::error::Result;
use rusqlite::Connection;

/// Enable WAL mode for crash-safe concurrent reads.
/// Must be called before any writes.
pub(crate) fn enable_wal_mode(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;",
    )?;
    Ok(())
}

/// Initialize the database schema (idempotent).
pub(crate) fn initialize_schema(connection: &Connection) -> Result<()> {
    enable_wal_mode(connection)?;

    connection.execute(
        "CREATE TABLE IF NOT EXISTS unspent_outputs (
            id INTEGER PRIMARY KEY,
            secret_hash BLOB UNIQUE NOT NULL,
            secret TEXT NOT NULL,
            amount INTEGER NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            spent INTEGER DEFAULT 0
        )",
        [],
    )?;

    connection.execute(
        "CREATE TABLE IF NOT EXISTS spent_hashes (
            id INTEGER PRIMARY KEY,
            hash BLOB UNIQUE NOT NULL,
            spent_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    connection.execute(
        "CREATE TABLE IF NOT EXISTS wallet_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    )?;

    connection.execute(
        "CREATE TABLE IF NOT EXISTS walletdepths (
            chain_code TEXT PRIMARY KEY,
            depth INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )?;

    // Initialize walletdepths if empty (matches Python generate_initial_walletdepths)
    connection.execute(
        "INSERT OR IGNORE INTO walletdepths (chain_code, depth) VALUES
         ('RECEIVE', 0), ('PAY', 0), ('CHANGE', 0), ('MINING', 0)",
        [],
    )?;

    // Performance indexes
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_unspent_outputs_secret_hash ON unspent_outputs(secret_hash)",
        [],
    )?;
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_unspent_outputs_spent ON unspent_outputs(spent)",
        [],
    )?;
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_spent_hashes_hash ON spent_hashes(hash)",
        [],
    )?;

    Ok(())
}

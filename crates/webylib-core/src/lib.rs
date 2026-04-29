//! Generic wallet core — `Wallet<A: Asset>`.
//!
//! Operations live in `ops/` (insert / pay / replace / recover / check / stats /
//! issue). Each op is a thin module wrapped by `#[wallet_op]` to generate
//! lock-store / run-effect / emit-event / persist plumbing.
//!
//! Snapshot, encryption, and passkey live alongside ops. Migrated from
//! `webylib/src/wallet/{mod,operations,snapshot,encryption}.rs` in M2.
#![forbid(unsafe_code)]
#![warn(missing_docs)]


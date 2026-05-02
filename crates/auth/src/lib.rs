//! Wallet-side PGP keyring + issuer fingerprint identity.
//!
//! Default crate: `rpgp` (pure-Rust). Sequoia is feature-gated for native
//! deployments that need its broader OpenPGP feature surface. WASM build
//! pins to rpgp.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

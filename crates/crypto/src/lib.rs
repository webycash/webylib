//! Crypto primitives — SHA256, Argon2, AES-256-GCM, Ed25519.
//!
//! Migrated from webylib `src/crypto.rs` in M2. Split per-primitive so unit
//! tests stay focused and the WASM build pulls only what it needs.

//! Procedural macros for webylib.
//!
//! Three macros land here:
//! - `#[wallet_op]` — generates lock/run-effect/emit-event/persist plumbing
//!   for a wallet operation. Mirrors webycash-server's `#[gen_server]`.
//! - `#[asset_storage]` — generates per-asset CRUD on the `Store` trait from
//!   a struct definition.
//! - `#[ffi_export]` — generates the `extern "C"` shim, error-code marshaling,
//!   opaque-handle lifecycle, and async-callback bridging for the FFI layer.
//!   Single source of truth replaces the existing 22K-LOC handwritten FFI.
//!
//! Implementations land alongside their consumers — `wallet_op`/`asset_storage`
//! in M2, `ffi_export` in M2 (FFI overhaul phase).

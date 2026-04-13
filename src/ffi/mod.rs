//! C FFI bindings for webylib.
//!
//! This module exposes the full wallet API through a C-compatible ABI,
//! enabling consumption from Python (ctypes/cffi), Node.js (ffi-napi),
//! .NET (P/Invoke), Go (cgo), Swift, Java (JNI), Kotlin (JNA), and C/C++.
//!
//! # Memory Ownership Rules
//!
//! - Strings returned via `out_*` pointers are heap-allocated by this library.
//!   The caller **must** free them with `weby_free_string()`.
//! - Wallet handles returned by `weby_wallet_open*` must be freed with `weby_wallet_free()`.
//! - The pointer from `weby_version()` is static — do **not** free it.
//! - The pointer from `weby_last_error_message()` is thread-local and valid until
//!   the next FFI call on the same thread — do **not** free it.
//!
//! # Error Handling
//!
//! Every function returns an `i32` error code. `0` means success.
//! On failure, call `weby_last_error_message()` for a human-readable description.
//!
//! # Building
//!
//! ```sh
//! # Shared library (.so / .dylib / .dll)
//! cargo build --release --features ffi
//!
//! # Generate C header
//! cbindgen --crate webylib --output include/webylib.h
//! ```

pub mod error;
pub mod types;
pub mod wallet_ops;

// Re-export all public FFI symbols at module root
pub use error::weby_last_error_message;
pub use types::weby_free_string;
pub use wallet_ops::*;

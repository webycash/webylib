//! HD derivation.
//!
//! Two modules:
//! - `legacy_webcash` — the existing 4-chain SHA256 scheme (Receive / Pay /
//!   Change / Mining). FROZEN; bit-for-bit compat required.
//! - `bip32` — BIP32/39/44 via `bitcoin::bip32`, path
//!   `m/83696968'/0'/family'/index'`, families `rgb=1`, `voucher=6`.
//!   Added in M4.
//!
//! Webcash flavor uses `legacy_webcash` exclusively; RGB and Voucher flavors
//! use `bip32`.

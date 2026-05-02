//! HD derivation for the webycash family.
//!
//! Two derivation schemes coexist:
//!
//! - [`legacy_webcash`] — the original 4-chain SHA256 scheme used by every
//!   wallet that has ever talked to webcash.org. Wire-format frozen.
//! - [`bip32`] — BIP32/39/44 derivation for the RGB and Voucher families.
//!   Path: `m/83696968'/0'/family'/index'`. Lands when the BIP32 wallet
//!   surface is first needed.
//!
//! [`ChainCode`] is shared between both schemes — `Receive`, `Pay`,
//! `Change`, `Mining` — so the recovery loop walks the same four chains
//! regardless of which derivation function turns chain+depth into a
//! 32-byte secret.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod legacy_webcash;

pub use legacy_webcash::{ChainCode, HdWallet};

//! Asset-generic wallet operations.
//!
//! Each op is parameterised over [`crate::WalletAsset`] and holds no
//! flavor-specific code. Per-flavor wallet crates wrap these ops with
//! their concrete type and any flavor-only side effects (`pay` for a
//! splittable asset; `transfer` for a non-splittable one — the
//! splittable-only ops are statically gated by additional bounds).

pub mod recover;

//! Asset-generic wallet operations.
//!
//! Where the flavor-specific wallet crates ([`webylib_wallet_webcash`],
//! [`webylib_wallet_rgb`], [`webylib_wallet_voucher`]) own the user-facing
//! API, this crate owns the operations whose shape is identical across
//! flavors. Today: recovery via [`ops::recover`]. As the wallet operations
//! land, [`ops`] gains `pay`, `replace`, `burn`, `check`, `stats`, `issue`
//! — each parameterised over [`asset::WalletAsset`] so a single
//! implementation serves all flavors.
//!
//! Design notes:
//!
//! - [`asset::WalletAsset`] is the wallet's view of an asset. It captures
//!   only what wallet ops need: how to render a public token for a
//!   `/health_check` lookup, how to extract the hash back from a server
//!   response key. The server-side asset crates carry the full
//!   `Asset + SplittableAsset + …` surface.
//! - Recovery returns [`recovery::RecoveryReport`] — a list of recovered
//!   outputs and the depth-walk results. Persistence is the caller's job;
//!   flavor crates wire the report to their own [`Store`]. This keeps
//!   recovery testable in isolation and lets the same primitive serve
//!   SQLite, in-memory, IndexedDB, or a one-shot CLI dump.
//!
//! [`Store`]: webylib_storage::Store

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod asset;
pub mod ops;
pub mod recovery;

pub use asset::{ChainCode, IssuedNamespace, WalletAsset};
pub use ops::recover::recover;
pub use recovery::{RecoveredOutput, RecoveryError, RecoveryReport};

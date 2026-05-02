//! The wallet's view of an asset.
//!
//! Wallet operations parameterise over [`WalletAsset`]. The server-side
//! asset crates (`webycash-asset-{webcash,rgb,voucher}`) carry the full
//! ledger surface; this trait captures only what flavor-agnostic wallet
//! ops need:
//!
//! 1. [`WalletAsset::Namespace`] — partition key for the asset's
//!    storage compartment. Webcash collapses this to `()`; RGB and
//!    Voucher carry `(ContractId, IssuerFp)`.
//! 2. [`WalletAsset::public_token_for_lookup`] — render a public-form
//!    token for `/api/v1/health_check`. Recovery doesn't need to know
//!    the asset's amount; the server returns it. Implementations
//!    construct the wire string from a 32-byte derived secret hex and
//!    the namespace.
//! 3. [`WalletAsset::extract_hash_from_response_key`] — parse the
//!    SHA256 hash back out of a server response key. The server's
//!    `/health_check` response keys mirror the input wire shape; the
//!    hash is at a known offset that varies by asset.
//!
//! Re-exports [`ChainCode`] from [`webylib_hd`] so wallet ops can use
//! the canonical 4-chain enum without an extra import.

pub use webylib_hd::ChainCode;

/// Storage / wire partition for issued assets (RGB, Voucher).
///
/// Webcash collapses [`WalletAsset::Namespace`] to `()`. Every other
/// flavor uses this struct: a (`ContractId`, issuer PGP fingerprint)
/// pair — issuer-scoped ledger compartment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IssuedNamespace {
    /// Asset-defined stable contract / series identifier.
    /// RGB: stringified `rgb_std::ContractId`. Voucher: issuer-chosen
    /// alphanumeric series name (max 64 chars per the protocol freeze).
    pub contract_id: String,
    /// Lower-case hex of the issuer's 20-byte OpenPGP V4 Ed25519
    /// fingerprint — unique per issuer cert.
    pub issuer_fp: String,
}

impl IssuedNamespace {
    /// Construct a namespace, normalising the issuer fingerprint to
    /// lower-case (the protocol freeze requires it).
    pub fn new(contract_id: impl Into<String>, issuer_fp: impl Into<String>) -> Self {
        Self {
            contract_id: contract_id.into(),
            issuer_fp: issuer_fp.into().to_ascii_lowercase(),
        }
    }
}

/// The wallet's view of an asset family. See module docs.
pub trait WalletAsset {
    /// Asset name — `"webcash"`, `"rgb-fungible"`, `"rgb-collectible"`,
    /// `"voucher"`. Used in error messages and log lines; must be stable.
    const NAME: &'static str;

    /// Partition key for the asset's storage compartment. `()` for
    /// webcash; for issued assets carries `(ContractId, IssuerFp)`.
    type Namespace: Clone + std::fmt::Debug;

    /// Render the public-form token a `/health_check` lookup would
    /// send for `secret_hex` under `ns`. Recovery uses this to query
    /// the server. The amount slot in the rendered string is a
    /// placeholder when the asset's wire format requires one — the
    /// server normalises it and reports the actual stored amount in
    /// its response.
    fn public_token_for_lookup(secret_hex: &str, ns: &Self::Namespace) -> String;

    /// Pull the 64-char SHA256 hash hex out of a server response key.
    /// Response keys mirror the wire shape; the hash is the
    /// asset-defined slot. Returns `None` when the key doesn't match
    /// this asset's expected layout (recovery skips that entry).
    fn extract_hash_from_response_key(key: &str) -> Option<&str>;

    /// Whether the server's `/api/v1/health_check` response includes
    /// an `"amount"` field for known tokens of this asset.
    ///
    /// - Webcash, RGB20, Voucher: `true`. The wallet relies on the
    ///   server-reported amount to materialise a recovered output. A
    ///   missing amount is a wire-format violation.
    /// - RGB21 collectible: `false`. RGB21 records carry no amount
    ///   semantics; recovery records the secret/hash and skips amount.
    const SERVER_REPORTS_AMOUNT: bool = true;
}

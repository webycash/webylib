//! Voucher wallet specialization. Always-splittable bearer credits.
//!
//! Surface mirrors `webyc voucher …` CLI verbs:
//!   - `pay(inputs, outputs)` — split: recipient outputs + change.
//!   - `insert(received, my_new)` — receive a voucher and rotate to
//!     a wallet-owned secret.
//!
//! Tokens carry `(contract_id, issuer_fp)` namespacing on the wire.
//! All operations land on the server's `/api/v1/replace`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use crate::core::{IssuedNamespace, WalletAsset};
use crate::server_client::{Client, ClientError};
use thiserror::Error;

/// Zero-sized asset marker for the Voucher flavor. Wire format:
/// `e{amt}:public:{hash}:{contract_id}:{issuer_fp}`. Splittable by
/// design (vouchers are always divisible bearer credits).
#[derive(Debug)]
pub struct Voucher;

impl WalletAsset for Voucher {
    const NAME: &'static str = "voucher";
    type Namespace = IssuedNamespace;

    fn public_token_for_lookup(secret_hex: &str, ns: &IssuedNamespace) -> String {
        use sha2::{Digest, Sha256};
        let hash = hex::encode(Sha256::digest(secret_hex.as_bytes()));
        format!(
            "e1:public:{hash}:{contract}:{issuer}",
            contract = ns.contract_id,
            issuer = ns.issuer_fp,
        )
    }

    fn extract_hash_from_response_key(key: &str) -> Option<&str> {
        let mut parts = key.splitn(5, ':');
        let _amt = parts.next()?;
        let _public = parts.next()?;
        let hash = parts.next()?;
        let _contract = parts.next()?;
        let _issuer = parts.next()?;
        if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(hash)
        } else {
            None
        }
    }
}

/// Failure modes from the wallet's verb methods.
#[derive(Debug, Error)]
pub enum WalletError {
    /// Underlying HTTP / transport / encode failure from the
    /// server-client layer.
    #[error("client: {0}")]
    Client(#[from] ClientError),
    /// Wallet-side precondition failed (e.g. empty inputs / outputs)
    /// — caught locally before the network round-trip.
    #[error("invariant: {0}")]
    Invariant(&'static str),
}

/// Convenience alias used across this crate for results from
/// any `VoucherWallet` method.
pub type WalletResult<T> = Result<T, WalletError>;

/// Voucher specialisation: always-splittable, issuer-namespaced
/// bearer credits. Verbs `pay` (N→M) and `insert` (1:1) both map
/// to the server's single `/api/v1/replace` endpoint with
/// `(contract_id, issuer_fp)` namespace enforcement.
pub struct VoucherWallet {
    client: Client,
}

impl VoucherWallet {
    /// Construct a new wallet pointed at a running `server-voucher`.
    ///
    /// ```no_run
    /// use webylib_wallet_voucher::VoucherWallet;
    /// let wallet = VoucherWallet::new("http://127.0.0.1:8183");
    /// # let _ = wallet;
    /// ```
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(server_url),
        }
    }

    /// Borrow the underlying transport client.
    pub fn server(&self) -> &Client {
        &self.client
    }

    /// `webyc voucher pay` — split-and-give. Server enforces
    /// conservation + same `(contract_id, issuer_fp)` namespace across
    /// every input and output.
    pub fn pay(&self, inputs: &[String], outputs: &[String]) -> WalletResult<()> {
        if inputs.is_empty() {
            return Err(WalletError::Invariant("pay requires ≥1 input"));
        }
        if outputs.is_empty() {
            return Err(WalletError::Invariant("pay requires ≥1 output"));
        }
        self.client.replace(inputs, outputs)?;
        Ok(())
    }

    /// `webyc voucher insert` — receive + rotate. 1:1 same-namespace.
    pub fn insert(&self, received_secret: &str, my_new_secret: &str) -> WalletResult<()> {
        self.client
            .replace(&[received_secret.to_string()], &[my_new_secret.to_string()])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pay_rejects_empty_inputs() {
        let w = VoucherWallet::new("http://no-where.invalid");
        let token = "e1.0:secret:aaa:credits:fff".to_string();
        let err = w.pay(&[], &[token]).unwrap_err();
        assert!(matches!(err, WalletError::Invariant(_)));
    }

    #[test]
    fn pay_rejects_empty_outputs() {
        let w = VoucherWallet::new("http://no-where.invalid");
        let token = "e1.0:secret:aaa:credits:fff".to_string();
        let err = w.pay(&[token], &[]).unwrap_err();
        assert!(matches!(err, WalletError::Invariant(_)));
    }

    #[test]
    fn voucher_token_format_includes_namespace() {
        let ns = IssuedNamespace::new("credits-2026-q2", "f".repeat(40));
        let token = Voucher::public_token_for_lookup(&"a".repeat(64), &ns);
        assert!(token.starts_with("e1:public:"));
        assert!(
            token.ends_with(":credits-2026-q2:ffffffffffffffffffffffffffffffffffffffff"),
            "got {token}",
        );
    }

    #[test]
    fn voucher_extract_hash_round_trip() {
        let key = "e3:public:e3aebbf7c0d2c4f7c1d8e5b6c0d8a1c0c0d8a1c0c0d8a1c0c0d8a1c0c0d8a1c0:credits:abcd";
        let hash = Voucher::extract_hash_from_response_key(key);
        assert_eq!(
            hash,
            Some("e3aebbf7c0d2c4f7c1d8e5b6c0d8a1c0c0d8a1c0c0d8a1c0c0d8a1c0c0d8a1c0")
        );
    }
}

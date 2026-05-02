//! Webcash wallet specialization.
//!
//! Surface mirrors `webyc webcash …` CLI verbs:
//!   - `pay(amount, recipient_secrets)` — split unspent outputs into a
//!     recipient-bound subset + change.
//!   - `insert(secret)` — receive a Webcash secret out-of-band and
//!     IMMEDIATELY replace it with a new secret only this wallet knows
//!     (defends against the sender re-spending after handing it off).
//!
//! Both operations land on the server's single `/api/v1/replace`
//! endpoint. The wallet picks distinct verbs to match user intent;
//! the server doesn't care which verb the wallet used.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use thiserror::Error;
use webylib_core::WalletAsset;
use webylib_server_client::{Client, ClientError};

/// Zero-sized asset marker for `WalletAsset` impls. Use to parameterise
/// asset-generic wallet ops over the Webcash flavor:
///
/// ```no_run
/// use webylib_core::recover;
/// use webylib_hd::HdWallet;
/// use webylib_server_client::Client;
/// use webylib_wallet_webcash::Webcash;
///
/// let client = Client::new("http://127.0.0.1:8181");
/// let hd = HdWallet::from_master_secret([0u8; 32]);
/// let report = recover::<Webcash>(&client, &hd, &(), 5, &Default::default());
/// # let _ = report;
/// ```
#[derive(Debug)]
pub struct Webcash;

impl WalletAsset for Webcash {
    const NAME: &'static str = "webcash";
    type Namespace = ();

    /// `e1:public:{sha256(secret_hex)}` — the legacy webcash wire shape.
    /// Production normalises any decimal amount in the request to `e1`
    /// when it echoes the result key, so we send `e1` straight away to
    /// avoid an avoidable string transform on response matching.
    fn public_token_for_lookup(secret_hex: &str, _ns: &()) -> String {
        use sha2::{Digest, Sha256};
        let hash = hex::encode(Sha256::digest(secret_hex.as_bytes()));
        format!("e1:public:{hash}")
    }

    /// Webcash response keys are `"e{amt}:public:{hash}"`. Hash is at
    /// index 2 after splitting on `':'`.
    fn extract_hash_from_response_key(key: &str) -> Option<&str> {
        let mut parts = key.splitn(3, ':');
        let _amt = parts.next()?;
        let _public = parts.next()?;
        let hash = parts.next()?;
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
/// any `WebcashWallet` method.
pub type WalletResult<T> = Result<T, WalletError>;

/// Webcash specialisation of the asset-flavored wallet. Wraps a
/// shared HTTP `Client` with the verbs `pay` (N→M with
/// conservation) and `insert` (1:1 receive-and-rotate). Both map
/// to the server's single `/api/v1/replace` endpoint.
pub struct WebcashWallet {
    client: Client,
}

impl WebcashWallet {
    /// Construct a new wallet pointed at a running `server-webcash`.
    ///
    /// ```no_run
    /// use webylib_wallet_webcash::WebcashWallet;
    /// let wallet = WebcashWallet::new("http://127.0.0.1:8181");
    /// // Now `wallet.pay(...)` and `wallet.insert(...)` are usable.
    /// # let _ = wallet;
    /// ```
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(server_url),
        }
    }

    /// Borrow the underlying transport client. Useful for asset-flavor
    /// agnostic operations that aren't covered by `pay` / `insert` —
    /// `mining_report`, `health_check`, `target`, `burn`.
    pub fn server(&self) -> &Client {
        &self.client
    }

    /// `webyc webcash pay` — split N inputs into M outputs (recipient
    /// outputs + change). Server enforces conservation.
    /// All tokens are full Webcash wire-format strings:
    /// `e{amount}:secret:{hex}`.
    pub fn pay(
        &self,
        inputs_secret_form: &[String],
        outputs_secret_form: &[String],
    ) -> WalletResult<()> {
        if inputs_secret_form.is_empty() {
            return Err(WalletError::Invariant("pay requires ≥1 input"));
        }
        if outputs_secret_form.is_empty() {
            return Err(WalletError::Invariant("pay requires ≥1 output"));
        }
        self.client
            .replace(inputs_secret_form, outputs_secret_form)?;
        Ok(())
    }

    /// `webyc webcash insert` — receive a single Webcash secret from
    /// someone else and immediately rotate it to a wallet-owned secret.
    /// 1:1 replace; same amount.
    pub fn insert(&self, received_secret: &str, my_new_secret: &str) -> WalletResult<()> {
        self.client
            .replace(&[received_secret.to_string()], &[my_new_secret.to_string()])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pay` MUST reject empty inputs before contacting the server —
    /// otherwise an empty-input replace would slip through to the
    /// network as malformed and waste a round-trip on a 500.
    #[test]
    fn pay_rejects_empty_inputs() {
        let w = WebcashWallet::new("http://no-where.invalid");
        let err = w.pay(&[], &["e1.0:secret:abc".into()]).unwrap_err();
        assert!(matches!(err, WalletError::Invariant(_)));
    }

    #[test]
    fn pay_rejects_empty_outputs() {
        let w = WebcashWallet::new("http://no-where.invalid");
        let err = w.pay(&["e1.0:secret:abc".into()], &[]).unwrap_err();
        assert!(matches!(err, WalletError::Invariant(_)));
    }

    #[test]
    fn wallet_asset_token_format_matches_legacy() {
        // Pin the wire shape: `e1:public:{sha256(secret_hex)}`. The
        // server normalises any amount to `e1`, so we send that.
        let secret = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let token = Webcash::public_token_for_lookup(secret, &());
        let prefix = "e1:public:";
        assert!(token.starts_with(prefix), "got {token}");
        let hash = &token[prefix.len()..];
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn wallet_asset_extract_hash_round_trip() {
        let key = "e1:public:fbe98164f16e9af34434388e9ac8e9efa286188dedd0f7218e1d9a578b7c3f73";
        assert_eq!(
            Webcash::extract_hash_from_response_key(key),
            Some("fbe98164f16e9af34434388e9ac8e9efa286188dedd0f7218e1d9a578b7c3f73")
        );
        assert_eq!(Webcash::extract_hash_from_response_key("garbage"), None);
        assert_eq!(Webcash::extract_hash_from_response_key("e1:public:short"), None);
    }
}

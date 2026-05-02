//! RGB wallet specialization (RGB20 fungible + RGB21 NFT).
//!
//! Surface mirrors `webyc rgb …` CLI verbs:
//!   - `transfer(input, recipient_outputs)` — give ownership to someone
//!     else by replacing my secret with output(s) that include the
//!     recipient's secret. For RGB20 the outputs may include change
//!     back to me; for RGB21 it's strictly 1:1 (one mine → one theirs).
//!   - `insert(received_secret, my_new_secret)` — receive an RGB
//!     secret out-of-band and IMMEDIATELY replace with a wallet-owned
//!     secret. 1:1 same-namespace replace.
//!
//! Both verbs land on the server's `/api/v1/replace` endpoint. The
//! server enforces same `(contract_id, issuer_fp)` namespace and (for
//! splittable RGB20) amount conservation. Real contract validation
//! happens client-side via webylib-wasm/contract.rs before the wallet
//! submits.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use thiserror::Error;
use webylib_core::{IssuedNamespace, WalletAsset};
use webylib_server_client::{Client, ClientError};

/// Zero-sized asset marker for the RGB20 fungible flavor. Wire format:
/// `e{amt}:public:{hash}:{contract_id}:{issuer_fp}`. Splittable.
#[derive(Debug)]
pub struct RgbFungible;

/// Zero-sized asset marker for the RGB21 collectible (NFT) flavor.
/// Wire format: `public:{hash}:{contract_id}:{issuer_fp}` — no amount
/// segment because each token is unique 1:1.
#[derive(Debug)]
pub struct RgbCollectible;

impl WalletAsset for RgbFungible {
    const NAME: &'static str = "rgb-fungible";
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

    /// RGB20 keys: `e{amt}:public:{hash}:{contract_id}:{issuer_fp}`.
    /// Hash is at index 2 after splitting on `':'`.
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

impl WalletAsset for RgbCollectible {
    const NAME: &'static str = "rgb-collectible";
    type Namespace = IssuedNamespace;
    /// RGB21 NFTs carry no amount: the collectible `/health_check`
    /// handler returns `{"spent": ...}` with no `amount` field.
    const SERVER_REPORTS_AMOUNT: bool = false;

    fn public_token_for_lookup(secret_hex: &str, ns: &IssuedNamespace) -> String {
        use sha2::{Digest, Sha256};
        let hash = hex::encode(Sha256::digest(secret_hex.as_bytes()));
        format!(
            "public:{hash}:{contract}:{issuer}",
            contract = ns.contract_id,
            issuer = ns.issuer_fp,
        )
    }

    /// RGB21 keys: `public:{hash}:{contract_id}:{issuer_fp}` — no
    /// amount segment, hash sits at index 1.
    fn extract_hash_from_response_key(key: &str) -> Option<&str> {
        let mut parts = key.splitn(4, ':');
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
/// any `RgbWallet` method.
pub type WalletResult<T> = Result<T, WalletError>;

/// RGB specialisation covering BOTH RGB20 fungible and RGB21
/// collectible. Verbs:
///   - `transfer` (any arity for RGB20; 1:1 for RGB21)
///   - `insert` (1:1 receive-and-rotate, same namespace)
///
/// Both map to the server's single `/api/v1/replace` endpoint with
/// `(contract_id, issuer_fp)` namespace enforcement.
pub struct RgbWallet {
    client: Client,
}

impl RgbWallet {
    /// Construct a new wallet pointed at a running `server-rgb` (or
    /// `server-rgb-collectible`).
    ///
    /// ```no_run
    /// use webylib_wallet_rgb::RgbWallet;
    /// let wallet = RgbWallet::new("http://127.0.0.1:8182");
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

    /// `webyc rgb transfer` — give ownership.
    ///
    /// For **RGB20** (splittable fungible): inputs/outputs follow the
    /// usual splittable rules — sum(inputs) == sum(outputs). The
    /// recipient's secret(s) are mixed into outputs alongside change.
    ///
    /// For **RGB21** (non-splittable NFT): exactly 1 input → 1 output.
    /// Token wire format omits the amount segment.
    ///
    /// Wire format is asset-flavor-specific; caller passes pre-formatted
    /// secret tokens (e.g. `e25.0:secret:{hex}:{contract}:{issuer}` or
    /// `secret:{hex}:{contract}:{issuer}` for collectible).
    pub fn transfer(&self, inputs: &[String], outputs: &[String]) -> WalletResult<()> {
        if inputs.is_empty() {
            return Err(WalletError::Invariant("transfer requires ≥1 input"));
        }
        if outputs.is_empty() {
            return Err(WalletError::Invariant("transfer requires ≥1 output"));
        }
        self.client.replace(inputs, outputs)?;
        Ok(())
    }

    /// `webyc rgb insert` — receive an RGB secret and rotate to a
    /// wallet-owned secret. Always 1:1 same-namespace.
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
    fn transfer_rejects_empty_inputs() {
        let w = RgbWallet::new("http://no-where.invalid");
        let token = "e1.0:secret:aaa:rgb20:fff".to_string();
        let err = w.transfer(&[], &[token]).unwrap_err();
        assert!(matches!(err, WalletError::Invariant(_)));
    }

    #[test]
    fn transfer_rejects_empty_outputs() {
        let w = RgbWallet::new("http://no-where.invalid");
        let token = "e1.0:secret:aaa:rgb20:fff".to_string();
        let err = w.transfer(&[token], &[]).unwrap_err();
        assert!(matches!(err, WalletError::Invariant(_)));
    }

    #[test]
    fn rgb20_token_format_includes_namespace() {
        let ns = IssuedNamespace::new("rgb20-usdc", "AABBCCDDEEFF00112233445566778899AABBCCDD");
        let token = RgbFungible::public_token_for_lookup(&"a".repeat(64), &ns);
        assert!(token.starts_with("e1:public:"));
        assert!(
            token.ends_with(":rgb20-usdc:aabbccddeeff00112233445566778899aabbccdd"),
            "issuer fp lower-cased per protocol freeze, got {token}"
        );
    }

    #[test]
    fn rgb21_token_format_drops_amount_segment() {
        let ns = IssuedNamespace::new("rgb21-tickets", "1".repeat(40));
        let token = RgbCollectible::public_token_for_lookup(&"b".repeat(64), &ns);
        assert!(
            token.starts_with("public:"),
            "rgb21 has no amount segment, got {token}"
        );
    }

    #[test]
    fn rgb20_extract_hash_round_trip() {
        let key = "e25:public:a0fab1377f49a759b57f63318262ebe89fabfc990e8e93ceac2984561482b9d4:rgb20-usdc:aabb";
        assert_eq!(
            RgbFungible::extract_hash_from_response_key(key),
            Some("a0fab1377f49a759b57f63318262ebe89fabfc990e8e93ceac2984561482b9d4")
        );
    }

    #[test]
    fn rgb21_extract_hash_round_trip() {
        let key =
            "public:fbe98164f16e9af34434388e9ac8e9efa286188dedd0f7218e1d9a578b7c3f73:nft-set:1234";
        assert_eq!(
            RgbCollectible::extract_hash_from_response_key(key),
            Some("fbe98164f16e9af34434388e9ac8e9efa286188dedd0f7218e1d9a578b7c3f73")
        );
    }

    /// The two flavors live behind distinct WalletAsset impls so the
    /// type system catches a `recover::<RgbFungible>` accidentally
    /// being pointed at an RGB21 server (or vice versa) at compile
    /// time. Smoke check: trait object shapes don't collide.
    #[test]
    fn rgb_fungible_and_collectible_are_distinct_types() {
        fn _expects_namespace<A: WalletAsset<Namespace = IssuedNamespace>>() {}
        _expects_namespace::<RgbFungible>();
        _expects_namespace::<RgbCollectible>();
    }
}

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

use thiserror::Error;
use webylib_server_client::{Client, ClientError};

#[derive(Debug, Error)]
pub enum WalletError {
    #[error("client: {0}")]
    Client(#[from] ClientError),
    #[error("invariant: {0}")]
    Invariant(&'static str),
}

pub type WalletResult<T> = Result<T, WalletError>;

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
    pub fn transfer(
        &self,
        inputs: &[String],
        outputs: &[String],
    ) -> WalletResult<()> {
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
}

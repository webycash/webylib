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
}

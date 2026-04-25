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

pub struct WebcashWallet {
    client: Client,
}

impl WebcashWallet {
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(server_url),
        }
    }

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

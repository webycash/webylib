//! Deterministic slot scan: probe one webcash master secret per wallet
//! slot and return the slots the server shows activity for.
//!
//! The caller owns slot derivation (which secret belongs to which slot);
//! this module owns only the webcash side — recovering each candidate
//! against the server and reporting what it found. Transport errors
//! propagate (fail-closed): a partial scan is never reported as complete.

use zeroize::Zeroizing;

use super::Wallet;
use crate::amount::Amount;
use crate::error::Result;
use crate::server::NetworkMode;

/// One wallet slot to probe.
pub struct SlotCandidate {
    /// Slot index in the caller's registry.
    pub index: u32,
    /// Label the caller files a hit under.
    pub label: String,
    /// 64-hex webcash master secret derived for this slot.
    pub master_secret_hex: Zeroizing<String>,
}

/// A slot the server reported activity for.
pub struct ActiveSlot {
    pub index: u32,
    pub label: String,
    /// Wallet state JSON (holds the slot's secrets).
    pub state_json: Zeroizing<String>,
    pub recovered_count: usize,
    pub balance: Amount,
}

impl std::fmt::Debug for ActiveSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveSlot")
            .field("index", &self.index)
            .field("label", &self.label)
            .field("state_json", &"<redacted>")
            .field("recovered_count", &self.recovered_count)
            .field("balance", &self.balance)
            .finish()
    }
}

/// Recover every candidate against `network` and return the active ones
/// (any recovered output or a non-zero balance), in candidate order.
pub async fn scan_slots(
    network: NetworkMode,
    candidates: &[SlotCandidate],
    gap_limit: usize,
) -> Result<Vec<ActiveSlot>> {
    let mut active = Vec::new();
    for candidate in candidates {
        let wallet = scratch_wallet(network.clone())?;
        wallet
            .store_master_secret(&candidate.master_secret_hex)
            .await?;
        let result = wallet.recover_from_wallet(gap_limit).await?;
        let balance = wallet.balance_amount().await?;
        if result.recovered_count > 0 || balance.wats > 0 {
            active.push(ActiveSlot {
                index: candidate.index,
                label: candidate.label.clone(),
                state_json: Zeroizing::new(wallet.to_json()?),
                recovered_count: result.recovered_count,
                balance,
            });
        }
    }
    Ok(active)
}

#[cfg(target_arch = "wasm32")]
fn scratch_wallet(network: NetworkMode) -> Result<Wallet> {
    Wallet::new_memory(network)
}

#[cfg(not(target_arch = "wasm32"))]
fn scratch_wallet(network: NetworkMode) -> Result<Wallet> {
    Wallet::open_json_memory(network)
}

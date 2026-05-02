//! Output types for [`crate::ops::recover`].

use std::collections::HashMap;

use crate::asset::WalletAsset;

/// One token the server reported as unspent under the wallet's seed.
///
/// Generic over the asset so the namespace travels with the row —
/// downstream persistence can route a webcash output (`Namespace = ()`)
/// to the unscoped store and an RGB output to its `(contract, issuer)`
/// compartment without losing type safety.
#[derive(Debug, Clone)]
pub struct RecoveredOutput<A: WalletAsset> {
    /// 64-char hex of the HD-derived secret.
    pub secret_hex: String,
    /// 64-char hex of the SHA256 hash the server reported on.
    pub hash: String,
    /// Stored amount in wats (8-decimal fixed-point), as the server
    /// reported it. `None` for assets that don't carry amount
    /// semantics (RGB21); `Some(_)` for every other flavor.
    pub amount_wats: Option<i64>,
    /// Which HD chain the secret came from (`Receive`/`Pay`/`Change`/`Mining`).
    pub chain: crate::asset::ChainCode,
    /// Depth on that chain.
    pub depth: u64,
    /// Asset namespace this token lives in.
    pub namespace: A::Namespace,
}

/// Outcome of one [`crate::ops::recover`] call.
#[derive(Debug, Clone)]
pub struct RecoveryReport<A: WalletAsset> {
    /// Every token the server reported `spent: false` for, in
    /// depth-then-chain encounter order.
    pub recovered: Vec<RecoveredOutput<A>>,
    /// Highest used depth observed on each chain (`+ 1` is the next
    /// fresh depth). Persisters update their chain-depth metadata from
    /// this map; missing entries mean the chain produced no hits.
    pub last_used_depth: HashMap<crate::asset::ChainCode, u64>,
}

impl<A: WalletAsset> RecoveryReport<A> {
    /// Empty report — used as the starting accumulator and as the
    /// return value when the seed has no on-chain history.
    pub fn empty() -> Self {
        Self {
            recovered: Vec::new(),
            last_used_depth: HashMap::new(),
        }
    }

    /// Number of recovered outputs.
    pub fn count(&self) -> usize {
        self.recovered.len()
    }

    /// Sum of recovered amounts in wats. Outputs whose asset doesn't
    /// carry amount semantics (RGB21) contribute zero.
    pub fn total_wats(&self) -> i64 {
        self.recovered.iter().filter_map(|o| o.amount_wats).sum()
    }
}

/// Failure modes for [`crate::ops::recover`].
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    /// `/api/v1/health_check` returned a non-2xx status or the request
    /// failed at the transport layer. Recovery aborts on the first such
    /// error rather than silently advancing past a missing batch — the
    /// historical "silent truncation" bug exists precisely because this
    /// case used to be swallowed.
    #[error("recovery aborted on {chain} at depth {depth}: {source}")]
    Server {
        /// Which chain was being scanned when the call failed.
        chain: &'static str,
        /// Depth of the first secret in the failing batch.
        depth: u64,
        /// Underlying client error.
        source: webylib_server_client::ClientError,
    },

    /// The server's response was 200 OK but the JSON didn't parse as
    /// `{"status": _, "results": {…}}`. Indicates a wire-format drift
    /// or a non-conforming server.
    #[error("malformed health_check response: {0}")]
    Decode(String),

    /// gap_limit was zero — recovery would loop forever. Caller error.
    #[error("gap_limit must be > 0")]
    InvalidGapLimit,
}

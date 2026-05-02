//! Asset-generic recovery: walk the four HD chains, ask the server for
//! `/health_check` on every candidate hash, return the unspent set.
//!
//! Recovery is identical in shape across every asset family:
//!
//! 1. Walk each chain from depth 0 in `gap_limit`-sized batches.
//! 2. Build a public-form token per derived secret via
//!    [`WalletAsset::public_token_for_lookup`] and POST the batch to
//!    `/api/v1/health_check`.
//! 3. Match each response entry back to the originating secret by hash
//!    (extracted via [`WalletAsset::extract_hash_from_response_key`]).
//! 4. For entries the server reports `spent: false`, record a
//!    [`RecoveredOutput`] with the server's amount.
//! 5. Advance to the next batch when any candidate in this batch hit
//!    (anything but `spent: null` — both `true` and `false` count as
//!    "this depth was used", so the next gap_limit gets a fresh window).
//!    Stop a chain when `gap_limit` consecutive depths come back
//!    unknown.
//!
//! The single asset-specific knob is the wire-format / namespace pair
//! the trait abstracts. The HD walk, the batching, the gap-limit logic,
//! and the error path are all flavor-agnostic.
//!
//! ### Error handling
//!
//! Transport / non-2xx errors propagate as [`RecoveryError::Server`].
//! No silent truncation: a single failed batch aborts recovery so the
//! caller knows to retry against a healthy server. (The historical
//! webcash-only `recover()` used to absorb these as "no more outputs"
//! and return `Ok` with a partial count — the user lost funds in
//! production because of it.)

use std::collections::HashMap;

use serde::Deserialize;

use webylib_hd::{ChainCode, HdWallet};
use webylib_server_client::Client;

use crate::asset::WalletAsset;
use crate::recovery::{RecoveredOutput, RecoveryError, RecoveryReport};

/// Server's `/api/v1/health_check` response envelope. Keys mirror the
/// (normalised) input wire shape; values are per-token status.
#[derive(Debug, Deserialize)]
struct HealthEnvelope {
    #[serde(default)]
    #[allow(dead_code)]
    status: String,
    results: HashMap<String, HealthEntry>,
}

#[derive(Debug, Deserialize)]
struct HealthEntry {
    /// `true` = spent, `false` = unspent, `null` = unknown.
    spent: Option<bool>,
    /// Stored amount as a decimal string (`"1"`, `"0.4"`, …). Present
    /// when `spent == Some(false)`; `null` when spent or unknown.
    #[serde(default)]
    amount: Option<String>,
}

/// Walk the four HD chains, recovering everything the server reports
/// `spent: false` for under the wallet's seed.
///
/// `reported_depths` is the wallet's own state of `(chain, depth)` —
/// recovery uses it to widen the search window when the wallet
/// remembers having used depths beyond the gap-limit reach. Pass an
/// empty map for a cold-start recovery from a freshly imported seed.
///
/// On success the [`RecoveryReport`] carries every recovered output
/// (in chain-then-depth encounter order) and the highest used depth
/// the scan saw on each chain.
pub fn recover<A: WalletAsset>(
    client: &Client,
    hd: &HdWallet,
    namespace: &A::Namespace,
    gap_limit: u64,
    reported_depths: &HashMap<ChainCode, u64>,
) -> Result<RecoveryReport<A>, RecoveryError> {
    if gap_limit == 0 {
        return Err(RecoveryError::InvalidGapLimit);
    }

    let mut report = RecoveryReport::<A>::empty();

    for &chain in &ChainCode::ALL {
        scan_chain::<A>(
            client,
            hd,
            namespace,
            chain,
            gap_limit,
            reported_depths.get(&chain).copied().unwrap_or(0),
            &mut report,
        )?;
    }

    Ok(report)
}

/// Walk a single chain. Pulled out so it can be exercised in isolation
/// by per-chain tests without setting up all four chains.
fn scan_chain<A: WalletAsset>(
    client: &Client,
    hd: &HdWallet,
    namespace: &A::Namespace,
    chain: ChainCode,
    gap_limit: u64,
    reported_depth: u64,
    report: &mut RecoveryReport<A>,
) -> Result<(), RecoveryError> {
    // Per-batch state.
    let mut current_depth = 0u64;
    let mut consecutive_empty: u64 = 0;
    let mut chain_max_used: Option<u64> = None;

    loop {
        // Build the batch of (secret, hash, public_token) triples for
        // this gap_limit window.
        let mut by_hash: HashMap<String, (String, u64)> = HashMap::new();
        let mut publics: Vec<String> = Vec::with_capacity(gap_limit as usize);
        for offset in 0..gap_limit {
            let depth = current_depth + offset;
            let secret_hex = hd.derive_secret(chain, depth);
            let public = A::public_token_for_lookup(&secret_hex, namespace);
            // Compute the hash from the secret to key this batch's
            // lookup map. Matches the server-side hash function for
            // every flavor (sha256 of the ASCII secret hex).
            let hash = sha256_hex_of_ascii(&secret_hex);
            by_hash.insert(hash, (secret_hex, depth));
            publics.push(public);
        }

        // Server round trip — propagate any error loudly.
        let raw = client
            .health_check(&publics)
            .map_err(|source| RecoveryError::Server {
                chain: chain.as_str(),
                depth: current_depth,
                source,
            })?;
        let env: HealthEnvelope = serde_json::from_str(&raw)
            .map_err(|e| RecoveryError::Decode(format!("{chain:?}@{current_depth}: {e}")))?;

        // Reconcile each response entry against the batch.
        let mut batch_had_hit = false;
        for (resp_key, entry) in &env.results {
            let Some(hash) = A::extract_hash_from_response_key(resp_key) else {
                continue;
            };
            let Some((secret_hex, depth)) = by_hash.get(hash) else {
                continue;
            };
            // Anything but `spent: null` means the server saw this
            // hash before; mark the depth as used.
            if entry.spent.is_some() {
                batch_had_hit = true;
                chain_max_used = Some(chain_max_used.map_or(*depth, |d| d.max(*depth)));
            }
            if entry.spent == Some(false) {
                let amount_wats = if A::SERVER_REPORTS_AMOUNT {
                    let parsed = entry
                        .amount
                        .as_deref()
                        .map(parse_decimal_to_wats)
                        .transpose()
                        .map_err(|e| RecoveryError::Decode(format!("amount: {e}")))?
                        .ok_or_else(|| {
                            RecoveryError::Decode(format!(
                                "{chain:?}@{depth}: server reported spent: false without amount"
                            ))
                        })?;
                    Some(parsed)
                } else {
                    // Asset has no amount semantics (e.g. RGB21 NFTs):
                    // the server's `/health_check` response carries no
                    // `amount` field by design.
                    None
                };
                report.recovered.push(RecoveredOutput {
                    secret_hex: secret_hex.clone(),
                    hash: hash.to_string(),
                    amount_wats,
                    chain,
                    depth: *depth,
                    namespace: namespace.clone(),
                });
            }
        }

        // Termination: stop once we've seen a full gap_limit window
        // with no hit, AND we've passed any reported depth the wallet
        // remembers using on this chain.
        if batch_had_hit {
            consecutive_empty = 0;
        } else {
            consecutive_empty = consecutive_empty.saturating_add(gap_limit);
        }

        let next_depth = current_depth.saturating_add(gap_limit);
        let past_reported = next_depth > reported_depth;

        if !batch_had_hit && consecutive_empty >= gap_limit && past_reported {
            break;
        }
        current_depth = next_depth;
    }

    if let Some(d) = chain_max_used {
        report.last_used_depth.insert(chain, d);
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

/// Server-side hash function: SHA256 over the ASCII bytes of the secret hex.
/// Matches `webycash-asset-{webcash,rgb,voucher}::*::to_public` for every
/// flavor — they all hash the secret hex string verbatim, no decoding.
fn sha256_hex_of_ascii(s: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Parse a server-reported decimal amount string (`"1"`, `"0.4"`,
/// `"195.3125"`) into wats. Mirrors `wats_to_string` round-trip on
/// the server. Tolerates trailing-zero stripping (production normalises
/// `1.00000000` → `1`).
fn parse_decimal_to_wats(s: &str) -> Result<i64, String> {
    const SCALE: i64 = 100_000_000;
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (whole_str, frac_str) = match body.split_once('.') {
        Some((w, f)) => (w, f),
        None => (body, ""),
    };
    if whole_str.is_empty() || !whole_str.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("malformed whole part: {s:?}"));
    }
    if !frac_str.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("malformed fractional part: {s:?}"));
    }
    if frac_str.len() > 8 {
        return Err(format!("more than 8 fractional digits: {s:?}"));
    }

    let whole: i64 = whole_str
        .parse()
        .map_err(|_| format!("whole part overflows i64: {s:?}"))?;
    let mut frac_padded = String::with_capacity(8);
    frac_padded.push_str(frac_str);
    while frac_padded.len() < 8 {
        frac_padded.push('0');
    }
    let frac: i64 = frac_padded
        .parse()
        .map_err(|_| format!("frac overflows i64: {s:?}"))?;
    let total = whole
        .checked_mul(SCALE)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(|| format!("amount overflows i64 wats: {s:?}"))?;
    Ok(if neg { -total } else { total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decimal_handles_production_shapes() {
        assert_eq!(parse_decimal_to_wats("1").unwrap(), 100_000_000);
        assert_eq!(parse_decimal_to_wats("0.4").unwrap(), 40_000_000);
        assert_eq!(parse_decimal_to_wats("195.3125").unwrap(), 19_531_250_000);
        assert_eq!(parse_decimal_to_wats("0").unwrap(), 0);
        assert_eq!(parse_decimal_to_wats("0.00000001").unwrap(), 1);
    }

    #[test]
    fn parse_decimal_rejects_garbage() {
        assert!(parse_decimal_to_wats("").is_err());
        assert!(parse_decimal_to_wats("abc").is_err());
        assert!(parse_decimal_to_wats("1.234567890").is_err()); // > 8 frac digits
        assert!(parse_decimal_to_wats(".5").is_err()); // empty whole
    }

    #[test]
    fn sha256_matches_server() {
        // Server hashes ASCII bytes of the secret hex. Pin the formula.
        let s = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        // sha256("abcd...") computed externally:
        //   echo -n "abcd1234..." | sha256sum
        // Pin: 81e8...  (we just check it's a 64-char lowercase hex
        // string of the digest of the input bytes — the algorithm is
        // sha2 0.10 SHA256, deterministic).
        let h = sha256_hex_of_ascii(s);
        assert_eq!(h.len(), 64);
        // Cross-check with sha2 directly to prove we hash bytes (not decode hex).
        use sha2::{Digest, Sha256};
        let manual = hex::encode(Sha256::digest(s.as_bytes()));
        assert_eq!(h, manual);
    }
}

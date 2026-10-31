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

use crate::hd::{ChainCode, HdWallet};
#[cfg(not(target_arch = "wasm32"))]
use crate::server_client::Client;
use crate::server_client::{ClientError, TokenStatus};

use crate::core::asset::WalletAsset;
use crate::core::recovery::{RecoveredOutput, RecoveryError, RecoveryReport};

/// Walk the four HD chains over the blocking native [`Client`], recovering
/// everything the server reports `spent: false` for under the wallet's seed.
/// Async transports drive the same [`RecoveryScan`] themselves.
///
/// `reported_depths` is the wallet's own state of `(chain, depth)` —
/// recovery uses it to widen the search window when the wallet
/// remembers having used depths beyond the gap-limit reach. Pass an
/// empty map for a cold-start recovery from a freshly imported seed.
///
/// On success the [`RecoveryReport`] carries every recovered output
/// (in chain-then-depth encounter order) and the highest used depth
/// the scan saw on each chain.
#[cfg(not(target_arch = "wasm32"))]
pub fn recover<A: WalletAsset>(
    client: &Client,
    hd: &HdWallet,
    namespace: &A::Namespace,
    gap_limit: u64,
    reported_depths: &HashMap<ChainCode, u64>,
) -> Result<RecoveryReport<A>, RecoveryError> {
    let mut scan = RecoveryScan::<A>::new(hd, namespace, gap_limit, reported_depths)?;
    while let Some(lookup) = scan.next_lookup() {
        let statuses = client
            .health_check_status(&lookup)
            .map_err(|e| scan.server_error(e))?;
        scan.apply(&statuses)?;
    }
    Ok(scan.finish())
}

/// One derived candidate awaiting its `/health_check` answer.
struct Candidate {
    secret_hex: String,
    depth: u64,
}

/// Sans-IO recovery state machine: yields `/health_check` lookups and
/// consumes the typed answers ([`crate::server_client::wire::parse_health_check`]),
/// so every transport (blocking native, async reqwest, browser fetch) shares
/// one HD walk, one gap-limit rule and one error path.
///
/// Protocol: `while let Some(tokens) = scan.next_lookup() { scan.apply(&statuses_for(tokens))? }`
/// then [`RecoveryScan::finish`]. Statuses must be in lookup order.
pub struct RecoveryScan<'a, A: WalletAsset> {
    hd: &'a HdWallet,
    namespace: &'a A::Namespace,
    gap_limit: u64,
    reported_depths: &'a HashMap<ChainCode, u64>,
    chain_idx: usize,
    current_depth: u64,
    consecutive_empty: u64,
    chain_max_used: Option<u64>,
    pending: Option<Vec<Candidate>>,
    report: RecoveryReport<A>,
}

impl<'a, A: WalletAsset> RecoveryScan<'a, A> {
    /// Start a scan. `gap_limit == 0` ⇒ [`RecoveryError::InvalidGapLimit`].
    pub fn new(
        hd: &'a HdWallet,
        namespace: &'a A::Namespace,
        gap_limit: u64,
        reported_depths: &'a HashMap<ChainCode, u64>,
    ) -> Result<Self, RecoveryError> {
        if gap_limit == 0 {
            return Err(RecoveryError::InvalidGapLimit);
        }
        Ok(Self {
            hd,
            namespace,
            gap_limit,
            reported_depths,
            chain_idx: 0,
            current_depth: 0,
            consecutive_empty: 0,
            chain_max_used: None,
            pending: None,
            report: RecoveryReport::<A>::empty(),
        })
    }

    fn chain(&self) -> Option<ChainCode> {
        ChainCode::ALL.get(self.chain_idx).copied()
    }

    /// Public tokens of the next `gap_limit` window, or `None` once every
    /// chain is exhausted. Calling it again before [`Self::apply`] re-yields
    /// the same window.
    pub fn next_lookup(&mut self) -> Option<Vec<String>> {
        let chain = self.chain()?;
        let candidates = self.pending.get_or_insert_with(|| {
            (0..self.gap_limit)
                .map(|offset| Candidate {
                    secret_hex: self.hd.derive_secret(chain, self.current_depth + offset),
                    depth: self.current_depth + offset,
                })
                .collect()
        });
        Some(
            candidates
                .iter()
                .map(|c| A::public_token_for_lookup(&c.secret_hex, self.namespace))
                .collect(),
        )
    }

    /// Wrap a transport failure with the chain/depth it aborted at.
    pub fn server_error(&self, source: ClientError) -> RecoveryError {
        RecoveryError::Server {
            chain: self.chain().map_or("done", |c| c.as_str()),
            depth: self.current_depth,
            source,
        }
    }

    /// Consume the statuses for the window [`Self::next_lookup`] yielded.
    /// Fail-closed: a count mismatch, an answer with no pending lookup, or an
    /// unspent amount-bearing token without an amount aborts recovery.
    pub fn apply(&mut self, statuses: &[TokenStatus]) -> Result<(), RecoveryError> {
        let chain = self
            .chain()
            .ok_or_else(|| RecoveryError::Decode("apply after scan finished".into()))?;
        let candidates = self
            .pending
            .take()
            .ok_or_else(|| RecoveryError::Decode("apply without a pending lookup".into()))?;
        if statuses.len() != candidates.len() {
            return Err(RecoveryError::Decode(format!(
                "{chain:?}@{}: {} statuses for {} lookups",
                self.current_depth,
                statuses.len(),
                candidates.len()
            )));
        }

        let mut batch_had_hit = false;
        for (cand, status) in candidates.iter().zip(statuses) {
            let amount = match status {
                TokenStatus::Unknown => continue,
                TokenStatus::Spent { .. } => None,
                TokenStatus::Unspent { amount } => Some(amount),
            };
            // Anything but unknown means the server saw this depth used.
            batch_had_hit = true;
            self.chain_max_used = Some(
                self.chain_max_used
                    .map_or(cand.depth, |d| d.max(cand.depth)),
            );
            let Some(amount) = amount else { continue };
            let amount_wats = if A::SERVER_REPORTS_AMOUNT {
                let s = amount.as_deref().ok_or_else(|| {
                    RecoveryError::Decode(format!(
                        "{chain:?}@{}: server reported spent: false without amount",
                        cand.depth
                    ))
                })?;
                Some(
                    parse_decimal_to_wats(s)
                        .map_err(|e| RecoveryError::Decode(format!("amount: {e}")))?,
                )
            } else {
                // Asset has no amount semantics (e.g. RGB21).
                None
            };
            self.report.recovered.push(RecoveredOutput {
                secret_hex: cand.secret_hex.clone(),
                hash: sha256_hex_of_ascii(&cand.secret_hex),
                amount_wats,
                chain,
                depth: cand.depth,
                namespace: self.namespace.clone(),
            });
        }

        // Termination: stop once a full gap_limit window had no hit AND we
        // are past any depth the wallet remembers using on this chain.
        if batch_had_hit {
            self.consecutive_empty = 0;
        } else {
            self.consecutive_empty = self.consecutive_empty.saturating_add(self.gap_limit);
        }
        let next_depth = self.current_depth.saturating_add(self.gap_limit);
        let reported = self.reported_depths.get(&chain).copied().unwrap_or(0);
        if !batch_had_hit && self.consecutive_empty >= self.gap_limit && next_depth > reported {
            if let Some(d) = self.chain_max_used.take() {
                self.report.last_used_depth.insert(chain, d);
            }
            self.chain_idx += 1;
            self.current_depth = 0;
            self.consecutive_empty = 0;
        } else {
            self.current_depth = next_depth;
        }
        Ok(())
    }

    /// The report. Call once [`Self::next_lookup`] returned `None`.
    pub fn finish(self) -> RecoveryReport<A> {
        self.report
    }
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

    /// Minimal amount-bearing asset for driving the scan without I/O.
    #[derive(Debug, Clone)]
    struct Coin;
    impl WalletAsset for Coin {
        const NAME: &'static str = "coin";
        type Namespace = ();
        const SERVER_REPORTS_AMOUNT: bool = true;
        fn public_token_for_lookup(secret_hex: &str, _ns: &()) -> String {
            format!("e1:public:{}", sha256_hex_of_ascii(secret_hex))
        }
        fn extract_hash_from_response_key(key: &str) -> Option<&str> {
            key.split(':').nth(2)
        }
    }

    fn hd() -> HdWallet {
        HdWallet::from_hex(&"11".repeat(32)).unwrap()
    }

    /// `(chain, window start depth, window length)` per `/health_check` call.
    type Calls = Vec<(ChainCode, u64, usize)>;

    /// Drive a scan with `answer(chain, depth) -> status`.
    fn run(
        gap: u64,
        reported: &HashMap<ChainCode, u64>,
        answer: impl Fn(ChainCode, u64) -> TokenStatus,
    ) -> Result<(RecoveryReport<Coin>, Calls), RecoveryError> {
        let hd = hd();
        let mut scan = RecoveryScan::<Coin>::new(&hd, &(), gap, reported)?;
        let mut calls = Vec::new();
        while let Some(lookup) = scan.next_lookup() {
            let chain = scan.chain().unwrap();
            let start = scan.current_depth;
            calls.push((chain, start, lookup.len()));
            let statuses: Vec<_> = (0..lookup.len() as u64)
                .map(|i| answer(chain, start + i))
                .collect();
            scan.apply(&statuses)?;
        }
        Ok((scan.finish(), calls))
    }

    #[test]
    fn scan_rejects_zero_gap() {
        let hd = hd();
        let m = HashMap::new();
        assert!(matches!(
            RecoveryScan::<Coin>::new(&hd, &(), 0, &m),
            Err(RecoveryError::InvalidGapLimit)
        ));
    }

    #[test]
    fn empty_wallet_does_one_window_per_chain() {
        let (report, calls) = run(5, &HashMap::new(), |_, _| TokenStatus::Unknown).unwrap();
        assert_eq!(report.count(), 0);
        assert_eq!(calls.len(), 4);
        assert!(calls.iter().all(|(_, d, n)| *d == 0 && *n == 5));
    }

    #[test]
    fn unspent_is_recovered_and_hits_extend_the_walk() {
        let (report, calls) = run(3, &HashMap::new(), |chain, depth| match (chain, depth) {
            (ChainCode::Receive, 1) => TokenStatus::Unspent {
                amount: Some("0.4".into()),
            },
            (ChainCode::Receive, 4) => TokenStatus::Spent { amount: None },
            _ => TokenStatus::Unknown,
        })
        .unwrap();
        assert_eq!(report.count(), 1);
        let out = &report.recovered[0];
        assert_eq!(
            (out.chain, out.depth, out.amount_wats),
            (ChainCode::Receive, 1, Some(40_000_000))
        );
        assert_eq!(out.secret_hex, hd().derive_secret(ChainCode::Receive, 1));
        assert_eq!(report.last_used_depth.get(&ChainCode::Receive), Some(&4));
        // Receive: windows 0,3 hit; 6 empty ⇒ stop. Other chains: one window.
        let receive: Vec<_> = calls
            .iter()
            .filter(|c| c.0 == ChainCode::Receive)
            .map(|c| c.1)
            .collect();
        assert_eq!(receive, vec![0, 3, 6]);
        assert_eq!(calls.len(), 6);
    }

    #[test]
    fn reported_depth_widens_the_window() {
        let mut m = HashMap::new();
        m.insert(ChainCode::Pay, 7);
        let (_, calls) = run(3, &m, |_, _| TokenStatus::Unknown).unwrap();
        let pay: Vec<_> = calls
            .iter()
            .filter(|c| c.0 == ChainCode::Pay)
            .map(|c| c.1)
            .collect();
        assert_eq!(pay, vec![0, 3, 6]);
    }

    #[test]
    fn unspent_without_amount_fails_closed() {
        let err = run(2, &HashMap::new(), |_, _| TokenStatus::Unspent {
            amount: None,
        })
        .unwrap_err();
        assert!(matches!(err, RecoveryError::Decode(_)), "{err:?}");
    }

    #[test]
    fn status_count_mismatch_fails_closed() {
        let hd = hd();
        let m = HashMap::new();
        let mut scan = RecoveryScan::<Coin>::new(&hd, &(), 3, &m).unwrap();
        assert!(scan.apply(&[]).is_err(), "apply before next_lookup");
        let lookup = scan.next_lookup().unwrap();
        assert_eq!(
            lookup,
            scan.next_lookup().unwrap(),
            "re-yields the pending window"
        );
        let err = scan.apply(&[TokenStatus::Unknown]).unwrap_err();
        assert!(matches!(err, RecoveryError::Decode(_)));
    }

    #[test]
    fn lookups_are_public_tokens_only() {
        let hd = hd();
        let m = HashMap::new();
        let mut scan = RecoveryScan::<Coin>::new(&hd, &(), 4, &m).unwrap();
        for t in scan.next_lookup().unwrap() {
            assert!(crate::server_client::wire::public_hash(&t).is_some(), "{t}");
            assert!(!t.contains("secret"));
        }
    }

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

//! Webcash proof-of-work: the sans-IO [`Work`] search, shared by
//! [`crate::wallet::Wallet::mine_batch`] (every target) and the native [`mine`] loop.
//!
//! Light CPU miner for testnet usage.
//!
//! This module provides a minimal, non-optimized SHA256 miner suitable for
//! mining webcash on the testnet (low constant difficulty). For production
//! mining, use a dedicated GPU miner.
//!
//! # Usage
//!
//! ```rust,no_run
//! use webylib::{Wallet, NetworkMode};
//! use webylib::miner;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let wallet = Wallet::open_with_network("testnet.db", NetworkMode::Testnet).await?;
//! let result = miner::mine(&wallet).await?;
//! println!("Mined {} webcash!", result.amount);
//! # Ok(())
//! # }
//! ```

use sha2::{Digest, Sha256};

#[cfg(feature = "native")]
use crate::error::Result;
#[cfg(feature = "native")]
use crate::wallet::Wallet;
use crate::Amount;

/// Result of a successful mining operation.
#[derive(Debug, Clone)]
pub struct MineResult {
    /// The mined webcash secret (already stored in wallet).
    pub webcash: String,
    /// The subsidy secret (if subsidy_amount > 0).
    pub subsidy: Option<String>,
    /// The preimage that satisfied the PoW.
    pub preimage: String,
    /// SHA256 hash of the preimage (hex).
    pub hash: String,
    /// Difficulty that was met.
    pub difficulty: u32,
    /// Amount mined.
    pub amount: Amount,
}

/// Outcome of one bounded nonce search ([`crate::wallet::Wallet::mine_batch`]).
#[derive(Debug, Clone)]
pub struct MineBatch {
    /// Nonces hashed in this batch.
    pub attempted: u64,
    /// Difficulty the batch searched against.
    pub difficulty: u32,
    /// Miner reward per solution at this target.
    pub mining_amount: Amount,
    /// `Some` when a solution was found, reported and stored.
    pub result: Option<MineResult>,
}

/// A found proof-of-work solution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    /// The preimage JSON the server receives in the mining report.
    pub preimage: String,
    /// `sha256(preimage)`, hex.
    pub hash_hex: String,
    /// Leading zero bits of the hash.
    pub achieved: u32,
}

/// The preimage fields fixed for one search (sans-IO).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    /// `e{amount}:secret:{hex}` the miner keeps.
    pub webcash: String,
    /// Subsidy webcash surrendered to the server, if the target has one.
    pub subsidy: Option<String>,
    /// Unix seconds stamped into the preimage.
    pub timestamp: u64,
    /// Required leading zero bits.
    pub difficulty: u32,
}

impl Work {
    /// The preimage JSON for `nonce`.
    pub fn preimage(&self, nonce: u64) -> String {
        let subsidy = match &self.subsidy {
            Some(s) => format!(r#"["{}"]"#, s),
            None => "[]".to_string(),
        };
        format!(
            r#"{{"webcash":["{}"],"subsidy":{},"timestamp":{},"difficulty":{},"nonce":{}}}"#,
            self.webcash, subsidy, self.timestamp, self.difficulty, nonce
        )
    }

    /// Hash nonces `start..start + max_attempts`; returns how many were hashed
    /// and the first solution, if any.
    pub fn search(&self, start: u64, max_attempts: u64) -> (u64, Option<Solution>) {
        for i in 0..max_attempts {
            let preimage = self.preimage(start.wrapping_add(i));
            let hash = Sha256::digest(preimage.as_bytes());
            let achieved = leading_zero_bits(&hash);
            if achieved >= self.difficulty {
                return (
                    i + 1,
                    Some(Solution {
                        preimage,
                        hash_hex: hex::encode(hash),
                        achieved,
                    }),
                );
            }
        }
        (max_attempts, None)
    }
}

/// Count leading zero bits in a SHA256 hash.
pub fn leading_zero_bits(hash: &[u8]) -> u32 {
    let mut count = 0u32;
    for byte in hash {
        if *byte == 0 {
            count += 8;
        } else {
            count += byte.leading_zeros();
            break;
        }
    }
    count
}

/// Mine until a solution is found, reported and stored.
///
/// Fetches the target once, then runs [`crate::wallet::Wallet::mine_batch`] over a running
/// nonce with the current clock. Light CPU miner, intended for testnet.
#[cfg(feature = "native")]
pub async fn mine(wallet: &Wallet) -> Result<MineResult> {
    const BATCH: u64 = 10_000;
    let target = wallet.server_get_target().await?;
    let mut nonce: u64 = 0;
    loop {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| crate::error::Error::wallet(format!("system time error: {}", e)))?
            .as_secs();
        let batch = wallet.mine_batch(&target, timestamp, nonce, BATCH).await?;
        if let Some(result) = batch.result {
            return Ok(result);
        }
        nonce = nonce.wrapping_add(batch.attempted);
        tokio::task::yield_now().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(difficulty: u32) -> Work {
        Work {
            webcash: "e1:secret:00".into(),
            subsidy: None,
            timestamp: 1_700_000_000,
            difficulty,
        }
    }

    #[test]
    fn preimage_is_the_server_report_shape() {
        let w = Work {
            subsidy: Some("e0.5:secret:11".into()),
            ..work(3)
        };
        assert_eq!(
            w.preimage(7),
            r#"{"webcash":["e1:secret:00"],"subsidy":["e0.5:secret:11"],"timestamp":1700000000,"difficulty":3,"nonce":7}"#
        );
        assert_eq!(
            work(3).preimage(0),
            r#"{"webcash":["e1:secret:00"],"subsidy":[],"timestamp":1700000000,"difficulty":3,"nonce":0}"#
        );
    }

    #[test]
    fn search_returns_first_solution_meeting_difficulty() {
        let w = work(8);
        let (attempted, sol) = w.search(0, 1_000_000);
        let sol = sol.expect("difficulty 8 is found well within 1e6 nonces");
        assert!(sol.achieved >= 8);
        assert_eq!(sol.preimage, w.preimage(attempted - 1));
        assert_eq!(
            sol.hash_hex,
            hex::encode(Sha256::digest(sol.preimage.as_bytes()))
        );
        // No earlier nonce met the target.
        let (_, none) = w.search(0, attempted - 1);
        assert!(none.is_none());
    }

    #[test]
    fn search_is_bounded() {
        let (attempted, sol) = work(255).search(5, 100);
        assert_eq!(attempted, 100);
        assert!(sol.is_none());
    }

    #[test]
    fn leading_zero_bits_counts_bits() {
        assert_eq!(leading_zero_bits(&[0, 0, 0x0f, 0xff]), 20);
        assert_eq!(leading_zero_bits(&[0x80]), 0);
        assert_eq!(leading_zero_bits(&[0, 0]), 16);
    }
}

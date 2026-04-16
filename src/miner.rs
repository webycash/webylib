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

use crate::error::{Error, Result};
use crate::hd::ChainCode;
use crate::server::{Legalese, MiningReportRequest};
use crate::wallet::Wallet;
use crate::webcash::SecretWebcash;
use crate::Amount;

/// Result of a successful mining operation.
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

/// Count leading zero bits in a SHA256 hash.
fn leading_zero_bits(hash: &[u8]) -> u32 {
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

/// Mine webcash using the wallet's server client and HD derivation.
///
/// 1. Fetches current mining target from the server.
/// 2. Derives a mining secret from the wallet's HD chain.
/// 3. Iterates SHA256 hashes until a solution meets the difficulty target.
/// 4. Submits the mining report to the server.
/// 5. Stores the mined webcash in the wallet.
///
/// This is a light CPU miner — not optimized, intended for testnet only.
pub async fn mine(wallet: &Wallet) -> Result<MineResult> {
    // 1. Get mining target from server
    let target = {
        let server = wallet.server_client.lock().await;
        server.get_target().await?
    };

    let difficulty = target.difficulty_target_bits;
    let mining_amount_str = &target.mining_amount;
    let subsidy_amount_str = &target.mining_subsidy_amount;

    let mining_amount_f64: f64 = mining_amount_str
        .parse()
        .map_err(|_| Error::wallet(format!("invalid mining amount: {}", mining_amount_str)))?;
    let mining_amount = Amount::from_webcash(mining_amount_f64)?;

    // 2. Derive mining secrets from HD wallet
    let (mining_secret_hex, _mining_depth) = wallet.derive_next_secret(ChainCode::Mining)?;
    let webcash_str = format!("e{}:secret:{}", mining_amount_str, mining_secret_hex);

    // Derive subsidy secret if subsidy > 0
    let subsidy_amount_f64: f64 = subsidy_amount_str.parse().unwrap_or(0.0);
    let subsidy_amount = Amount::from_webcash(subsidy_amount_f64)?;
    let subsidy_str = if !subsidy_amount.is_zero() {
        let (subsidy_secret_hex, _) = wallet.derive_next_secret(ChainCode::Mining)?;
        Some(format!(
            "e{}:secret:{}",
            subsidy_amount_str, subsidy_secret_hex
        ))
    } else {
        None
    };

    // 3. Mine: iterate timestamps until SHA256(preimage) has enough leading zeros
    let mut nonce: u64 = 0;
    let preimage_str;
    let hash_hex;

    loop {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| Error::wallet(format!("system time error: {}", e)))?
            .as_secs();

        // Build preimage JSON
        let subsidy_array = match &subsidy_str {
            Some(s) => format!(r#"["{}"]"#, s),
            None => "[]".to_string(),
        };

        let candidate = format!(
            r#"{{"webcash":["{}"],"subsidy":{},"timestamp":{},"difficulty":{},"nonce":{}}}"#,
            webcash_str, subsidy_array, timestamp, difficulty, nonce
        );

        let hash = Sha256::digest(candidate.as_bytes());
        if leading_zero_bits(&hash) >= difficulty {
            preimage_str = candidate;
            hash_hex = hex::encode(hash);
            break;
        }

        nonce += 1;

        // Yield to runtime periodically to avoid blocking
        if nonce % 10_000 == 0 {
            tokio::task::yield_now().await;
        }
    }

    // 4. Submit mining report to server
    let report = MiningReportRequest {
        preimage: preimage_str.clone(),
        legalese: Legalese { terms: true },
    };

    {
        let server = wallet.server_client.lock().await;
        server.submit_mining_report(&report).await?;
    }

    // 5. Store the mined webcash directly (no /replace needed — we own the HD-derived secret)
    let secret = SecretWebcash::parse(&webcash_str)?;
    wallet.store_directly(secret).await?;

    Ok(MineResult {
        webcash: webcash_str,
        subsidy: subsidy_str,
        preimage: preimage_str,
        hash: hash_hex,
        difficulty,
        amount: mining_amount,
    })
}

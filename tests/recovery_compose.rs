//! Recovery scenarios — wallet wiped, seed retained, server queried.
//!
//! Targets the docker-compose webcash flavor on :8181 (DynamoDB-backed).
//! Each scenario:
//!   1. Open a wallet with a fresh, unique HD seed.
//!   2. Mine / pay / burn against the server using that seed's HD chain.
//!   3. Wipe the local SQLite (keeping ONLY the seed).
//!   4. Reopen wallet, restore the same seed, call `recover(seed, gap_limit)`.
//!   5. Assert the recovered set matches expectation.
//!
//! Skips quietly when the compose isn't reachable. Each fixture uses a
//! fresh unique seed so tests are isolated against the persistent server.
//!
//! The legacy `Wallet::recover` is the only recovery implementation in the
//! repo today (the new asset-trait architecture has no recovery op yet) —
//! these tests pin its behaviour and surface the bugs the user has hit
//! historically.

#![cfg(feature = "native")]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use std::str::FromStr;
use tempfile::TempDir;
use webylib::amount::Amount;
use webylib::hd::{ChainCode, HDWallet};
use webylib::server::NetworkMode;
use webylib::Wallet;

const PORT: u16 = 8181;

fn server_reachable() -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{PORT}").parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

fn server_url() -> String {
    format!("http://127.0.0.1:{PORT}")
}

fn leading_zero_bits(hash: &[u8]) -> u32 {
    let zb = hash.iter().take_while(|&&b| b == 0).count() as u32;
    hash.get(zb as usize).map_or(0, |b| b.leading_zeros()) + zb * 8
}

fn find_pow(template_with_marker: &str, bits: u32) -> String {
    for nonce in 0..1_000_000u64 {
        let p = template_with_marker.replace("__N__", &nonce.to_string());
        if leading_zero_bits(&Sha256::digest(p.as_bytes())) >= bits {
            return p;
        }
    }
    panic!("could not satisfy difficulty {bits}");
}

/// One scenario's isolated state. The seed is unique per fixture so two
/// scenarios running back-to-back against the same server do not collide
/// at the same HD depths.
struct RecoveryFixture {
    seed_hex: String,
    seed: [u8; 32],
    dir: TempDir,
}

impl RecoveryFixture {
    fn fresh_unique(salt: u8) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut seed = [0u8; 32];
        seed[0] = salt;
        seed[1..17].copy_from_slice(&nanos.to_be_bytes());
        // Mix in a per-process counter so tests in the same nanosecond differ.
        for (i, b) in seed.iter_mut().enumerate().skip(17) {
            *b = (i as u8).wrapping_mul(salt).wrapping_add(salt);
        }
        Self {
            seed_hex: hex::encode(seed),
            seed,
            dir: TempDir::new().expect("tempdir"),
        }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.path().join("wallet.db")
    }

    async fn open_wallet(&self) -> Wallet {
        let w = Wallet::open_with_network(self.db_path(), NetworkMode::Custom(server_url()))
            .await
            .expect("open wallet");
        w.store_master_secret(&self.seed_hex)
            .await
            .expect("store seed");
        w
    }

    /// Wipe the SQLite file, reopen with the same seed.
    async fn wipe_keep_seed(&self) -> Wallet {
        std::fs::remove_file(self.db_path()).ok();
        // tempdir survives the file delete — recreate fresh DB at same path.
        self.open_wallet().await
    }

    fn hd(&self) -> HDWallet {
        HDWallet::from_master_secret(self.seed)
    }
}

/// Mine one token at MINING:depth using the fixture's seed. Submits to the
/// compose webcash server via /api/v1/mining_report. Does NOT touch the
/// wallet's local store — the whole point of recovery is to re-discover
/// these tokens from the server.
fn mine_at_depth(fixture: &RecoveryFixture, depth: u64, amount: &str, subsidy: &str) {
    use webylib::server_client::Client;

    let secret = fixture
        .hd()
        .derive_secret(ChainCode::Mining, depth);
    // Subsidy secret must also be unique; derive a deterministic one from
    // the mining secret so re-runs against the same server collide cleanly
    // (the server rejects duplicate subsidy hashes).
    let mut h = Sha256::new();
    h.update(b"subsidy:");
    h.update(secret.as_bytes());
    let subsidy_secret = hex::encode(h.finalize());

    let template = format!(
        r#"{{"webcash":["e{amount}:secret:{secret}"],"subsidy":["e{subsidy}:secret:{subsidy_secret}"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_pow(&template, 4);

    let client = Client::new(server_url());
    client.mining_report(&preimage).expect("mining_report");
}

/// Helper: how many unspent tokens does the wallet hold locally?
async fn unspent_count(w: &Wallet) -> usize {
    w.list_webcash().await.expect("list_webcash").len()
}

async fn balance_wats(w: &Wallet) -> i64 {
    w.balance_amount().await.expect("balance").wats
}

// ─────────────────────────────────────────────────────────────────────
// P0 scenarios
// ─────────────────────────────────────────────────────────────────────

/// Empty seed: nothing was ever mined under this seed. Recovery must
/// terminate with `recovered_count == 0` and not error out.
#[tokio::test]
async fn recover_empty_seed_yields_zero() {
    if !server_reachable() {
        eprintln!("skipping: compose webcash :8181 not reachable");
        return;
    }
    let f = RecoveryFixture::fresh_unique(0xa0);
    let w = f.open_wallet().await;
    let r = w.recover(&f.seed_hex, 5).await.expect("recover");
    assert_eq!(r.recovered_count, 0, "fresh seed must recover 0 tokens");
    assert_eq!(r.total_amount.wats, 0);
    assert_eq!(unspent_count(&w).await, 0);
}

/// Mine N tokens at MINING:0..N-1, wipe local store (keep seed), recover.
/// All N must come back as unspent at the right amount.
#[tokio::test]
async fn recover_after_mining_finds_all_outputs() {
    if !server_reachable() {
        eprintln!("skipping: compose webcash :8181 not reachable");
        return;
    }
    let f = RecoveryFixture::fresh_unique(0xa1);
    let _ = f.open_wallet().await;

    const N: u64 = 3;
    for d in 0..N {
        mine_at_depth(&f, d, "1.0", "0.5");
    }

    let w = f.wipe_keep_seed().await;
    assert_eq!(unspent_count(&w).await, 0, "wipe must clear local store");

    let r = w.recover(&f.seed_hex, 5).await.expect("recover");
    assert_eq!(
        r.recovered_count as u64, N,
        "must recover all mined outputs"
    );
    let total = balance_wats(&w).await;
    let expected_per_token = Amount::from_str("1.0").unwrap().wats;
    assert_eq!(
        total,
        expected_per_token * N as i64,
        "balance must equal sum of mined amounts (server-reported)"
    );
}

/// Mine 1 → pay/split into PAY:0 + CHANGE:0 → wipe → recover.
/// The user's historical pain point ("recovery after mining the way and
/// where are inserted"): the original MINING:0 must register as spent on
/// the server, while PAY:0 and CHANGE:0 must come back unspent.
#[tokio::test]
async fn recover_after_mine_then_pay_finds_pay_and_change() {
    if !server_reachable() {
        eprintln!("skipping");
        return;
    }
    let f = RecoveryFixture::fresh_unique(0xa2);
    let w = f.open_wallet().await;

    mine_at_depth(&f, 0, "1.0", "0.5");

    // Pull the mined token into local store via recovery so we have funds
    // to pay from. (Mining stores it on the server only.)
    let r0 = w.recover(&f.seed_hex, 5).await.expect("seed recover");
    assert_eq!(
        r0.recovered_count, 1,
        "expected to find the one mined token"
    );

    // Pay 0.4 → recipient (PAY:0), 0.6 → CHANGE:0
    let pay_amount = Amount::from_str("0.4").unwrap();
    w.pay(pay_amount, "split test").await.expect("pay");

    let w2 = f.wipe_keep_seed().await;
    let r = w2.recover(&f.seed_hex, 5).await.expect("post-pay recover");
    // PAY:0 + CHANGE:0 are unspent on the server. MINING:0 is spent
    // (consumed as the input). Recovery walks all 4 chains and only stores
    // unspent outputs. Total balance must equal 1.0 webcash.
    assert_eq!(
        r.recovered_count, 2,
        "pay+change must surface as 2 unspent outputs"
    );
    assert_eq!(
        balance_wats(&w2).await,
        Amount::from_str("1.0").unwrap().wats,
        "post-recovery balance must equal pre-pay total (no funds lost)"
    );
}

/// Mine 3 → manually burn 1 via /api/v1/burn → wipe → recover.
/// The burned token must be silently skipped (server reports spent=true);
/// the other 2 must come back.
#[tokio::test]
async fn recover_skips_burned_outputs() {
    if !server_reachable() {
        eprintln!("skipping");
        return;
    }
    use webylib::server_client::Client;

    let f = RecoveryFixture::fresh_unique(0xa3);
    let _ = f.open_wallet().await;

    for d in 0..3u64 {
        mine_at_depth(&f, d, "1.0", "0.5");
    }

    // Burn MINING:1 directly via the server.
    let burn_secret = f.hd().derive_secret(ChainCode::Mining, 1);
    let burn_token = format!("e1.0:secret:{burn_secret}");
    let client = Client::new(server_url());
    client.burn(&burn_token).expect("burn");

    let w = f.wipe_keep_seed().await;
    let r = w.recover(&f.seed_hex, 5).await.expect("recover");
    assert_eq!(
        r.recovered_count, 2,
        "burned token must NOT appear in recovered set"
    );
    assert_eq!(
        balance_wats(&w).await,
        Amount::from_str("2.0").unwrap().wats,
        "balance must equal sum of unburned tokens"
    );
}

/// Transient server errors must NOT be silently absorbed: prior behaviour
/// (`consecutive_empty += gap_limit` on `Err`) made recovery appear to
/// finish with `recovered_count = 0` even when the server was unreachable.
/// That's exactly the "many bugs in recovery" the user reported.
///
/// Pointing the wallet at a closed port forces every `/health_check` call
/// to fail. Recovery must surface a loud error rather than return Ok with
/// a zero count.
#[tokio::test]
async fn recover_propagates_server_errors() {
    if !server_reachable() {
        eprintln!("skipping");
        return;
    }
    // Pick a port that nothing is listening on. Connect should fail fast
    // and reqwest will return a transport error.
    let bad_url = "http://127.0.0.1:1".to_string();
    let dir = TempDir::new().unwrap();
    let seed = [0xa5u8; 32];
    let seed_hex = hex::encode(seed);

    let w = Wallet::open_with_network(dir.path().join("w.db"), NetworkMode::Custom(bad_url))
        .await
        .expect("open wallet");
    w.store_master_secret(&seed_hex).await.expect("seed");

    let res = w.recover(&seed_hex, 5).await;
    assert!(
        res.is_err(),
        "recover must error when /health_check is unreachable, got Ok({:?})",
        res.ok()
    );
}

/// Two consecutive recoveries against the same wallet must yield the
/// same final state — no duplicate-store errors, no double-counting.
/// The legacy code matches "UNIQUE constraint" / "already exists" by
/// substring; if the SQL error string ever shifts, this test fails.
#[tokio::test]
async fn recover_is_idempotent() {
    if !server_reachable() {
        eprintln!("skipping");
        return;
    }
    let f = RecoveryFixture::fresh_unique(0xa4);
    let _ = f.open_wallet().await;

    for d in 0..2u64 {
        mine_at_depth(&f, d, "1.0", "0.5");
    }

    let w = f.wipe_keep_seed().await;
    let r1 = w.recover(&f.seed_hex, 5).await.expect("first recover");
    assert_eq!(r1.recovered_count, 2);
    let bal1 = balance_wats(&w).await;

    // Run recover again — must NOT error and must NOT duplicate balance.
    let _r2 = w
        .recover(&f.seed_hex, 5)
        .await
        .expect("second recover must not error");
    let bal2 = balance_wats(&w).await;
    assert_eq!(bal1, bal2, "second recovery must not change balance");
}

//! Integration test: webylib ServerClient against a live `server-webcash`
//! binary backed by Docker'd Redis.
//!
//! Runs only when:
//!   1. Docker is available, AND
//!   2. The sister webycash-server workspace has built `server-webcash`
//!      (`cargo build -p webycash-server-webcash` in
//!      ../webycash-server). Otherwise the test prints a skip message
//!      and exits cleanly so offline CI stays green.
//!
//! Exercises:
//!   - `ServerClient::get_target`
//!   - `ServerClient::submit_mining_report` (with a PoW preimage at d=4)
//!   - `ServerClient::health_check` (mined → unspent)
//!   - `ServerClient::replace` (split 1.0 → 0.4 + 0.6)
//!   - `ServerClient::health_check` (input spent / outputs unspent)
//!
//! This is the wallet-side analog of webycash-server's conformance suite:
//! it proves the existing webylib client speaks the protocol that the new
//! generic server-webcash binary serves.

#![cfg(feature = "native")]

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};
use webylib::server::{
    Legalese, MiningReportRequest, NetworkMode, ReplaceRequest, ServerClient, ServerConfig,
};
use webylib::webcash::{PublicWebcash, SecretWebcash};
use webylib::Amount;

fn server_binary_path() -> std::path::PathBuf {
    // webylib repo is at e.g. /Users/foo/workspace/webycash/webylib.
    // Sister webycash-server repo is at /Users/foo/workspace/webycash/webycash-server.
    // Go up ONE level from CARGO_MANIFEST_DIR to land in the webycash umbrella.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    std::path::PathBuf::from(&manifest_dir)
        .parent()
        .expect("manifest dir parent")
        .join("webycash-server")
        .join("target")
        .join("debug")
        .join("webycash-server-webcash")
}

fn pick_port() -> u16 {
    let s = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let p = s.local_addr().expect("addr").port();
    drop(s);
    p
}

fn docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

struct Stack {
    server: Child,
    redis_name: String,
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
        let _ = Command::new("docker")
            .args(["stop", &self.redis_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn start_stack(server_port: u16) -> Option<Stack> {
    if !docker_available() {
        eprintln!("Docker not available");
        return None;
    }
    let bin = server_binary_path();
    if !bin.exists() {
        eprintln!("server binary not found at {}", bin.display());
        return None;
    }
    eprintln!("starting stack: bin={} port={}", bin.display(), server_port);
    let redis_port = pick_port();
    let redis_name = format!(
        "webylib-conf-redis-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let s = Command::new("docker")
        .args([
            "run",
            "-d",
            "--rm",
            "--name",
            &redis_name,
            "-p",
            &format!("{redis_port}:6379"),
            "redis:7-alpine",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !s.success() {
        return None;
    }
    // Wait for Redis to be reachable.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", redis_port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let bind = format!("127.0.0.1:{server_port}");
    let server = Command::new(&bin)
        .env("WEBCASH_BIND_ADDR", &bind)
        .env("WEBCASH_MODE", "testnet")
        .env("WEBYCASH_DIFFICULTY", "4")
        .env("REDIS_URL", format!("redis://127.0.0.1:{redis_port}"))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(&bind).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Some(Stack { server, redis_name })
}

fn find_preimage_at_difficulty(template_with_marker: &str, bits: u32) -> String {
    for nonce in 0..200_000u64 {
        let preimage = template_with_marker.replace("__N__", &nonce.to_string());
        let lz = leading_zero_bits(&Sha256::digest(preimage.as_bytes()));
        if lz >= bits {
            return preimage;
        }
    }
    panic!("could not satisfy difficulty");
}

fn leading_zero_bits(hash: &[u8]) -> u32 {
    let full_zero_bytes = hash.iter().take_while(|&&b| b == 0).count() as u32;
    hash.get(full_zero_bytes as usize)
        .map_or(0, |b| b.leading_zeros())
        + full_zero_bytes * 8
}

#[tokio::test]
async fn webylib_speaks_to_server_webcash() {
    let port = pick_port();
    let Some(_stack) = start_stack(port) else {
        eprintln!(
            "skipping: needs Docker AND `cargo build -p webycash-server-webcash` in ../webycash-server"
        );
        return;
    };

    let client = ServerClient::with_config(ServerConfig {
        network: NetworkMode::Custom(format!("http://127.0.0.1:{port}")),
        timeout_seconds: 10,
    })
    .expect("client");

    // 1. /api/v1/target
    let target = client.get_target().await.expect("target");
    assert_eq!(target.difficulty_target_bits, 4);

    // 2. /api/v1/mining_report — mine a 1.0 webcash + 0.5 subsidy.
    let secret_hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let template = format!(
        r#"{{"webcash":["e1.0:secret:{secret_hex}"],"subsidy":["e0.5:secret:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],"timestamp":1714003200,"difficulty":4,"nonce":__N__}}"#
    );
    let preimage = find_preimage_at_difficulty(&template, 4);
    let mr = client
        .submit_mining_report(&MiningReportRequest {
            preimage,
            legalese: Legalese { terms: true },
        })
        .await
        .expect("mine");
    assert_eq!(mr.status, "success");

    // 3. /api/v1/health_check — mined hash must be unspent.
    let secret = SecretWebcash::parse(&format!("e1.0:secret:{secret_hex}")).expect("secret");
    let public = secret.to_public();
    let hc = client.health_check(std::slice::from_ref(&public)).await.expect("hc");
    assert_eq!(hc.status, "success");
    let key = format!("e1:public:{}", hex::encode(public.hash));
    let result = hc.results.get(&key).unwrap_or_else(|| {
        panic!("missing key {key} in {:?}", hc.results.keys().collect::<Vec<_>>())
    });
    assert_eq!(result.spent, Some(false), "expected unspent");

    // 4. /api/v1/replace — split 1.0 → 0.4 + 0.6 via webylib's client.
    let out1_hex = "1111111111111111111111111111111111111111111111111111111111111111";
    let out2_hex = "2222222222222222222222222222222222222222222222222222222222222222";
    let resp = client
        .replace(&ReplaceRequest {
            webcashes: vec![format!("e1.0:secret:{secret_hex}")],
            new_webcashes: vec![
                format!("e0.4:secret:{out1_hex}"),
                format!("e0.6:secret:{out2_hex}"),
            ],
            legalese: Legalese { terms: true },
        })
        .await
        .expect("replace");
    assert_eq!(resp.status, "success");

    // 5. health_check post-replace.
    let amount04 = Amount::from_str("0.4").unwrap();
    let amount06 = Amount::from_str("0.6").unwrap();
    let mut h_in = [0u8; 32];
    h_in.copy_from_slice(&Sha256::digest(secret_hex.as_bytes()));
    let mut h_out1 = [0u8; 32];
    h_out1.copy_from_slice(&Sha256::digest(out1_hex.as_bytes()));
    let mut h_out2 = [0u8; 32];
    h_out2.copy_from_slice(&Sha256::digest(out2_hex.as_bytes()));
    let pub_in = PublicWebcash::new(h_in, Amount::from_str("1.0").unwrap());
    let pub_out1 = PublicWebcash::new(h_out1, amount04);
    let pub_out2 = PublicWebcash::new(h_out2, amount06);
    let hc2 = client
        .health_check(&[pub_in.clone(), pub_out1.clone(), pub_out2.clone()])
        .await
        .expect("hc2");
    let key_in = format!("e1:public:{}", hex::encode(pub_in.hash));
    let key_out1 = format!("e0.4:public:{}", hex::encode(pub_out1.hash));
    let key_out2 = format!("e0.6:public:{}", hex::encode(pub_out2.hash));
    assert_eq!(hc2.results[&key_in].spent, Some(true));
    assert_eq!(hc2.results[&key_out1].spent, Some(false));
    assert_eq!(hc2.results[&key_out2].spent, Some(false));
}

// Small helper for Amount::from_str inside test (Amount doesn't expose FromStr publicly).
trait AmountFromStr {
    fn from_str(s: &str) -> std::result::Result<Amount, String>;
}
impl AmountFromStr for Amount {
    fn from_str(s: &str) -> std::result::Result<Amount, String> {
        <Amount as std::str::FromStr>::from_str(s).map_err(|e| e.to_string())
    }
}

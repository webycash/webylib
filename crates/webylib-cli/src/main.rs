//! `webyc` — command-line wallet driving the asset-flavored
//! `Wallet<A: Asset>` crates against a running webycash-server flavor.
//!
//! The server is a single-use-seal registry — every wallet verb maps
//! to its `/api/v1/replace` endpoint, with semantics differing only in
//! how the wallet shapes its inputs and outputs.
//!
//! ```text
//!   webyc webcash pay      → /api/v1/replace (any arity, conserves)
//!   webyc webcash insert   → /api/v1/replace (1:1)
//!   webyc rgb transfer     → /api/v1/replace (any arity, namespaced)
//!   webyc rgb insert       → /api/v1/replace (1:1)
//!   webyc voucher pay      → /api/v1/replace (any arity, conserves)
//!   webyc voucher insert   → /api/v1/replace (1:1)
//! ```

use anyhow::{anyhow, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use webylib_server_client::Client;
use webylib_wallet_rgb::RgbWallet;
use webylib_wallet_voucher::VoucherWallet;
use webylib_wallet_webcash::WebcashWallet;

#[derive(Parser, Debug)]
#[command(
    name = "webyca",
    version,
    about = "Webycash multi-asset wallet CLI",
    long_about = None,
)]
struct Cli {
    /// Server base URL (e.g. `http://127.0.0.1:8181`). Per-flavor.
    /// Defaults to `WEBCASH_SERVER_URL` env var if set.
    #[arg(long, env = "WEBCASH_SERVER_URL", global = true)]
    server: Option<String>,

    #[command(subcommand)]
    flavor: Flavor,
}

#[derive(Subcommand, Debug)]
enum Flavor {
    /// Webcash (frozen wire format, no namespace, mining-only issuance)
    Webcash {
        #[command(subcommand)]
        cmd: WebcashCmd,
    },
    /// RGB20 fungible / RGB21 collectible (issuer-namespaced)
    Rgb {
        #[command(subcommand)]
        cmd: RgbCmd,
    },
    /// Voucher: always-splittable, issuer-private, issuer-namespaced
    Voucher {
        #[command(subcommand)]
        cmd: VoucherCmd,
    },
    /// Read-only: print the server's mining target (difficulty, ratios).
    /// Flavor-agnostic — works against any of the four binaries.
    Target,
    /// Read-only: print the server's economy statistics
    /// (total circulation, mining report count, current epoch,
    /// difficulty, mining/subsidy amounts).
    Stats,
    /// Read-only: check whether one or more public tokens are spent.
    /// Flavor-agnostic — wire format depends on the server you point at.
    Check {
        /// Comma-separated public tokens, e.g.
        /// `e1.0:public:HASH` (webcash) or
        /// `e10.0:public:HASH:contract:fingerprint` (rgb / voucher).
        #[arg(long, value_delimiter = ',')]
        tokens: Vec<String>,
    },
    /// Permanently destroy a single secret. The server marks it spent
    /// without minting a replacement — there is no recovery.
    Burn {
        /// The secret token to destroy, in the asset's wire format.
        #[arg(long)]
        secret: String,
    },
    /// Submit a PoW preimage to /api/v1/mining_report. Caller is
    /// responsible for finding a preimage that satisfies the current
    /// difficulty target — see `webyca target`.
    MiningReport {
        /// The preimage string (raw JSON or base64 — the server
        /// auto-detects).
        #[arg(long)]
        preimage: String,
    },
    /// Convert a secret token to its public form locally — no server
    /// contact required. The hash is `sha256(secret_hex_bytes)` for
    /// every flavor; namespace fields (contract, issuer fingerprint)
    /// are preserved as-is.
    DerivePublic {
        /// Secret token in the asset's wire form, e.g.
        /// `e1.0:secret:HEX64` (webcash),
        /// `e10.0:secret:HEX64:contract:fingerprint` (rgb / voucher),
        /// or `secret:HEX64:contract:fingerprint` (rgb collectible).
        #[arg(long)]
        secret: String,
    },
    /// Local-only: verify that a `secret` token derives the given
    /// `public` token. Exits 0 on match, 2 on mismatch. Catches
    /// hash-mismatch / namespace-drift mistakes BEFORE submitting
    /// to a server.
    Verify {
        #[arg(long)]
        secret: String,
        #[arg(long)]
        public: String,
    },
    /// Print a shell completion script to stdout. Pipe into your
    /// shell's completion path, e.g.
    ///
    ///   webyca completions bash > ~/.local/share/bash-completion/completions/webyca
    ///   webyca completions zsh  > ~/.zsh/completions/_webyca
    ///   webyca completions fish > ~/.config/fish/completions/webyca.fish
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand, Debug)]
enum WebcashCmd {
    /// Split N inputs into M outputs. Server enforces conservation.
    Pay {
        /// Comma-separated input secrets: `e{amt}:secret:{hex},...`
        #[arg(long, value_delimiter = ',')]
        inputs: Vec<String>,
        /// Comma-separated output secrets (recipient + change).
        #[arg(long, value_delimiter = ',')]
        outputs: Vec<String>,
    },
    /// Receive a webcash secret and immediately rotate to a wallet-owned secret.
    Insert {
        /// The secret you just received from someone else.
        #[arg(long)]
        received: String,
        /// Your new wallet-owned secret of the same amount.
        #[arg(long)]
        rotate_to: String,
    },
}

#[derive(Subcommand, Debug)]
enum RgbCmd {
    /// Transfer ownership. RGB20: any arity, conservation enforced.
    /// RGB21: 1:1 with amount segment dropped.
    Transfer {
        #[arg(long, value_delimiter = ',')]
        inputs: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        outputs: Vec<String>,
    },
    /// Receive an RGB secret and rotate to a wallet-owned one (same namespace).
    Insert {
        #[arg(long)]
        received: String,
        #[arg(long)]
        rotate_to: String,
    },
}

#[derive(Subcommand, Debug)]
enum VoucherCmd {
    /// Split / spend voucher units. Always splittable.
    Pay {
        #[arg(long, value_delimiter = ',')]
        inputs: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        outputs: Vec<String>,
    },
    /// Receive a voucher secret and rotate to a wallet-owned one.
    Insert {
        #[arg(long)]
        received: String,
        #[arg(long)]
        rotate_to: String,
    },
}

fn require_server(cli: &Cli) -> Result<&str> {
    cli.server
        .as_deref()
        .ok_or_else(|| anyhow!("--server URL required (or set WEBCASH_SERVER_URL)"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Local-only verbs don't require --server.
    match &cli.flavor {
        Flavor::DerivePublic { secret } => return run_derive_public(secret),
        Flavor::Verify { secret, public } => return run_verify(secret, public),
        Flavor::Completions { shell } => return run_completions(*shell),
        _ => {}
    }
    let server = require_server(&cli)?.to_string();
    match cli.flavor {
        Flavor::Webcash { cmd } => run_webcash(&server, cmd),
        Flavor::Rgb { cmd } => run_rgb(&server, cmd),
        Flavor::Voucher { cmd } => run_voucher(&server, cmd),
        Flavor::Target => run_target(&server),
        Flavor::Stats => run_stats(&server),
        Flavor::Check { tokens } => run_check(&server, tokens),
        Flavor::Burn { secret } => run_burn(&server, &secret),
        Flavor::MiningReport { preimage } => run_mining_report(&server, &preimage),
        Flavor::DerivePublic { .. }
        | Flavor::Verify { .. }
        | Flavor::Completions { .. } => unreachable!("handled above"),
    }
}

fn run_completions(shell: Shell) -> Result<()> {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}

/// Local-only: replace the `secret:HEX` segment with `public:SHA256`
/// while preserving any prefix (`e{amount}:`) and trailing namespace
/// (`:contract:fp`). Handles both forms:
///   - `e{amt}:secret:HEX[:contract:fp]` (webcash, RGB20, voucher)
///   - `secret:HEX[:contract:fp]` (RGB21 collectible)
fn derive_public_form(secret: &str) -> Result<String> {
    use sha2::{Digest, Sha256};

    // The `secret:` marker is either at the very start or preceded by
    // `:`. Anchor accordingly.
    let (prefix, after_marker) = if let Some(rest) = secret.strip_prefix("secret:") {
        ("", rest)
    } else if let Some(at) = secret.find(":secret:") {
        let after = &secret[at + ":secret:".len()..];
        (&secret[..at + 1], after) // prefix keeps the trailing `:` for re-emission
    } else {
        anyhow::bail!("token missing `secret:` segment");
    };

    // Hex segment is up to the next `:` or end of string.
    let (hex_seg, tail) = match after_marker.find(':') {
        Some(i) => (&after_marker[..i], &after_marker[i..]),
        None => (after_marker, ""),
    };
    if hex_seg.len() != 64 || !hex_seg.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("expected 64 hex chars after `secret:`, got {hex_seg:?}");
    }

    let public_hash = hex::encode(Sha256::digest(hex_seg.as_bytes()));
    Ok(format!("{prefix}public:{public_hash}{tail}"))
}

fn run_derive_public(secret: &str) -> Result<()> {
    println!("{}", derive_public_form(secret)?);
    Ok(())
}

/// Compare the secret's derived public form to the user-supplied one.
/// Exits 0 on byte-exact match, 2 on mismatch (so scripts can branch).
fn run_verify(secret: &str, public: &str) -> Result<()> {
    let derived = derive_public_form(secret).context("derive from secret")?;
    if derived == public {
        println!("ok: secret matches public");
        Ok(())
    } else {
        eprintln!("mismatch:\n  derived: {derived}\n  expected: {public}");
        std::process::exit(2);
    }
}

fn run_burn(server: &str, secret: &str) -> Result<()> {
    let client = Client::new(server.to_string());
    client.burn(secret).context("burn")?;
    println!("ok: burned");
    Ok(())
}

fn run_mining_report(server: &str, preimage: &str) -> Result<()> {
    let client = Client::new(server.to_string());
    client.mining_report(preimage).context("mining_report")?;
    println!("ok: mining_report accepted");
    Ok(())
}

fn run_target(server: &str) -> Result<()> {
    let client = Client::new(server.to_string());
    let body = client.target().context("target")?;
    println!("{body}");
    Ok(())
}

fn run_stats(server: &str) -> Result<()> {
    let client = Client::new(server.to_string());
    let body = client.stats().context("stats")?;
    println!("{body}");
    Ok(())
}

fn run_check(server: &str, tokens: Vec<String>) -> Result<()> {
    if tokens.is_empty() {
        anyhow::bail!("--tokens requires ≥1 public token");
    }
    let client = Client::new(server.to_string());
    let body = client.health_check(&tokens).context("health_check")?;
    println!("{body}");
    Ok(())
}

fn run_webcash(server: &str, cmd: WebcashCmd) -> Result<()> {
    let wallet = WebcashWallet::new(server.to_string());
    match cmd {
        WebcashCmd::Pay { inputs, outputs } => {
            wallet.pay(&inputs, &outputs).context("webcash pay")?;
            println!("ok: replaced {} → {}", inputs.len(), outputs.len());
        }
        WebcashCmd::Insert { received, rotate_to } => {
            wallet.insert(&received, &rotate_to).context("webcash insert")?;
            println!("ok: rotated received secret → wallet-owned");
        }
    }
    Ok(())
}

fn run_rgb(server: &str, cmd: RgbCmd) -> Result<()> {
    let wallet = RgbWallet::new(server.to_string());
    match cmd {
        RgbCmd::Transfer { inputs, outputs } => {
            wallet.transfer(&inputs, &outputs).context("rgb transfer")?;
            println!("ok: replaced {} → {}", inputs.len(), outputs.len());
        }
        RgbCmd::Insert { received, rotate_to } => {
            wallet.insert(&received, &rotate_to).context("rgb insert")?;
            println!("ok: rotated received secret → wallet-owned");
        }
    }
    Ok(())
}

fn run_voucher(server: &str, cmd: VoucherCmd) -> Result<()> {
    let wallet = VoucherWallet::new(server.to_string());
    match cmd {
        VoucherCmd::Pay { inputs, outputs } => {
            wallet.pay(&inputs, &outputs).context("voucher pay")?;
            println!("ok: replaced {} → {}", inputs.len(), outputs.len());
        }
        VoucherCmd::Insert { received, rotate_to } => {
            wallet.insert(&received, &rotate_to).context("voucher insert")?;
            println!("ok: rotated received secret → wallet-owned");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Catches breakage in derive proc-macros, conflicting flags, or
    /// option name collisions across subcommands.
    #[test]
    fn cli_definition_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn webcash_pay_parses_comma_separated_lists() {
        let cli = Cli::try_parse_from([
            "webyca",
            "--server", "http://x:8080",
            "webcash", "pay",
            "--inputs", "e1.0:secret:aaa,e2.0:secret:bbb",
            "--outputs", "e0.5:secret:ccc,e2.5:secret:ddd",
        ])
        .expect("parse");
        match cli.flavor {
            Flavor::Webcash {
                cmd: WebcashCmd::Pay { inputs, outputs },
            } => {
                assert_eq!(inputs.len(), 2);
                assert_eq!(outputs.len(), 2);
                assert_eq!(inputs[0], "e1.0:secret:aaa");
                assert_eq!(outputs[1], "e2.5:secret:ddd");
            }
            other => panic!("wrong arm: {:?}", other),
        }
        assert_eq!(cli.server.as_deref(), Some("http://x:8080"));
    }

    #[test]
    fn rgb_transfer_namespaced_token_format() {
        let token = "e10.0:secret:aaa:rgb20:fffeeeddd";
        let cli = Cli::try_parse_from([
            "webyca", "--server", "http://x", "rgb", "transfer",
            "--inputs", token,
            "--outputs", &format!("{token},e0.0:secret:zzz:rgb20:fffeeeddd"),
        ])
        .expect("parse");
        match cli.flavor {
            Flavor::Rgb { cmd: RgbCmd::Transfer { inputs, outputs } } => {
                assert_eq!(inputs, vec![token.to_string()]);
                assert_eq!(outputs.len(), 2);
            }
            _ => panic!("wrong arm"),
        }
    }

    #[test]
    fn voucher_insert_pair() {
        let cli = Cli::try_parse_from([
            "webyca", "--server", "http://x", "voucher", "insert",
            "--received", "e10:secret:r:c:f",
            "--rotate-to", "e10:secret:n:c:f",
        ])
        .expect("parse");
        match cli.flavor {
            Flavor::Voucher { cmd: VoucherCmd::Insert { received, rotate_to } } => {
                assert_eq!(received, "e10:secret:r:c:f");
                assert_eq!(rotate_to, "e10:secret:n:c:f");
            }
            _ => panic!("wrong arm"),
        }
    }

    /// Server flag is global — usable BEFORE or AFTER the subcommand.
    #[test]
    fn server_flag_works_after_subcommand() {
        let cli = Cli::try_parse_from([
            "webyca", "webcash", "pay",
            "--server", "http://post:8080",
            "--inputs", "x", "--outputs", "y",
        ])
        .expect("parse");
        assert_eq!(cli.server.as_deref(), Some("http://post:8080"));
    }

    #[test]
    fn missing_subcommand_is_an_error() {
        let err = Cli::try_parse_from(["webyca"]).expect_err("must require subcommand");
        // clap reports a "missing subcommand" / "MissingRequiredArgument"-shaped error.
        assert!(matches!(
            err.kind(),
            clap::error::ErrorKind::MissingSubcommand
                | clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        ));
    }

    #[test]
    fn target_subcommand_is_flavor_agnostic() {
        let cli = Cli::try_parse_from(["webyca", "--server", "http://x", "target"])
            .expect("parse");
        assert!(matches!(cli.flavor, Flavor::Target));
    }

    #[test]
    fn stats_subcommand_is_flavor_agnostic() {
        let cli = Cli::try_parse_from(["webyca", "--server", "http://x", "stats"])
            .expect("parse");
        assert!(matches!(cli.flavor, Flavor::Stats));
    }

    #[test]
    fn check_subcommand_takes_comma_separated_tokens() {
        let cli = Cli::try_parse_from([
            "webyca", "--server", "http://x", "check",
            "--tokens", "e1:public:aaa,e2:public:bbb",
        ])
        .expect("parse");
        match cli.flavor {
            Flavor::Check { tokens } => {
                assert_eq!(tokens.len(), 2);
                assert_eq!(tokens[0], "e1:public:aaa");
            }
            _ => panic!("wrong arm"),
        }
    }

    #[test]
    fn burn_subcommand_takes_single_secret() {
        let cli = Cli::try_parse_from([
            "webyca", "--server", "http://x", "burn",
            "--secret", "e1.0:secret:deadbeef",
        ])
        .expect("parse");
        match cli.flavor {
            Flavor::Burn { secret } => assert_eq!(secret, "e1.0:secret:deadbeef"),
            _ => panic!("wrong arm"),
        }
    }

    #[test]
    fn mining_report_subcommand_takes_preimage() {
        let cli = Cli::try_parse_from([
            "webyca", "--server", "http://x", "mining-report",
            "--preimage", "{\"webcash\":[],\"subsidy\":[],\"timestamp\":1,\"difficulty\":4,\"nonce\":0}",
        ])
        .expect("parse");
        match cli.flavor {
            Flavor::MiningReport { preimage } => assert!(preimage.contains("webcash")),
            _ => panic!("wrong arm"),
        }
    }

    /// `derive-public` works without --server (local-only computation).
    #[test]
    fn derive_public_does_not_require_server() {
        let cli = Cli::try_parse_from([
            "webyca", "derive-public",
            "--secret", "e1.0:secret:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .expect("parse");
        match cli.flavor {
            Flavor::DerivePublic { secret } => assert!(secret.contains(":secret:")),
            _ => panic!("wrong arm"),
        }
        assert!(cli.server.is_none(), "no --server required");
    }

    #[test]
    fn verify_does_not_require_server() {
        let cli = Cli::try_parse_from([
            "webyca", "verify",
            "--secret", "e1.0:secret:abc",
            "--public", "e1.0:public:def",
        ])
        .expect("parse");
        match cli.flavor {
            Flavor::Verify { secret, public } => {
                assert_eq!(secret, "e1.0:secret:abc");
                assert_eq!(public, "e1.0:public:def");
            }
            _ => panic!("wrong arm"),
        }
        assert!(cli.server.is_none(), "no --server required");
    }

    #[test]
    fn derive_public_form_webcash_matches_sha256() {
        use sha2::{Digest, Sha256};
        let secret = "a".repeat(64);
        let token = format!("e1.0:secret:{secret}");
        let derived = derive_public_form(&token).unwrap();
        let expected = format!(
            "e1.0:public:{}",
            hex::encode(Sha256::digest(secret.as_bytes()))
        );
        assert_eq!(derived, expected);
    }

    #[test]
    fn derive_public_form_rgb_namespaced_preserves_namespace() {
        let token = format!(
            "e10.0:secret:{}:rgb20-usdc:aabbccddeeff00112233445566778899aabbccdd",
            "b".repeat(64),
        );
        let derived = derive_public_form(&token).unwrap();
        assert!(derived.starts_with("e10.0:public:"));
        assert!(derived.ends_with(":rgb20-usdc:aabbccddeeff00112233445566778899aabbccdd"));
    }

    #[test]
    fn derive_public_form_collectible_no_amount_segment() {
        let token = format!(
            "secret:{}:rgb21-art:aabbccddeeff00112233445566778899aabbccdd",
            "c".repeat(64),
        );
        let derived = derive_public_form(&token).unwrap();
        assert!(derived.starts_with("public:"));
        assert!(!derived.starts_with("e"));
    }

    #[test]
    fn derive_public_form_rejects_no_secret_marker() {
        let err = derive_public_form("e1.0:public:abc").unwrap_err();
        assert!(err.to_string().contains("missing `secret:` segment"));
    }

    #[test]
    fn derive_public_form_rejects_short_hex() {
        let err = derive_public_form("e1.0:secret:short").unwrap_err();
        assert!(err.to_string().contains("64 hex chars"));
    }

    #[test]
    fn completions_subcommand_takes_shell_value() {
        let cli = Cli::try_parse_from(["webyca", "completions", "bash"])
            .expect("parse");
        match cli.flavor {
            Flavor::Completions { shell } => assert_eq!(shell, Shell::Bash),
            _ => panic!("wrong arm"),
        }
        assert!(cli.server.is_none(), "no --server required");
    }

    #[test]
    fn completions_supports_zsh_fish_powershell() {
        for shell_name in ["zsh", "fish", "powershell"] {
            let cli = Cli::try_parse_from(["webyca", "completions", shell_name])
                .expect("parse");
            assert!(matches!(cli.flavor, Flavor::Completions { .. }));
        }
    }

    #[test]
    fn unknown_flavor_is_rejected() {
        let err = Cli::try_parse_from(["webyca", "ethereum", "pay"])
            .expect_err("unknown flavor must reject");
        assert!(matches!(
            err.kind(),
            clap::error::ErrorKind::InvalidSubcommand
                | clap::error::ErrorKind::UnknownArgument
        ));
    }
}

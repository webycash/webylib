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
use clap::{Parser, Subcommand};

use webylib_wallet_rgb::RgbWallet;
use webylib_wallet_voucher::VoucherWallet;
use webylib_wallet_webcash::WebcashWallet;

#[derive(Parser, Debug)]
#[command(
    name = "webyc",
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
    let server = require_server(&cli)?.to_string();
    match cli.flavor {
        Flavor::Webcash { cmd } => run_webcash(&server, cmd),
        Flavor::Rgb { cmd } => run_rgb(&server, cmd),
        Flavor::Voucher { cmd } => run_voucher(&server, cmd),
    }
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

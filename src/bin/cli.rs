//! Webcash CLI - Command Line Interface for Webcash Wallet
//!
//! Webcash-only. For RGB / Voucher operations and a unified verb
//! surface across all four asset flavors, build the `webyca` binary
//! from `crates/webylib-cli` (`cargo build -p webylib-cli`).

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::str::FromStr;
use webylib::passkey::EncryptedData;
use webylib::{Amount, NetworkMode, SecretWebcash, Wallet};

#[derive(Clone, ValueEnum)]
enum Network {
    Production,
    Testnet,
}

#[derive(Parser)]
#[command(name = "webyc")]
#[command(about = "Webcash wallet command line interface (webcash-only — see `webyca` for multi-asset)")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Wallet database file path [default: ~/.webyc/wallet.db]
    #[arg(short, long)]
    wallet: Option<PathBuf>,

    /// Enable passkey authentication for encrypted wallets
    #[arg(long)]
    passkey: bool,

    /// Network to use (production or testnet)
    #[arg(short, long, default_value = "production")]
    network: Network,

    /// Custom server URL (overrides --network)
    #[arg(long)]
    server_url: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new wallet with master secret
    Setup {
        /// Optional master secret in hex format (64 characters) - if not provided, generates new one
        #[arg(short = 'p', long)]
        master_secret: Option<String>,
        /// Enable passkey encryption (Face ID/Touch ID on mobile)
        #[arg(long)]
        passkey: bool,
    },
    /// Show wallet information
    Info,
    /// Insert webcash into wallet
    Insert {
        /// Webcash to insert
        webcash: Option<String>,
        /// Optional memo
        #[arg(short, long)]
        memo: Option<String>,
        /// Skip server validation (offline mode)
        #[arg(long)]
        offline: bool,
    },
    /// Generate payment webcash
    Pay {
        /// Amount to pay
        amount: String,
        /// Optional memo
        #[arg(short, long)]
        memo: Option<String>,
    },
    /// Check wallet against server
    Check,
    /// Recover wallet from stored master secret
    Recover {
        /// Gap limit for recovery
        #[arg(long, default_value = "20")]
        gap_limit: usize,
    },
    /// Merge small outputs
    Merge {
        /// Maximum outputs to merge at once
        #[arg(long, default_value = "20")]
        group: usize,
        /// Maximum output size
        #[arg(long, default_value = "50000000")]
        max: String,
        /// Optional memo
        #[arg(long)]
        memo: Option<String>,
    },
    /// Mine webcash (testnet only — low difficulty)
    Mine,
    /// Encrypt wallet using passkey or password
    Encrypt {
        /// Output file for encrypted wallet
        #[arg(short, long)]
        output: PathBuf,
        /// Use password instead of passkey
        #[arg(long)]
        password: bool,
    },
    /// Decrypt wallet from encrypted file
    Decrypt {
        /// Input file containing encrypted wallet
        #[arg(short, long)]
        input: PathBuf,
        /// Use password instead of passkey
        #[arg(long)]
        password: bool,
    },
    /// Encrypt the wallet database file with password (for runtime use)
    EncryptDb {
        /// Use password instead of passkey authentication
        #[arg(long)]
        password: bool,
    },
    /// Decrypt the wallet database file (for runtime use)
    DecryptDb {
        /// Use password instead of passkey authentication
        #[arg(long)]
        password: bool,
    },
}

fn resolve_network(cli: &Cli) -> NetworkMode {
    if let Some(url) = &cli.server_url {
        return NetworkMode::Custom(url.clone());
    }
    match cli.network {
        Network::Production => NetworkMode::Production,
        Network::Testnet => NetworkMode::Testnet,
    }
}

async fn open_wallet_at(
    path: &std::path::Path,
    network: NetworkMode,
) -> Result<Wallet, Box<dyn std::error::Error>> {
    let wallet = Wallet::open_with_network(path, network).await?;
    Ok(wallet)
}

fn default_wallet_path() -> PathBuf {
    match dirs_next::home_dir() {
        Some(home) => home.join(".webyc").join("wallet.db"),
        None => {
            eprintln!("Cannot determine home directory. Use --wallet to specify a path.");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let network = resolve_network(&cli);
    let wallet_path = cli.wallet.clone().unwrap_or_else(default_wallet_path);
    println!("Network: {:?}", network);

    // Helper: open wallet with the resolved network for any command
    macro_rules! open_wallet {
        () => {
            open_wallet_at(&wallet_path, network.clone()).await
        };
    }

    match cli.command {
        Commands::Setup {
            master_secret,
            passkey,
        } => {
            println!("Setting up new wallet at: {}", wallet_path.display());

            // Generate or use provided master secret (now optional - wallet auto-generates)
            let explicit_master_secret = master_secret.is_some();
            let master_secret_hex = match master_secret {
                Some(secret) => {
                    println!("🎯 Using provided master secret: {}...", &secret[..8]);
                    secret
                }
                None => {
                    println!("🔑 Master secret will be auto-generated using hardware RNG");
                    // Return empty string - wallet will auto-generate
                    String::new()
                }
            };

            if passkey {
                println!("🔐 Passkey encryption enabled");
            }

            match open_wallet!() {
                Ok(wallet) => {
                    // Store the master secret only if explicitly provided
                    if explicit_master_secret {
                        match wallet.store_master_secret(&master_secret_hex).await {
                            Ok(()) => {
                                println!(
                                    "✅ Wallet created successfully with provided master secret!"
                                );
                            }
                            Err(e) => {
                                eprintln!("❌ Failed to store master secret: {}", e);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        println!(
                            "✅ Wallet created successfully with auto-generated master secret!"
                        );
                    }

                    let stats = wallet.stats().await?;
                    println!("📊 Wallet statistics:");
                    println!("  Total webcash: {}", stats.total_webcash);
                    println!("  Unspent webcash: {}", stats.unspent_webcash);
                    println!("  Balance: {}", stats.total_balance);
                    println!(
                        "  Passkey encryption: {}",
                        if wallet.is_passkey_enabled() {
                            "Enabled"
                        } else {
                            "Disabled"
                        }
                    );
                }
                Err(e) => {
                    eprintln!("❌ Failed to create wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Info => {
            println!("Wallet information for: {}", wallet_path.display());
            match open_wallet!() {
                Ok(wallet) => {
                    let balance = wallet.balance().await?;
                    let stats = wallet.stats().await?;
                    let webcash_list = wallet.list_webcash().await?;

                    println!("📊 Wallet Statistics:");
                    println!("  Balance: {} WEBCASH", balance);
                    println!("  Total entries: {}", stats.total_webcash);
                    println!("  Unspent entries: {}", stats.unspent_webcash);
                    println!("  Spent entries: {}", stats.spent_webcash);

                    if !webcash_list.is_empty() {
                        println!("\n💰 Unspent Webcash:");
                        for (i, webcash) in webcash_list.iter().enumerate() {
                            println!("  {}. {} ({})", i + 1, webcash, webcash.amount);
                        }
                    } else {
                        println!("\n💰 No unspent webcash in wallet");
                    }
                }
                Err(e) => {
                    eprintln!("❌ Failed to open wallet: {}", e);
                    eprintln!(
                        "💡 Try running 'webyc --wallet {} setup' to create a new wallet",
                        wallet_path.display()
                    );
                    std::process::exit(1);
                }
            }
        }
        Commands::Insert {
            webcash,
            memo,
            offline,
        } => {
            println!("Inserting webcash into wallet: {}", wallet_path.display());
            if let Some(memo) = memo {
                println!("Memo: {}", memo);
            }
            if offline {
                println!("🔄 Offline mode - skipping server validation");
            }

            // Get webcash from argument or environment variable
            let webcash_str = match webcash {
                Some(wc) => wc,
                None => {
                    eprintln!("❌ No webcash secret provided");
                    eprintln!("💡 Provide webcash secret as argument");
                    std::process::exit(1);
                }
            };

            // Parse the webcash string
            let secret_webcash = match SecretWebcash::parse(&webcash_str) {
                Ok(wc) => wc,
                Err(e) => {
                    eprintln!("❌ Invalid webcash format: {}", e);
                    eprintln!("💡 Expected format: e<amount>:<type>:<value>");
                    std::process::exit(1);
                }
            };

            match open_wallet!() {
                Ok(wallet) => {
                    // Match Python: insert does NOT validate before replace by default
                    // Only validate if explicitly requested (not the default behavior)
                    let validate_with_server = false; // Python doesn't validate before replace
                    match wallet
                        .insert_with_validation(secret_webcash.clone(), validate_with_server)
                        .await
                    {
                        Ok(()) => {
                            println!(
                                "✅ Successfully inserted webcash: {}",
                                secret_webcash.amount
                            );
                            let new_balance = wallet.balance().await?;
                            println!("📊 New balance: {} WEBCASH", new_balance);
                        }
                        Err(e) => {
                            eprintln!("❌ Failed to insert webcash: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("❌ Failed to open wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Pay { amount, memo } => {
            let memo_str = memo.as_deref().unwrap_or("Payment");
            println!(
                "Generating payment webcash for amount: {} with memo: '{}' from wallet: {}",
                amount,
                memo_str,
                wallet_path.display()
            );

            // Parse the amount
            let payment_amount = match Amount::from_str(&amount) {
                Ok(amt) => amt,
                Err(e) => {
                    eprintln!("❌ Invalid amount format: {}", e);
                    std::process::exit(1);
                }
            };

            match open_wallet!() {
                Ok(wallet) => match wallet.pay(payment_amount, memo_str).await {
                    Ok(message) => {
                        println!("✅ {}", message);
                        let new_balance = wallet
                            .balance()
                            .await
                            .unwrap_or_else(|_| "unknown".to_string());
                        println!("📊 New balance: {} WEBCASH", new_balance);
                    }
                    Err(e) => {
                        eprintln!("❌ Payment generation failed: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("❌ Failed to open wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Check => {
            println!("Checking wallet against server: {}", wallet_path.display());
            match open_wallet!() {
                Ok(wallet) => match wallet.check().await {
                    Ok(result) => {
                        println!("✅ Wallet check completed successfully");
                        println!("  Valid: {}", result.valid_count);
                        println!("  Spent: {}", result.spent_count);
                    }
                    Err(e) => {
                        eprintln!("❌ Wallet check failed: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("❌ Failed to open wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Recover { gap_limit } => {
            println!(
                "Recovering wallet with gap limit: {} for wallet: {}",
                gap_limit,
                wallet_path.display()
            );

            match open_wallet!() {
                Ok(wallet) => match wallet.recover_from_wallet(gap_limit).await {
                    Ok(summary) => {
                        println!("✅ Recovery completed successfully");
                        println!("{}", summary);
                    }
                    Err(e) => {
                        eprintln!("❌ Recovery failed: {}", e);
                        eprintln!("💡 Try: webyc setup -p <master_secret>  # Create wallet with master secret");
                        eprintln!("💡 Or:   webyc recover <master_secret>   # Recover from external master secret");
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("❌ Failed to open wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Merge { group, max, memo } => {
            println!(
                "Merging outputs (group: {}, max: {}) for wallet: {}",
                group,
                max,
                wallet_path.display()
            );
            if let Some(memo) = memo {
                println!("Memo: {}", memo);
            }

            match open_wallet!() {
                Ok(wallet) => match wallet.merge(group).await {
                    Ok(summary) => {
                        println!("✅ Merge completed successfully");
                        println!("{}", summary);
                    }
                    Err(e) => {
                        eprintln!("❌ Merge failed: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("❌ Failed to open wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Encrypt { output, password } => {
            println!(
                "Encrypting wallet: {} to: {}",
                wallet_path.display(),
                output.display()
            );

            if password {
                println!("🔐 Password-based encryption");

                // Get password from user
                print!("Enter encryption password: ");
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                let password = rpassword::read_password().unwrap();

                match open_wallet!() {
                    Ok(wallet) => {
                        match wallet.encrypt_with_password(&password).await {
                            Ok(encrypted_data) => {
                                // Write encrypted data to file
                                let data = serde_json::to_vec_pretty(&encrypted_data)?;
                                std::fs::write(&output, data)?;
                                println!(
                                    "✅ Wallet encrypted with password and saved to: {}",
                                    output.display()
                                );
                                println!("🔒 Encryption algorithm: {}", encrypted_data.algorithm);
                                println!(
                                    "📅 Encrypted at: {}",
                                    encrypted_data.metadata.encrypted_at
                                );
                            }
                            Err(e) => {
                                eprintln!("❌ Encryption failed: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                println!("🔐 Passkey encryption (Face ID/Touch ID)");

                match open_wallet!() {
                    Ok(wallet) => {
                        match wallet.encrypt_with_passkey().await {
                            Ok(encrypted_data) => {
                                // Write encrypted data to file
                                let data = serde_json::to_vec_pretty(&encrypted_data)?;
                                std::fs::write(&output, data)?;
                                println!(
                                    "✅ Wallet encrypted with passkey and saved to: {}",
                                    output.display()
                                );
                                println!("🔒 Encryption algorithm: {}", encrypted_data.algorithm);
                                println!("📱 Platform: {}", encrypted_data.metadata.platform);
                                if let Some(pk_type) = &encrypted_data.metadata.passkey_type {
                                    println!("👤 Passkey type: {}", pk_type);
                                }
                                println!(
                                    "📅 Encrypted at: {}",
                                    encrypted_data.metadata.encrypted_at
                                );
                            }
                            Err(e) => {
                                eprintln!("❌ Passkey encryption failed: {}", e);
                                eprintln!("💡 Try using --password for password-based encryption");
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet with passkey encryption: {}", e);
                        eprintln!("💡 Try using --password for password-based encryption");
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Decrypt { input, password } => {
            println!(
                "Decrypting wallet from: {} to: {}",
                input.display(),
                wallet_path.display()
            );

            // Read encrypted data from file
            let data = match std::fs::read(&input) {
                Ok(data) => data,
                Err(e) => {
                    eprintln!("❌ Failed to read encrypted file: {}", e);
                    std::process::exit(1);
                }
            };

            let encrypted_data: EncryptedData = match serde_json::from_slice(&data) {
                Ok(data) => data,
                Err(e) => {
                    eprintln!("❌ Invalid encrypted file format: {}", e);
                    std::process::exit(1);
                }
            };

            println!("🔍 Encrypted file info:");
            println!("  Algorithm: {}", encrypted_data.algorithm);
            println!("  Platform: {}", encrypted_data.metadata.platform);
            println!("  Encrypted at: {}", encrypted_data.metadata.encrypted_at);
            if let Some(pk_type) = &encrypted_data.metadata.passkey_type {
                println!("  Passkey type: {}", pk_type);
            }

            if password || encrypted_data.algorithm.contains("PASSWORD") {
                println!("🔐 Password-based decryption");

                // Get password from user
                print!("Enter decryption password: ");
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                let password = rpassword::read_password().unwrap();

                match open_wallet!() {
                    Ok(wallet) => {
                        match wallet
                            .decrypt_with_password(&encrypted_data, &password)
                            .await
                        {
                            Ok(()) => {
                                println!(
                                    "✅ Wallet decrypted successfully from: {}",
                                    input.display()
                                );
                                let balance = wallet.balance().await?;
                                println!("💰 Restored wallet balance: {} WEBCASH", balance);
                            }
                            Err(e) => {
                                eprintln!("❌ Decryption failed: {}", e);
                                eprintln!("💡 Check your password and try again");
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                println!("🔐 Passkey decryption (Face ID/Touch ID)");

                match open_wallet!() {
                    Ok(wallet) => match wallet.decrypt_with_passkey(&encrypted_data).await {
                        Ok(()) => {
                            println!(
                                "✅ Wallet decrypted successfully with passkey from: {}",
                                input.display()
                            );
                            let balance = wallet.balance().await?;
                            println!("💰 Restored wallet balance: {} WEBCASH", balance);
                        }
                        Err(e) => {
                            eprintln!("❌ Passkey decryption failed: {}", e);
                            eprintln!("💡 Try using --password if passkey authentication is not available");
                            std::process::exit(1);
                        }
                    },
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet with passkey encryption: {}", e);
                        eprintln!("💡 Try using --password for password-based decryption");
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::EncryptDb { password } => {
            println!("🔐 Encrypting wallet database: {}", wallet_path.display());

            if password {
                println!("🔑 Password-based encryption");

                // Get password from user
                print!("Enter encryption password: ");
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                let encryption_password = rpassword::read_password().unwrap();

                // Open normal wallet and encrypt with password
                match open_wallet!() {
                    Ok(wallet) => {
                        match wallet
                            .encrypt_database_with_password(&encryption_password)
                            .await
                        {
                            Ok(()) => {
                                println!(
                                    "✅ Wallet database encrypted successfully with password!"
                                );
                                println!("🔒 Use the same password to decrypt the database");
                            }
                            Err(e) => {
                                eprintln!("❌ Failed to encrypt database: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet for encryption: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                println!("🔐 Passkey encryption (Face ID/Touch ID)");

                // Open wallet with passkey encryption enabled
                match open_wallet!() {
                    Ok(wallet) => match wallet.encrypt_database().await {
                        Ok(()) => {
                            println!("✅ Wallet database encrypted successfully!");
                            println!("🔒 The database file is now encrypted and can only be opened with passkey authentication");
                        }
                        Err(e) => {
                            eprintln!("❌ Failed to encrypt database: {}", e);
                            std::process::exit(1);
                        }
                    },
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet for encryption: {}", e);
                        eprintln!(
                            "💡 Make sure the wallet exists and passkey features are available"
                        );
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::DecryptDb { password } => {
            println!("🔓 Decrypting wallet database: {}", wallet_path.display());

            if password {
                println!("🔑 Password-based decryption");

                // Get password from user
                print!("Enter decryption password: ");
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                let decryption_password = rpassword::read_password().unwrap();

                // Decrypt database with password (no need to open wallet first)
                let dummy_wallet = open_wallet!()
                    .map_err(|_| "Cannot access encrypted database without correct method")?;

                match dummy_wallet
                    .decrypt_database_with_password(&decryption_password)
                    .await
                {
                    Ok(()) => {
                        println!("✅ Wallet database decrypted successfully with password!");
                        println!("🔓 Database is now accessible as normal SQLite file");
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to decrypt database: {}", e);
                        eprintln!("💡 Check your password and try again");
                        std::process::exit(1);
                    }
                }
            } else {
                println!("🔐 Passkey decryption (Face ID/Touch ID)");

                // Open wallet with passkey encryption
                match open_wallet!() {
                    Ok(wallet) => match wallet.decrypt_database().await {
                        Ok(()) => {
                            println!("✅ Wallet database decrypted and ready for use!");
                            println!("🔓 You can now perform transactions with this wallet");
                            let balance = wallet.balance().await?;
                            println!("💰 Current balance: {} WEBCASH", balance);
                        }
                        Err(e) => {
                            eprintln!("❌ Failed to decrypt database: {}", e);
                            std::process::exit(1);
                        }
                    },
                    Err(e) => {
                        eprintln!("❌ Failed to open encrypted wallet: {}", e);
                        eprintln!("💡 Make sure the database is encrypted and passkey authentication is available");
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Mine => {
            println!("Mining webcash on {:?}...", network);
            match open_wallet!() {
                Ok(wallet) => match wallet.mine().await {
                    Ok(result) => {
                        println!("Mined {} webcash!", result.amount);
                        println!("Webcash: {}", result.webcash);
                        println!("Difficulty: {} bits", result.difficulty);
                        println!("Hash: {}", result.hash);
                        let balance = wallet.balance().await?;
                        println!("New balance: {} WEBCASH", balance);
                    }
                    Err(e) => {
                        eprintln!("Mining failed: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("Failed to open wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

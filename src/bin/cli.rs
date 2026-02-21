//! Webcash CLI - Command Line Interface for Webcash Wallet

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::str::FromStr;
use webylib::{Wallet, SecretWebcash, Amount};
use webylib::biometric::EncryptedData;

#[derive(Parser)]
#[command(name = "webyc")]
#[command(about = "Webcash wallet command line interface")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Wallet database file path
    #[arg(short, long, default_value = "wallet.db")]
    wallet: PathBuf,

    /// Enable biometric authentication for encrypted wallets
    #[arg(long)]
    biometric: bool,

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
        /// Enable biometric encryption (Face ID/Touch ID on mobile)
        #[arg(long)]
        biometric: bool,
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
    /// Encrypt wallet using biometrics or password
    Encrypt {
        /// Output file for encrypted wallet
        #[arg(short, long)]
        output: PathBuf,
        /// Use password instead of biometrics
        #[arg(long)]
        password: bool,
    },
    /// Decrypt wallet from encrypted file
    Decrypt {
        /// Input file containing encrypted wallet
        #[arg(short, long)]
        input: PathBuf,
        /// Use password instead of biometrics
        #[arg(long)]
        password: bool,
    },
    /// Encrypt the wallet database file with password (for runtime use)
    EncryptDb {
        /// Use password instead of biometric authentication
        #[arg(long)]
        password: bool,
    },
    /// Decrypt the wallet database file (for runtime use)
    DecryptDb {
        /// Use password instead of biometric authentication
        #[arg(long)]
        password: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Setup { master_secret, biometric } => {
            println!("Setting up new wallet at: {}", cli.wallet.display());

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

            if biometric {
                println!("🔐 Biometric encryption enabled");
            }

            match Wallet::open_with_biometric(&cli.wallet, biometric).await {
                Ok(wallet) => {
                    // Store the master secret only if explicitly provided
                    if explicit_master_secret {
                        match wallet.store_master_secret(&master_secret_hex).await {
                            Ok(()) => {
                                println!("✅ Wallet created successfully with provided master secret!");
                            }
                            Err(e) => {
                                eprintln!("❌ Failed to store master secret: {}", e);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        println!("✅ Wallet created successfully with auto-generated master secret!");
                    }
                    
                    let stats = wallet.stats().await?;
                    println!("📊 Wallet statistics:");
                    println!("  Total webcash: {}", stats.total_webcash);
                    println!("  Unspent webcash: {}", stats.unspent_webcash);
                    println!("  Balance: {}", stats.total_balance);
                    println!("  Biometric encryption: {}", if wallet.is_biometric_enabled() { "Enabled" } else { "Disabled" });
                }
                Err(e) => {
                    eprintln!("❌ Failed to create wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Info => {
            println!("Wallet information for: {}", cli.wallet.display());
            match Wallet::open_with_biometric(&cli.wallet, cli.biometric).await {
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
                    eprintln!("💡 Try running 'webyc --wallet {} setup' to create a new wallet", cli.wallet.display());
                    std::process::exit(1);
                }
            }
        }
        Commands::Insert { webcash, memo, offline } => {
            println!("Inserting webcash into wallet: {}", cli.wallet.display());
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

            match Wallet::open_with_biometric(&cli.wallet, cli.biometric).await {
                Ok(wallet) => {
                    // Match Python: insert does NOT validate before replace by default
                    // Only validate if explicitly requested (not the default behavior)
                    let validate_with_server = false; // Python doesn't validate before replace
                    match wallet.insert_with_validation(secret_webcash.clone(), validate_with_server).await {
                        Ok(()) => {
                            println!("✅ Successfully inserted webcash: {}", secret_webcash.amount);
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
            println!("Generating payment webcash for amount: {} with memo: '{}' from wallet: {}", amount, memo_str, cli.wallet.display());

            // Parse the amount
            let payment_amount = match Amount::from_str(&amount) {
                Ok(amt) => amt,
                Err(e) => {
                    eprintln!("❌ Invalid amount format: {}", e);
                    std::process::exit(1);
                }
            };

            match Wallet::open_with_biometric(&cli.wallet, cli.biometric).await {
                Ok(wallet) => {
                    match wallet.pay(payment_amount, memo_str).await {
                        Ok(message) => {
                            println!("✅ {}", message);
                            let new_balance = wallet.balance().await.unwrap_or_else(|_| "unknown".to_string());
                            println!("📊 New balance: {} WEBCASH", new_balance);
                        }
                        Err(e) => {
                            eprintln!("❌ Payment generation failed: {}", e);
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
        Commands::Check => {
            println!("Checking wallet against server: {}", cli.wallet.display());
            match Wallet::open_with_biometric(&cli.wallet, cli.biometric).await {
                Ok(wallet) => {
                    match wallet.check().await {
                        Ok(()) => {
                            println!("✅ Wallet check completed successfully");
                        }
                        Err(e) => {
                            eprintln!("❌ Wallet check failed: {}", e);
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
        Commands::Recover { gap_limit } => {
            println!("Recovering wallet with gap limit: {} for wallet: {}", gap_limit, cli.wallet.display());

            match Wallet::open_with_biometric(&cli.wallet, cli.biometric).await {
                Ok(wallet) => {
                    match wallet.recover_from_wallet(gap_limit).await {
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
                    }
                }
                Err(e) => {
                    eprintln!("❌ Failed to open wallet: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Merge { group, max, memo } => {
            println!("Merging outputs (group: {}, max: {}) for wallet: {}", group, max, cli.wallet.display());
            if let Some(memo) = memo {
                println!("Memo: {}", memo);
            }

            match Wallet::open_with_biometric(&cli.wallet, cli.biometric).await {
                Ok(wallet) => {
                    match wallet.merge(group).await {
                        Ok(summary) => {
                            println!("✅ Merge completed successfully");
                            println!("{}", summary);
                        }
                        Err(e) => {
                            eprintln!("❌ Merge failed: {}", e);
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
        Commands::Encrypt { output, password } => {
            println!("Encrypting wallet: {} to: {}", cli.wallet.display(), output.display());
            
            if password {
                println!("🔐 Password-based encryption");
                
                // Get password from user
                print!("Enter encryption password: ");
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                let password = rpassword::read_password().unwrap();
                
                match Wallet::open_with_biometric(&cli.wallet, cli.biometric).await {
                    Ok(wallet) => {
                        match wallet.encrypt_with_password(&password).await {
                            Ok(encrypted_data) => {
                                // Write encrypted data to file
                                let data = serde_json::to_vec_pretty(&encrypted_data)?;
                                std::fs::write(&output, data)?;
                                println!("✅ Wallet encrypted with password and saved to: {}", output.display());
                                println!("🔒 Encryption algorithm: {}", encrypted_data.algorithm);
                                println!("📅 Encrypted at: {}", encrypted_data.metadata.encrypted_at);
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
                println!("🔐 Biometric encryption (Face ID/Touch ID)");
                
                match Wallet::open_with_biometric(&cli.wallet, true).await {
                    Ok(wallet) => {
                        match wallet.encrypt_with_biometrics().await {
                            Ok(encrypted_data) => {
                                // Write encrypted data to file
                                let data = serde_json::to_vec_pretty(&encrypted_data)?;
                                std::fs::write(&output, data)?;
                                println!("✅ Wallet encrypted with biometrics and saved to: {}", output.display());
                                println!("🔒 Encryption algorithm: {}", encrypted_data.algorithm);
                                println!("📱 Platform: {}", encrypted_data.metadata.platform);
                                if let Some(bio_type) = &encrypted_data.metadata.biometric_type {
                                    println!("👤 Biometric type: {}", bio_type);
                                }
                                println!("📅 Encrypted at: {}", encrypted_data.metadata.encrypted_at);
                            }
                            Err(e) => {
                                eprintln!("❌ Biometric encryption failed: {}", e);
                                eprintln!("💡 Try using --password for password-based encryption");
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet with biometric encryption: {}", e);
                        eprintln!("💡 Try using --password for password-based encryption");
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Decrypt { input, password } => {
            println!("Decrypting wallet from: {} to: {}", input.display(), cli.wallet.display());
            
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
            if let Some(bio_type) = &encrypted_data.metadata.biometric_type {
                println!("  Biometric type: {}", bio_type);
            }
            
            if password || encrypted_data.algorithm.contains("PASSWORD") {
                println!("🔐 Password-based decryption");
                
                // Get password from user
                print!("Enter decryption password: ");
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                let password = rpassword::read_password().unwrap();
                
                match Wallet::open_with_biometric(&cli.wallet, cli.biometric).await {
                    Ok(wallet) => {
                        match wallet.decrypt_with_password(&encrypted_data, &password).await {
                            Ok(()) => {
                                println!("✅ Wallet decrypted successfully from: {}", input.display());
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
                println!("🔐 Biometric decryption (Face ID/Touch ID)");
                
                match Wallet::open_with_biometric(&cli.wallet, true).await {
                    Ok(wallet) => {
                        match wallet.decrypt_with_biometrics(&encrypted_data).await {
                            Ok(()) => {
                                println!("✅ Wallet decrypted successfully with biometrics from: {}", input.display());
                                let balance = wallet.balance().await?;
                                println!("💰 Restored wallet balance: {} WEBCASH", balance);
                            }
                            Err(e) => {
                                eprintln!("❌ Biometric decryption failed: {}", e);
                                eprintln!("💡 Try using --password if biometric authentication is not available");
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet with biometric encryption: {}", e);
                        eprintln!("💡 Try using --password for password-based decryption");
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::EncryptDb { password } => {
            println!("🔐 Encrypting wallet database: {}", cli.wallet.display());
            
            if password {
                println!("🔑 Password-based encryption");
                
                // Get password from user
                print!("Enter encryption password: ");
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                let encryption_password = rpassword::read_password().unwrap();
                
                // Open normal wallet and encrypt with password
                match Wallet::open_with_biometric(&cli.wallet, false).await {
                    Ok(wallet) => {
                        match wallet.encrypt_database_with_password(&encryption_password).await {
                            Ok(()) => {
                                println!("✅ Wallet database encrypted successfully with password!");
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
                println!("🔐 Biometric encryption (Face ID/Touch ID)");
                
                // Open wallet with biometric encryption enabled
                match Wallet::open_with_biometric(&cli.wallet, true).await {
                    Ok(wallet) => {
                        match wallet.encrypt_database().await {
                            Ok(()) => {
                                println!("✅ Wallet database encrypted successfully!");
                                println!("🔒 The database file is now encrypted and can only be opened with biometric authentication");
                            }
                            Err(e) => {
                                eprintln!("❌ Failed to encrypt database: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to open wallet for encryption: {}", e);
                        eprintln!("💡 Make sure the wallet exists and biometric features are available");
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::DecryptDb { password } => {
            println!("🔓 Decrypting wallet database: {}", cli.wallet.display());
            
            if password {
                println!("🔑 Password-based decryption");
                
                // Get password from user
                print!("Enter decryption password: ");
                use std::io::Write;
                std::io::stdout().flush().unwrap();
                let decryption_password = rpassword::read_password().unwrap();
                
                // Decrypt database with password (no need to open wallet first)
                let dummy_wallet = Wallet::open_with_biometric(&cli.wallet, false).await
                    .map_err(|_| "Cannot access encrypted database without correct method")?;
                
                match dummy_wallet.decrypt_database_with_password(&decryption_password).await {
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
                println!("🔐 Biometric decryption (Face ID/Touch ID)");
                
                // Open wallet with biometric encryption
                match Wallet::open_with_biometric(&cli.wallet, true).await {
                    Ok(wallet) => {
                        match wallet.decrypt_database().await {
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
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to open encrypted wallet: {}", e);
                        eprintln!("💡 Make sure the database is encrypted and biometric authentication is available");
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    Ok(())
}

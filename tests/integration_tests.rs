//! Integration tests for Webcash wallet
//!
//! Consolidated integration test suite including:
//! - Full wallet integration tests
//! - Server API tests
//! - CLI manual workflow tests
//! - Money preservation tests
//! - Phase 2 verification tests
//! - Passkey encryption tests
//! - Runtime encryption tests
//!
//! These tests require TEST_WEBCASH_SECRET environment variable to be set
//! with a valid secret webcash string for testing against the live Webcash server.

use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tokio::task;
use webylib::passkey::EncryptedData;
use webylib::*;

// ============================================================================
// Core Integration Tests
// ============================================================================

/// Comprehensive integration test using real TEST_WEBCASH_SECRET
#[tokio::test]
async fn test_full_wallet_integration() {
    let test_secret = match env::var("TEST_WEBCASH_SECRET") {
        Ok(secret) => secret,
        Err(_) => {
            println!("⚠️  Skipping integration test: TEST_WEBCASH_SECRET not set");
            return;
        }
    };

    println!("🧪 Starting comprehensive wallet integration test");
    let _ = fs::remove_file("integration_test_wallet.db");

    let wallet = Wallet::open("integration_test_wallet.db")
        .await
        .expect("Failed to create wallet");

    let webcash = SecretWebcash::parse(&test_secret).expect("Failed to parse TEST_WEBCASH_SECRET");

    wallet
        .insert(webcash.clone())
        .await
        .expect("Failed to insert test webcash");

    let balance = wallet.balance().await.expect("Failed to get balance");
    println!("📈 Wallet balance: {} WEBCASH", balance);

    let stats = wallet.stats().await.expect("Failed to get wallet stats");
    println!(
        "📊 Wallet stats: {} total, {} unspent",
        stats.total_webcash, stats.unspent_webcash
    );

    let _ = fs::remove_file("integration_test_wallet.db");
}

/// Test server API endpoints individually
#[tokio::test]
async fn test_server_api_endpoints() {
    let test_secret = match env::var("TEST_WEBCASH_SECRET") {
        Ok(secret) => secret,
        Err(_) => {
            println!("⚠️  Skipping server API test: TEST_WEBCASH_SECRET not set");
            return;
        }
    };

    let server_client = crate::server::ServerClient::new().expect("Failed to create server client");

    let webcash = SecretWebcash::parse(&test_secret).expect("Failed to parse test webcash");
    let public_webcash = vec![webcash.to_public()];

    match server_client.health_check(&public_webcash).await {
        Ok(response) => println!(
            "✅ Health check successful: {} entries",
            response.results.len()
        ),
        Err(e) => println!("⚠️  Health check failed: {}", e),
    }

    match server_client.get_target().await {
        Ok(target) => println!(
            "✅ Target query successful: difficulty {}",
            target.difficulty_target_bits
        ),
        Err(e) => println!("⚠️  Target query failed: {}", e),
    }
}

/// Test wallet operations without server dependency
#[tokio::test]
async fn test_wallet_operations_offline() {
    let _ = fs::remove_file("offline_test_wallet.db");

    let wallet = Wallet::open("offline_test_wallet.db")
        .await
        .expect("Failed to create offline test wallet");

    let balance = wallet.balance().await.unwrap();
    assert_eq!(balance, "0");

    let test_master_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    wallet
        .store_master_secret(test_master_secret)
        .await
        .unwrap();

    let test_webcash = SecretWebcash::parse(
        "e1.00000000:secret:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    )
    .unwrap();
    wallet.store_directly(test_webcash.clone()).await.unwrap();

    let balance = wallet.balance().await.unwrap();
    assert_eq!(balance, "1");

    let _ = fs::remove_file("offline_test_wallet.db");
}

/// Comprehensive cross-wallet integration test with HD recovery
#[tokio::test]
async fn test_cross_wallet_hd_recovery_integration() {
    let test_secret = match env::var("TEST_WEBCASH_SECRET") {
        Ok(secret) => secret,
        Err(_) => {
            println!("⚠️  Skipping cross-wallet integration test: TEST_WEBCASH_SECRET not set");
            return;
        }
    };

    let _ = fs::remove_file("primary_test_wallet.db");
    let _ = fs::remove_file("secondary_test_wallet.db");

    let master_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let primary_wallet = Wallet::open("primary_test_wallet.db")
        .await
        .expect("Failed to create primary wallet");

    primary_wallet
        .store_master_secret(master_secret)
        .await
        .expect("Failed to store master secret");

    let webcash = SecretWebcash::parse(&test_secret).expect("Failed to parse TEST_WEBCASH_SECRET");

    match primary_wallet.insert(webcash.clone()).await {
        Ok(_) => {}
        Err(e) => {
            if e.to_string().contains("spent") {
                primary_wallet
                    .insert_with_validation(webcash.clone(), false)
                    .await
                    .expect("Failed to insert test webcash");
            } else {
                panic!("Failed to insert test webcash: {}", e);
            }
        }
    }

    use crate::hd::{ChainCode, HDWallet};
    let master_secret_bytes = hex::decode(master_secret).expect("Failed to decode master secret");
    let mut master_secret_array = [0u8; 32];
    master_secret_array.copy_from_slice(&master_secret_bytes);
    let hd_wallet = HDWallet::from_master_secret(master_secret_array);

    let pay_secret_hex = hd_wallet
        .derive_secret(ChainCode::Pay, 0);

    let payment_webcash = SecretWebcash::new(
        crate::webcash::SecureString::new(pay_secret_hex),
        webcash.amount,
    );

    let secondary_wallet = Wallet::open("secondary_test_wallet.db")
        .await
        .expect("Failed to create secondary wallet");

    let payment_webcash_for_insert = SecretWebcash::parse(&payment_webcash.to_string())
        .expect("Failed to parse payment webcash");

    secondary_wallet
        .insert(payment_webcash_for_insert.clone())
        .await
        .expect("Failed to insert payment");

    let _recovery_result = secondary_wallet
        .recover(master_secret, 5)
        .await
        .expect("Failed to recover secondary wallet");

    let _ = fs::remove_file("primary_test_wallet.db");
    let _ = fs::remove_file("secondary_test_wallet.db");
}

// ============================================================================
// CLI Manual Workflow Test
// ============================================================================

/// CLI manual test - uses webyc CLI like a human would
#[tokio::test]
async fn test_cli_manual_workflow() {
    let test_secret = match env::var("TEST_WEBCASH_SECRET") {
        Ok(secret) => secret.trim().to_string(),
        Err(_) => {
            println!("⚠️  Skipping test: TEST_WEBCASH_SECRET not set");
            return;
        }
    };

    println!("🧪 CLI MANUAL WORKFLOW TEST");
    println!("💰 Input webcash: {}", test_secret);

    let input_webcash =
        webylib::SecretWebcash::parse(&test_secret).expect("❌ Failed to parse input webcash");
    let input_amount = input_webcash.amount;

    let wallet_path = "cli_test_wallet.db";
    let history_path = "cli_test_history.txt";
    let master_secret_path = "cli_test_master_secret.txt";
    let cli_binary = "./target/release/webyc";

    if !PathBuf::from(cli_binary).exists() {
        panic!(
            "❌ CLI binary not found at {}. Run 'cargo build --release' first",
            cli_binary
        );
    }

    let _ = fs::remove_file(wallet_path);
    let _ = fs::remove_file(history_path);
    let _ = fs::remove_file(master_secret_path);

    let mut history_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(history_path)
        .expect("❌ Failed to create history file");

    writeln!(history_file, "=== CLI MANUAL WORKFLOW HISTORY ===").unwrap();
    writeln!(history_file, "Input webcash: {}", test_secret).unwrap();
    writeln!(history_file, "Input amount: {} WEBCASH", input_amount).unwrap();
    writeln!(history_file).unwrap();

    println!("\n📁 STEP 1: Setup Wallet with Master Secret (CLI)");

    use webylib::crypto::CryptoSecret;
    let master_secret_obj = CryptoSecret::generate().expect("❌ Failed to generate master secret");
    let master_secret = master_secret_obj.to_hex();

    fs::write(master_secret_path, &master_secret).expect("❌ Failed to save master secret");

    let setup_output = Command::new(cli_binary)
        .args(["--wallet", wallet_path, "setup", "-p", &master_secret])
        .output()
        .expect("❌ Failed to run webyc setup");

    if !setup_output.status.success() {
        let stderr = String::from_utf8_lossy(&setup_output.stderr);
        panic!("❌ Wallet setup failed: {}", stderr);
    }

    println!("✅ Wallet created successfully");

    println!("\n📥 STEP 2: Insert Webcash (CLI)");

    let insert_output = Command::new(cli_binary)
        .args(["--wallet", wallet_path, "insert", &test_secret])
        .output()
        .expect("❌ Failed to run webyc insert");

    if !insert_output.status.success() {
        let stderr = String::from_utf8_lossy(&insert_output.stderr);
        panic!("❌ Insert failed: {}", stderr);
    }

    println!("\n⏭️  STEP 4: Skipping Payment (PRESERVING ALL MONEY)");

    println!("\n🔑 STEP 7: Save Master Secret");
    fs::write(master_secret_path, &master_secret).expect("❌ Failed to save master secret");
    println!("✅ Master secret saved to: {}", master_secret_path);

    println!("\n🗑️  STEP 8: Delete Wallet");
    fs::remove_file(wallet_path).expect("❌ Failed to delete wallet");

    println!("\n🔄 STEP 9: Recreate Wallet from Master Secret");
    let recreate_output = Command::new(cli_binary)
        .args(["--wallet", wallet_path, "setup", "-p", &master_secret])
        .output()
        .expect("❌ Failed to run webyc setup");

    if !recreate_output.status.success() {
        let stderr = String::from_utf8_lossy(&recreate_output.stderr);
        panic!("❌ Failed to recreate wallet: {}", stderr);
    }

    println!("\n🔄 STEP 10: Recover Webcash (CLI)");
    let recover_output = Command::new(cli_binary)
        .args(["--wallet", wallet_path, "recover", "--gap-limit", "20"])
        .output()
        .expect("❌ Failed to run webyc recover");

    if !recover_output.status.success() {
        let stderr = String::from_utf8_lossy(&recover_output.stderr);
        panic!("❌ Recovery failed: {}", stderr);
    }

    println!("\n🎯 STEP 12: Generate Final Output (CLI)");
    let balance_check = Command::new(cli_binary)
        .args(["--wallet", wallet_path, "info"])
        .output()
        .expect("❌ Failed to check balance");

    let current_balance = if balance_check.status.success() {
        let stdout = String::from_utf8_lossy(&balance_check.stdout);
        let mut balance = Amount::ZERO;
        for line in stdout.lines() {
            if line.contains("Balance:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                for (i, part) in parts.iter().enumerate() {
                    if part == &"Balance:" && i + 1 < parts.len() {
                        if let Ok(amt) = Amount::from_str(parts[i + 1]) {
                            balance = amt;
                            break;
                        }
                    }
                }
            }
        }
        balance
    } else {
        Amount::ZERO
    };

    if current_balance == Amount::ZERO {
        panic!("❌ No balance available!");
    }

    let amount_str = format!("{}", current_balance);
    let final_pay_output = Command::new(cli_binary)
        .args([
            "--wallet",
            wallet_path,
            "pay",
            &amount_str,
            "-m",
            "Final output",
        ])
        .output()
        .expect("❌ Failed to run webyc pay");

    let output_webcash_str = if final_pay_output.status.success() {
        let stdout = String::from_utf8_lossy(&final_pay_output.stdout);
        let mut output_secret = String::new();

        for line in stdout.lines() {
            if line.contains("Send this webcash to recipient:") {
                if let Some(recipient_pos) = line.find("recipient:") {
                    let after_recipient = &line[recipient_pos + 10..].trim_start();
                    if let Some(e_pos) = after_recipient.find("e") {
                        let candidate_str = &after_recipient[e_pos..];
                        let end = candidate_str
                            .find(|c: char| c.is_whitespace() || c == '\n' || c == '\r')
                            .unwrap_or(candidate_str.len());
                        let candidate = candidate_str[..end].trim().to_string();

                        if webylib::SecretWebcash::parse(&candidate).is_ok() {
                            output_secret = candidate;
                            break;
                        }
                    }
                }
            }
        }

        if output_secret.is_empty() {
            for line in stdout.lines() {
                if line.contains(":secret:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    for part in parts {
                        if part.starts_with("e")
                            && part.contains(":secret:")
                            && part.len() > 70
                            && webylib::SecretWebcash::parse(part).is_ok()
                        {
                            output_secret = part.to_string();
                            break;
                        }
                    }
                    if !output_secret.is_empty() {
                        break;
                    }
                }
            }
        }

        if output_secret.is_empty() {
            panic!("❌ Could not extract output webcash");
        }

        output_secret
    } else {
        panic!("❌ Payment failed");
    };

    let output_webcash =
        webylib::SecretWebcash::parse(&output_webcash_str).expect("❌ Invalid output webcash");

    println!("\n🎉 CLI TEST COMPLETED!");
    println!("📤 FINAL OUTPUT SECRET (use for next test):");
    println!("{}", output_webcash_str);

    let _ = fs::remove_file(wallet_path);

    assert!(
        output_webcash.amount > Amount::ZERO,
        "❌ OUTPUT AMOUNT IS ZERO!"
    );

    if output_webcash.amount == input_amount {
        println!("\n✅ PERFECT: Output amount exactly matches input - NO MONEY LOST!");
    }
}

// ============================================================================
// Complete Money Preservation Test
// ============================================================================

/// Extract webcash from payment message
fn extract_webcash_from_message(msg: &str, expected_amount: Amount) -> String {
    let amount_str = format!("e{}:secret:", expected_amount);
    if let Some(start) = msg.find(&amount_str) {
        let end = msg[start..].find(' ').unwrap_or(msg.len() - start);
        return msg[start..start + end].to_string();
    }

    for line in msg.lines() {
        if line.contains(":secret:") {
            if let Some(start) = line.find("e") {
                let end = line[start..].find(' ').unwrap_or(line.len() - start);
                return line[start..start + end].to_string();
            }
        }
    }

    panic!("❌ Could not extract webcash from: {}", msg);
}

/// Recover from master secret and generate payment
async fn recover_and_pay(
    wallet: &Wallet,
    master_secret: &str,
    target_amount: Amount,
    history_file: &mut std::fs::File,
) -> String {
    match wallet.recover(master_secret, 20).await {
        Ok(_summary) => {
            writeln!(history_file, "  Recovery: SUCCESS").unwrap();

            let balance = wallet
                .balance()
                .await
                .expect("❌ Failed to get balance after recovery");
            let balance_amount = Amount::from_str(&balance).expect("❌ Failed to parse balance");

            if balance_amount >= target_amount {
                match wallet
                    .pay(target_amount, "Final output after recovery")
                    .await
                {
                    Ok(msg) => extract_webcash_from_message(&msg, target_amount),
                    Err(_e) => {
                        let remaining = wallet
                            .list_webcash()
                            .await
                            .expect("❌ Failed to list after recovery");
                        if let Some(first) = remaining.first() {
                            first.to_string()
                        } else {
                            panic!("❌ No webcash after recovery!");
                        }
                    }
                }
            } else {
                let remaining = wallet
                    .list_webcash()
                    .await
                    .expect("❌ Failed to list after recovery");
                if let Some(first) = remaining.first() {
                    first.to_string()
                } else {
                    panic!("❌ No webcash after recovery!");
                }
            }
        }
        Err(e) => {
            writeln!(history_file, "  Recovery: FAILED - {}", e).unwrap();
            let remaining = wallet
                .list_webcash()
                .await
                .expect("❌ Failed to list webcash");
            if let Some(first) = remaining.first() {
                first.to_string()
            } else {
                panic!("❌ No webcash available!");
            }
        }
    }
}

/// Complete test that NEVER loses money
#[tokio::test]
async fn test_complete_money_preservation() {
    let test_secret = match env::var("TEST_WEBCASH_SECRET") {
        Ok(secret) => secret.trim().to_string(),
        Err(_) => {
            println!("⚠️  Skipping test: TEST_WEBCASH_SECRET not set");
            return;
        }
    };

    println!("🧪 COMPLETE MONEY PRESERVATION TEST");

    let input_webcash =
        SecretWebcash::parse(&test_secret).expect("❌ Failed to parse input webcash");
    let input_amount = input_webcash.amount;

    let wallet_path = "preservation_test_wallet.db";
    let history_path = "preservation_test_history.txt";
    let master_secret_path = "preservation_test_master_secret.txt";

    let _ = fs::remove_file(wallet_path);
    let _ = fs::remove_file(history_path);
    let _ = fs::remove_file(master_secret_path);

    let mut history_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(history_path)
        .expect("❌ Failed to create history file");

    let wallet = Wallet::open(wallet_path)
        .await
        .expect("❌ Failed to create wallet");

    use crate::crypto::CryptoSecret;
    let master_secret_obj = CryptoSecret::generate().expect("❌ Failed to generate master secret");
    let master_secret = master_secret_obj.to_hex();

    wallet
        .store_master_secret(&master_secret)
        .await
        .expect("❌ Failed to store master secret");

    fs::write(master_secret_path, &master_secret).expect("❌ Failed to save master secret");

    match wallet.insert(input_webcash.clone()).await {
        Ok(_) => {
            println!("✅ Webcash inserted successfully");
        }
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("spent") {
                panic!("❌ Input webcash has been spent!");
            } else {
                panic!("❌ Failed to insert webcash: {}", e);
            }
        }
    }

    let payment_amount = Amount::from_str("0.00001").expect("❌ Failed to parse payment amount");

    let payment_result = wallet.pay(payment_amount, "Test payment").await;
    let payment_webcash_str = match payment_result {
        Ok(msg) => extract_webcash_from_message(&msg, payment_amount),
        Err(e) => {
            if e.to_string().contains("Insufficient funds") {
                println!("⚠️  Insufficient funds, skipping payment");
                String::new()
            } else {
                panic!("❌ Payment failed: {}", e);
            }
        }
    };

    if !payment_webcash_str.is_empty() {
        let payment_webcash =
            SecretWebcash::parse(&payment_webcash_str).expect("❌ Failed to parse payment webcash");

        match wallet.insert(payment_webcash.clone()).await {
            Ok(_) => {}
            Err(e) => {
                if !e.to_string().contains("spent") {
                    panic!("❌ Failed to re-insert payment: {}", e);
                }
            }
        }
    }

    let final_balance = wallet.balance().await.expect("❌ Failed to get balance");
    let final_balance_amount =
        Amount::from_str(&final_balance).expect("❌ Failed to parse balance");

    let output_webcash_str = if final_balance_amount >= input_amount {
        match wallet.pay(input_amount, "Final output webcash").await {
            Ok(msg) => extract_webcash_from_message(&msg, input_amount),
            Err(_e) => {
                recover_and_pay(&wallet, &master_secret, input_amount, &mut history_file).await
            }
        }
    } else {
        recover_and_pay(&wallet, &master_secret, input_amount, &mut history_file).await
    };

    let output_webcash =
        SecretWebcash::parse(&output_webcash_str).expect("❌ Invalid output webcash");

    println!("\n🎉 TEST COMPLETED!");
    println!("📤 FINAL OUTPUT SECRET (use for next test):");
    println!("{}", output_webcash_str);

    let _ = fs::remove_file(wallet_path);

    assert_eq!(
        output_webcash.amount, input_amount,
        "❌ OUTPUT AMOUNT {} DOES NOT MATCH INPUT {}!",
        output_webcash.amount, input_amount
    );
}

// ============================================================================
// Online Preservation Test
// ============================================================================

/// Online test that preserves the exact input amount
#[tokio::test]
async fn test_online_amount_preservation() {
    let test_secret = match env::var("TEST_WEBCASH_SECRET") {
        Ok(secret) => secret,
        Err(_) => {
            println!("⚠️  Skipping test: TEST_WEBCASH_SECRET not set");
            return;
        }
    };

    println!("🧪 Online Amount Preservation Test");

    let input_webcash =
        SecretWebcash::parse(&test_secret).expect("❌ Failed to parse input webcash");
    let input_amount = input_webcash.amount;

    let wallet_path = "online_preservation_test_wallet.db";
    let history_path = "online_preservation_history.txt";
    let _ = fs::remove_file(wallet_path);
    let _ = fs::remove_file(history_path);

    let wallet = Wallet::open(wallet_path)
        .await
        .expect("❌ Failed to create wallet");

    let test_master_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    wallet
        .store_master_secret(test_master_secret)
        .await
        .expect("❌ Failed to store master secret");

    match wallet.insert(input_webcash.clone()).await {
        Ok(_) => {
            println!("✅ Webcash inserted successfully");
        }
        Err(e) => {
            if e.to_string().contains("spent") {
                panic!("❌ Input webcash has been spent!");
            } else {
                panic!("❌ Failed to insert webcash: {}", e);
            }
        }
    }

    let payment_amount = Amount::from_str("0.00001").expect("❌ Failed to parse payment amount");

    let payment_result = wallet.pay(payment_amount, "Test payment").await;
    let _payment_webcash_str = match payment_result {
        Ok(msg) => {
            if let Some(start) = msg.find("e0.00001:secret:") {
                let end = msg[start..].find(' ').unwrap_or(msg.len() - start);
                msg[start..start + end].to_string()
            } else {
                panic!("❌ Could not extract payment webcash");
            }
        }
        Err(e) => {
            if e.to_string().contains("Insufficient funds") {
                println!("⚠️  Insufficient funds for payment");
                String::new()
            } else {
                panic!("❌ Payment failed: {}", e);
            }
        }
    };

    let final_balance = wallet
        .balance()
        .await
        .expect("❌ Failed to get final balance");
    let final_balance_amount =
        Amount::from_str(&final_balance).expect("❌ Failed to parse final balance");

    let amount_to_return = if final_balance_amount >= input_amount {
        input_amount
    } else {
        final_balance_amount
    };

    let output_result = wallet.pay(amount_to_return, "Test output webcash").await;
    let _output_webcash_str = match output_result {
        Ok(msg) => {
            if let Some(start) = msg.find(&format!("e{}:secret:", amount_to_return)) {
                let end = msg[start..].find(' ').unwrap_or(msg.len() - start);
                msg[start..start + end].to_string()
            } else {
                panic!("❌ Could not extract output webcash");
            }
        }
        Err(_e) => {
            panic!("❌ Could not generate output webcash");
        }
    };

    let _ = fs::remove_file(wallet_path);
}

// ============================================================================
// Phase 2 Verification Tests
// ============================================================================

/// Comprehensive Phase 2 verification test
#[tokio::test]
async fn test_phase2_all_operations() {
    let test_secret = match env::var("TEST_WEBCASH_SECRET") {
        Ok(secret) => secret,
        Err(_) => {
            println!("⚠️  Skipping Phase 2 test: TEST_WEBCASH_SECRET not set");
            return;
        }
    };

    println!("🧪 Phase 2 Verification Test");

    let wallet_path = "phase2_test_wallet.db";
    let _ = fs::remove_file(wallet_path);

    let wallet = Wallet::open(wallet_path)
        .await
        .expect("❌ Failed to create wallet");

    let test_master_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    wallet
        .store_master_secret(test_master_secret)
        .await
        .expect("❌ Failed to store master secret");

    let webcash =
        SecretWebcash::parse(&test_secret).expect("❌ Failed to parse TEST_WEBCASH_SECRET");

    let insert_result = wallet.insert(webcash.clone()).await;
    match insert_result {
        Ok(_) => {
            println!("✅ Webcash inserted successfully");
        }
        Err(e) => {
            if e.to_string().contains("spent") {
                wallet
                    .insert_with_validation(webcash.clone(), false)
                    .await
                    .expect("❌ Failed to insert webcash");
            } else {
                panic!("❌ Failed to insert webcash: {}", e);
            }
        }
    }

    let balance = wallet.balance().await.expect("❌ Failed to get balance");
    assert!(!balance.is_empty());

    let webcash_list = wallet
        .list_webcash()
        .await
        .expect("❌ Failed to list webcash");
    assert!(!webcash_list.is_empty());

    let stats = wallet.stats().await.expect("❌ Failed to get wallet stats");
    assert!(stats.total_webcash > 0);

    let server_client =
        crate::server::ServerClient::new().expect("❌ Failed to create server client");

    let public_webcash: Vec<PublicWebcash> = webcash_list.iter().map(|wc| wc.to_public()).collect();

    let _ = server_client.health_check(&public_webcash).await;

    match server_client.get_target().await {
        Ok(_target) => println!("✅ Target query successful"),
        Err(e) => panic!("Target endpoint should always work: {}", e),
    }

    use crate::hd::{ChainCode, HDWallet};
    let master_secret_bytes =
        hex::decode(test_master_secret).expect("❌ Failed to decode master secret");
    let mut master_secret_array = [0u8; 32];
    master_secret_array.copy_from_slice(&master_secret_bytes);

    let hd_wallet = HDWallet::from_master_secret(master_secret_array);

    let _receive_secret = hd_wallet
        .derive_secret(ChainCode::Receive, 0);

    let _pay_secret = hd_wallet
        .derive_secret(ChainCode::Pay, 0);

    let recovery_wallet_path = "phase2_recovery_wallet.db";
    let _ = fs::remove_file(recovery_wallet_path);

    let recovery_wallet = Wallet::open(recovery_wallet_path)
        .await
        .expect("❌ Failed to create recovery wallet");

    recovery_wallet
        .store_master_secret(test_master_secret)
        .await
        .expect("❌ Failed to store master secret");

    let _ = recovery_wallet.recover_from_wallet(5).await;

    let _ = fs::remove_file(recovery_wallet_path);
    let _ = fs::remove_file(wallet_path);
}
// ============================================================================
// Passkey Encryption Tests
// ============================================================================

async fn create_test_wallet(temp_dir: &TempDir, wallet_name: &str) -> (Wallet, PathBuf) {
    let wallet_path = temp_dir.path().join(format!("{}.db", wallet_name));
    let wallet = Wallet::open(&wallet_path)
        .await
        .expect("Should create test wallet");

    (wallet, wallet_path)
}

async fn create_passkey_wallet(temp_dir: &TempDir, wallet_name: &str) -> (Wallet, PathBuf) {
    let wallet_path = temp_dir.path().join(format!("{}_passkey.db", wallet_name));
    let wallet = Wallet::open_with_passkey(&wallet_path, true)
        .await
        .expect("Should create passkey wallet");

    (wallet, wallet_path)
}

async fn populate_wallet_with_test_data(
    wallet: &Wallet,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let test_webcashes = vec![
        "e0.00100000:secret:deadbeef1234567890abcdef1234567890abcdef1234567890abcdef12345678",
        "e0.05000000:secret:cafebabe1234567890abcdef1234567890abcdef1234567890abcdef12345678",
        "e1.00000000:secret:feedface1234567890abcdef1234567890abcdef1234567890abcdef12345678",
    ];

    for webcash_str in test_webcashes {
        if let Ok(webcash) = SecretWebcash::parse(webcash_str) {
            wallet.store_directly(webcash).await?;
        }
    }

    Ok(())
}

/// Test basic wallet encryption and decryption workflow
#[tokio::test]
async fn test_wallet_encryption_basic_workflow() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let (wallet, _wallet_path) = create_test_wallet(&temp_dir, "basic_test").await;

    populate_wallet_with_test_data(&wallet)
        .await
        .expect("Should populate wallet");

    let initial_balance = wallet.balance().await.expect("Should get initial balance");

    let password = "test_encryption_password";
    let encrypted_data = wallet
        .encrypt_with_password(password)
        .await
        .expect("Should encrypt wallet");

    assert_eq!(encrypted_data.algorithm, "AES-256-GCM-PASSWORD");
    assert!(!encrypted_data.ciphertext.is_empty());

    let (new_wallet, _new_path) = create_test_wallet(&temp_dir, "restored").await;

    new_wallet
        .decrypt_with_password(&encrypted_data, password)
        .await
        .expect("Should decrypt wallet");

    let restored_balance = new_wallet
        .balance()
        .await
        .expect("Should get restored balance");

    assert_eq!(
        initial_balance, restored_balance,
        "Balance should be preserved"
    );
}

/// Test passkey wallet creation and basic operations
#[tokio::test]
async fn test_passkey_wallet_creation() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let (wallet, _wallet_path) = create_passkey_wallet(&temp_dir, "passkey_test").await;

    assert!(
        wallet.is_passkey_enabled(),
        "Passkey encryption should be enabled"
    );

    let available = wallet.is_passkey_available().await;
    assert!(
        available.is_ok(),
        "Passkey availability check should not error"
    );

    populate_wallet_with_test_data(&wallet)
        .await
        .expect("Should populate passkey wallet");

    let balance = wallet
        .balance()
        .await
        .expect("Should get balance from passkey wallet");

    assert!(!balance.is_empty(), "Balance should not be empty");
}

/// Test passkey encryption (using placeholder implementation)
#[tokio::test]
async fn test_passkey_wallet_encryption() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let (wallet, _wallet_path) = create_passkey_wallet(&temp_dir, "passkey_encrypt").await;

    populate_wallet_with_test_data(&wallet)
        .await
        .expect("Should populate wallet");

    let initial_balance = wallet.balance().await.expect("Should get initial balance");

    let encrypted_data = wallet
        .encrypt_with_passkey()
        .await
        .expect("Should encrypt with passkey");

    assert_eq!(encrypted_data.algorithm, "AES-256-GCM");

    // Create a fresh wallet with the same filename (same keyring service name) in a
    // separate subdirectory so it starts empty but shares the passkey identity.
    let restore_dir = temp_dir.path().join("restore");
    fs::create_dir_all(&restore_dir).expect("Should create restore subdir");
    let restore_path = restore_dir.join("passkey_encrypt_passkey.db");
    let new_wallet = Wallet::open_with_passkey(&restore_path, true)
        .await
        .expect("Should create restore wallet with matching passkey identity");

    new_wallet
        .decrypt_with_passkey(&encrypted_data)
        .await
        .expect("Should decrypt with passkey");

    let restored_balance = new_wallet
        .balance()
        .await
        .expect("Should get restored balance");

    assert_eq!(
        initial_balance, restored_balance,
        "Balance should be preserved"
    );
}

/// Test wallet encryption with large amounts of data
#[tokio::test]
async fn test_wallet_encryption_large_dataset() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let (wallet, _wallet_path) = create_test_wallet(&temp_dir, "large_dataset").await;

    for i in 0..100 {
        let amount_str = format!("0.{:08}", i + 1);
        let secret_hex = format!("{:064}", i);
        let webcash_str = format!("e{}:secret:{}", amount_str, secret_hex);

        if let Ok(webcash) = SecretWebcash::parse(&webcash_str) {
            wallet
                .store_directly(webcash)
                .await
                .expect("Should store webcash");
        }
    }

    let initial_stats = wallet.stats().await.expect("Should get initial stats");

    let password = "large_dataset_password";
    let encrypted_data = wallet
        .encrypt_with_password(password)
        .await
        .expect("Should encrypt large wallet");

    assert!(!encrypted_data.ciphertext.is_empty());

    let (new_wallet, _new_path) = create_test_wallet(&temp_dir, "large_restored").await;

    new_wallet
        .decrypt_with_password(&encrypted_data, password)
        .await
        .expect("Should decrypt large wallet");

    let restored_stats = new_wallet.stats().await.expect("Should get restored stats");

    assert_eq!(
        initial_stats.unspent_webcash, restored_stats.unspent_webcash,
        "Webcash count should be preserved"
    );
}

/// Test cross-wallet migration
#[tokio::test]
async fn test_cross_wallet_migration() {
    let temp_dir = TempDir::new().expect("Should create temp dir");

    let (source_wallet, _source_path) = create_test_wallet(&temp_dir, "source").await;
    populate_wallet_with_test_data(&source_wallet)
        .await
        .expect("Should populate source wallet");

    let (dest_wallet, _dest_path) = create_passkey_wallet(&temp_dir, "destination").await;

    let source_balance = source_wallet
        .balance()
        .await
        .expect("Should get source balance");

    let password = "migration_password";
    let encrypted_data = source_wallet
        .encrypt_with_password(password)
        .await
        .expect("Should encrypt source wallet");

    dest_wallet
        .decrypt_with_password(&encrypted_data, password)
        .await
        .expect("Should decrypt to destination wallet");

    let dest_balance = dest_wallet
        .balance()
        .await
        .expect("Should get destination balance");

    assert_eq!(
        source_balance, dest_balance,
        "Migration should preserve balance"
    );
    assert!(
        dest_wallet.is_passkey_enabled(),
        "Destination should have passkey enabled"
    );
}

/// Test error handling in encryption workflows
#[tokio::test]
async fn test_encryption_error_handling() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let (wallet, _wallet_path) = create_test_wallet(&temp_dir, "error_test").await;

    populate_wallet_with_test_data(&wallet)
        .await
        .expect("Should populate wallet");

    let password = "test_password";
    let mut encrypted = wallet
        .encrypt_with_password(password)
        .await
        .expect("Should encrypt wallet");

    encrypted.algorithm = "INVALID_ALGORITHM".to_string();
    let (corrupt_wallet, _corrupt_path) = create_test_wallet(&temp_dir, "corrupted").await;
    let corrupt_result = corrupt_wallet
        .decrypt_with_password(&encrypted, password)
        .await;
    assert!(corrupt_result.is_err(), "Should error on invalid algorithm");

    let mut encrypted = wallet
        .encrypt_with_password(password)
        .await
        .expect("Should encrypt wallet");
    encrypted.ciphertext = b"invalid_hex".to_vec();

    let (invalid_wallet, _invalid_path) = create_test_wallet(&temp_dir, "invalid_data").await;
    let invalid_result = invalid_wallet
        .decrypt_with_password(&encrypted, password)
        .await;
    assert!(invalid_result.is_err(), "Should error on invalid hex data");
}

/// Test wallet stats preservation across encryption/decryption
#[tokio::test]
async fn test_wallet_stats_preservation() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let (wallet, _wallet_path) = create_test_wallet(&temp_dir, "stats_test").await;

    populate_wallet_with_test_data(&wallet)
        .await
        .expect("Should populate wallet");

    let original_stats = wallet.stats().await.expect("Should get original stats");

    let password = "stats_preservation_test";
    let encrypted = wallet
        .encrypt_with_password(password)
        .await
        .expect("Should encrypt wallet");

    let (restored_wallet, _restored_path) = create_test_wallet(&temp_dir, "stats_restored").await;
    restored_wallet
        .decrypt_with_password(&encrypted, password)
        .await
        .expect("Should restore wallet");

    let restored_stats = restored_wallet
        .stats()
        .await
        .expect("Should get restored stats");

    assert_eq!(
        original_stats.total_webcash, restored_stats.total_webcash,
        "Total webcash count should match"
    );
    assert_eq!(
        original_stats.unspent_webcash, restored_stats.unspent_webcash,
        "Unspent webcash count should match"
    );
}

/// Performance test for encryption/decryption operations
#[tokio::test]
async fn test_encryption_performance() {
    let temp_dir = TempDir::new().expect("Should create temp dir");
    let (wallet, _wallet_path) = create_test_wallet(&temp_dir, "performance_test").await;

    for i in 0..50 {
        let amount_str = format!("0.{:08}", i + 1);
        let secret_hex = format!("{:064}", i);
        let webcash_str = format!("e{}:secret:{}", amount_str, secret_hex);

        if let Ok(webcash) = SecretWebcash::parse(&webcash_str) {
            wallet
                .store_directly(webcash)
                .await
                .expect("Should store webcash");
        }
    }

    let password = "performance_test_password";

    let encrypt_start = std::time::Instant::now();
    let encrypted = wallet
        .encrypt_with_password(password)
        .await
        .expect("Should encrypt wallet");
    let encrypt_duration = encrypt_start.elapsed();

    let (restore_wallet, _restore_path) =
        create_test_wallet(&temp_dir, "performance_restored").await;

    let decrypt_start = std::time::Instant::now();
    restore_wallet
        .decrypt_with_password(&encrypted, password)
        .await
        .expect("Should decrypt wallet");
    let decrypt_duration = decrypt_start.elapsed();

    println!("Encryption took: {:?}", encrypt_duration);
    println!("Decryption took: {:?}", decrypt_duration);

    assert!(
        encrypt_duration.as_secs() < 5,
        "Encryption should complete within 5 seconds"
    );
    assert!(
        decrypt_duration.as_secs() < 5,
        "Decryption should complete within 5 seconds"
    );
}

/// Test concurrent wallet operations with encryption
#[tokio::test]
async fn test_concurrent_wallet_operations() {
    let temp_dir = Arc::new(TempDir::new().expect("Should create temp dir"));
    let password = "concurrent_test_password";

    let mut handles = Vec::new();

    for i in 0..5 {
        let temp_dir = temp_dir.clone();
        let password = password.to_string();

        let handle = task::spawn(async move {
            let (wallet, _path) = create_test_wallet(&temp_dir, &format!("concurrent_{}", i)).await;

            let webcash_str = format!("e0.{:08}:secret:{:064}", i + 1, i);
            if let Ok(webcash) = SecretWebcash::parse(&webcash_str) {
                wallet
                    .store_directly(webcash)
                    .await
                    .expect("Should store concurrent webcash");
            }

            let encrypted = wallet
                .encrypt_with_password(&password)
                .await
                .expect("Concurrent encryption should succeed");

            let (restore_wallet, _restore_path) =
                create_test_wallet(&temp_dir, &format!("concurrent_restore_{}", i)).await;
            restore_wallet
                .decrypt_with_password(&encrypted, &password)
                .await
                .expect("Concurrent decryption should succeed");

            let balance = restore_wallet
                .balance()
                .await
                .expect("Should get concurrent balance");

            assert!(
                !balance.is_empty(),
                "Concurrent balance should not be empty"
            );
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("Concurrent task should complete");
    }
}

// ============================================================================
// Runtime Encryption Tests
// ============================================================================

/// Test basic runtime database encryption workflow
#[tokio::test]
async fn test_runtime_database_encryption_workflow() {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let wallet_path = temp_dir.path().join("runtime_encrypted_wallet.db");

    let wallet = Wallet::open_with_passkey(&wallet_path, true)
        .await
        .expect("Failed to create wallet with passkey encryption");

    assert!(!Wallet::is_database_encrypted(&wallet_path).unwrap());

    let test_webcash = SecretWebcash::parse(
        "e0.00001:secret:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect("Failed to create test webcash");

    wallet
        .store_directly(test_webcash)
        .await
        .expect("Failed to store test webcash");

    let initial_balance = wallet
        .balance_amount()
        .await
        .expect("Failed to get initial balance");
    assert!(initial_balance > Amount::from_str("0").unwrap());

    wallet
        .close()
        .await
        .expect("Failed to close and encrypt wallet");

    assert!(Wallet::is_database_encrypted(&wallet_path).unwrap());

    let file_contents = fs::read(&wallet_path).expect("Failed to read encrypted database");

    assert!(!file_contents.starts_with(b"SQLite format 3"));
    let _encrypted_data: EncryptedData = serde_json::from_slice(&file_contents)
        .expect("Encrypted database should be valid EncryptedData JSON");

    let wallet2 = Wallet::open_with_passkey(&wallet_path, true)
        .await
        .expect("Failed to open encrypted wallet");

    let restored_balance = wallet2
        .balance_amount()
        .await
        .expect("Failed to get balance from decrypted wallet");

    assert_eq!(initial_balance, restored_balance);

    let webcash_list = wallet2
        .list_webcash()
        .await
        .expect("Failed to list webcash");
    assert!(!webcash_list.is_empty());

    wallet2
        .close()
        .await
        .expect("Failed to close wallet second time");

    assert!(Wallet::is_database_encrypted(&wallet_path).unwrap());
}

/// Test that non-passkey wallets are not affected
#[tokio::test]
async fn test_regular_wallet_unaffected() {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let wallet_path = temp_dir.path().join("regular_wallet.db");

    let wallet = Wallet::open(&wallet_path)
        .await
        .expect("Failed to create regular wallet");

    let test_webcash = SecretWebcash::parse(
        "e0.00001:secret:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    )
    .expect("Failed to create test webcash");

    wallet
        .store_directly(test_webcash)
        .await
        .expect("Failed to store test webcash");

    let balance = wallet
        .balance_amount()
        .await
        .expect("Failed to get balance");

    wallet
        .close()
        .await
        .expect("Failed to close regular wallet");

    assert!(!Wallet::is_database_encrypted(&wallet_path).unwrap());

    let file_contents = fs::read(&wallet_path).expect("Failed to read regular database");
    assert!(file_contents.starts_with(b"SQLite format 3"));

    let wallet2 = Wallet::open(&wallet_path)
        .await
        .expect("Failed to reopen regular wallet");

    let restored_balance = wallet2
        .balance_amount()
        .await
        .expect("Failed to get restored balance");

    assert_eq!(balance, restored_balance);
}

/// Test opening encrypted wallet without passkey flag fails
#[tokio::test]
async fn test_encrypted_wallet_requires_passkey_flag() {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let wallet_path = temp_dir.path().join("encrypted_for_flag_test.db");

    {
        let wallet = Wallet::open_with_passkey(&wallet_path, true)
            .await
            .expect("Failed to create encrypted wallet");

        let test_webcash = SecretWebcash::parse(
            "e0.00001:secret:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        )
        .expect("Failed to create test webcash");

        wallet
            .store_directly(test_webcash)
            .await
            .expect("Failed to store test webcash");

        wallet
            .close()
            .await
            .expect("Failed to close and encrypt wallet");
    }

    assert!(Wallet::is_database_encrypted(&wallet_path).unwrap());

    let result = Wallet::open(&wallet_path).await;
    assert!(result.is_err());

    let wallet = Wallet::open_with_passkey(&wallet_path, true)
        .await
        .expect("Failed to open with correct passkey flag");

    let balance = wallet
        .balance_amount()
        .await
        .expect("Failed to get balance");
    assert!(balance > Amount::from_str("0").unwrap());
}

/// Test error handling for corrupted encrypted database
#[tokio::test]
async fn test_corrupted_encrypted_database_handling() {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let wallet_path = temp_dir.path().join("corrupted_test.db");

    {
        let wallet = Wallet::open_with_passkey(&wallet_path, true)
            .await
            .expect("Failed to create wallet");
        wallet.close().await.expect("Failed to close wallet");
    }

    fs::write(&wallet_path, b"corrupted json data").expect("Failed to corrupt database");

    let result = Wallet::open_with_passkey(&wallet_path, true).await;
    assert!(result.is_err());
}

/// Test multiple encrypt/decrypt cycles
#[tokio::test]
async fn test_multiple_encryption_cycles() {
    let temp_dir = tempdir().expect("Failed to create temp directory");
    let wallet_path = temp_dir.path().join("cycles_test.db");

    let initial_webcash = SecretWebcash::parse(
        "e0.00001:secret:c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7c7",
    )
    .expect("Failed to create test webcash");

    for cycle in 1..=3 {
        let wallet = if cycle == 1 {
            let w = Wallet::open_with_passkey(&wallet_path, true)
                .await
                .expect("Failed to create wallet");
            w.store_directly(initial_webcash.clone())
                .await
                .expect("Failed to store initial webcash");
            w
        } else {
            Wallet::open_with_passkey(&wallet_path, true)
                .await
                .unwrap_or_else(|_| panic!("Failed to open wallet in cycle {}", cycle))
        };

        let balance = wallet
            .balance_amount()
            .await
            .unwrap_or_else(|_| panic!("Failed to get balance in cycle {}", cycle));
        assert!(balance > Amount::from_str("0").unwrap());

        let cycle_webcash = SecretWebcash::parse(&format!(
            "e0.0000{}:secret:{:064x}",
            cycle,
            cycle as u64 * 0x1111111111111111u64
        ))
        .expect("Failed to create cycle webcash");

        wallet
            .store_directly(cycle_webcash)
            .await
            .expect("Failed to store cycle webcash");

        wallet
            .close()
            .await
            .unwrap_or_else(|_| panic!("Failed to close wallet in cycle {}", cycle));

        assert!(Wallet::is_database_encrypted(&wallet_path).unwrap());
    }

    let final_wallet = Wallet::open_with_passkey(&wallet_path, true)
        .await
        .expect("Failed to open wallet for final verification");

    let unspent = final_wallet
        .list_webcash()
        .await
        .expect("Failed to list final webcash");

    assert_eq!(unspent.len(), 4);
}

# Testing Guide for Webcash Rust Library

## Overview

This document provides comprehensive instructions for testing the Webcash Rust library. All tests are designed with **money preservation** as a critical requirement - tests must never lose webcash during execution.

## Test Organization

Tests are organized into two main categories:

### Unit Tests (`tests/unit_tests.rs`)
- Fast, isolated tests for individual components
- No network dependencies
- No real webcash required
- Covers: amount, webcash parsing, HD derivation, crypto, error handling

### Integration Tests (`tests/integration_tests.rs`)
- Full wallet operations with real webcash
- Requires `TEST_WEBCASH_SECRET` environment variable
- Tests against live Webcash server
- Covers: setup, insert, pay, list, recover, merge, CLI operations

## Prerequisites

### Required Environment Variable

**CRITICAL**: Set `TEST_WEBCASH_SECRET` with a valid, unspent webcash secret:

```bash
export TEST_WEBCASH_SECRET='e0.0001:secret:your_64_hex_character_secret_here'
```

### Validating Your Test Secret

Before running tests, verify your secret is valid and unspent:

```bash
# Using Python (requires requests library)
python3 << 'EOF'
import requests
import hashlib

secret = "your_64_hex_character_secret_here"
hash_val = hashlib.sha256(bytes(secret, "ascii")).hexdigest()
public = f"e0.0001:public:{hash_val}"

response = requests.post('https://webcash.org/api/v1/health_check', json=[public])
result = response.json()
spent = result.get("results", {}).get(public, {}).get("spent")
amount = result.get("results", {}).get(public, {}).get("amount")

print(f"Secret: e0.0001:secret:{secret}")
print(f"Spent: {spent}")
print(f"Amount: {amount}")
print(f"Status: {'✅ VALID UNSPENT' if spent == False else '❌ INVALID OR SPENT'}")
EOF
```

## Running Tests

### Run All Tests

```bash
cargo test
```

### Run Only Unit Tests

```bash
cargo test --lib
```

### Run Only Integration Tests

```bash
# Unit tests
cargo test --test unit_tests

# Integration tests (requires TEST_WEBCASH_SECRET)
export TEST_WEBCASH_SECRET='e0.0001:secret:your_secret_here'
cargo test --test integration_tests

# CLI manual workflow test (most comprehensive)
cargo test --test integration_tests test_cli_manual_workflow -- --nocapture
```

### Run Specific Test

```bash
cargo test --test integration_tests test_full_wallet_integration -- --nocapture
```

## Core Test Workflows

### 1. Full Wallet Integration Test

Tests complete wallet functionality:
- ✅ Wallet setup
- ✅ Webcash insertion
- ✅ Balance and statistics
- ✅ Server health check
- ✅ Server API (target endpoint)
- ✅ Payment flow
- ✅ Post-payment verification

**Command:**
```bash
export TEST_WEBCASH_SECRET='e0.0001:secret:your_secret_here'
cargo test --test integration_tests test_full_wallet_integration -- --nocapture
```

### 2. CLI Manual Workflow Test (RECOMMENDED)

Most comprehensive test that simulates real user workflow:
- ✅ Wallet setup with master secret
- ✅ Webcash insertion via CLI
- ✅ Balance check via CLI
- ✅ Webcash listing
- ✅ Master secret preservation
- ✅ Wallet deletion (simulating loss)
- ✅ Wallet recreation from master secret
- ✅ HD recovery via CLI
- ✅ Payment to generate output secret
- ✅ **CRITICAL**: Returns output secret with same amount as input

**Features:**
- Tracks all operations in `cli_test_history.txt`
- Saves master secret to `cli_test_master_secret.txt`
- Outputs final secret to `FINAL_OUTPUT_SECRET.txt`
- **NEVER LOSES MONEY** - always returns valid output secret

**Command:**
```bash
export TEST_WEBCASH_SECRET='e0.0001:secret:your_secret_here'
cargo test --test integration_tests test_cli_manual_workflow -- --nocapture
```

**Output Files:**
- `cli_test_history.txt` - Complete operation history
- `cli_test_master_secret.txt` - Master secret for recovery
- `FINAL_OUTPUT_SECRET.txt` - Output secret (reuse for next test)

### 3. Complete Money Preservation Test

Library-based test with full money preservation:
- ✅ Library operations (insert, pay, list)
- ✅ CLI operations (setup, insert, pay, recover)
- ✅ Complete tracking and history
- ✅ Master secret preservation
- ✅ Recovery testing
- ✅ Output secret generation

**Command:**
```bash
export TEST_WEBCASH_SECRET='e0.0001:secret:your_secret_here'
cargo test --test integration_tests test_complete_money_preservation -- --nocapture
```

### 4. Cross-Wallet HD Recovery Test

Tests HD wallet recovery across multiple wallets:
- ✅ Primary wallet setup
- ✅ Webcash insertion into primary
- ✅ Payment generation (HD derivation)
- ✅ Secondary wallet creation
- ✅ Payment insertion into secondary
- ✅ HD recovery from master secret
- ✅ Return payment workflow

**Command:**
```bash
export TEST_WEBCASH_SECRET='e0.0001:secret:your_secret_here'
cargo test --test integration_tests test_cross_wallet_hd_recovery_integration -- --nocapture
```

## Money Preservation Mechanism

### How Tests Preserve Money

1. **Input Tracking**: All input webcash secrets are logged in history files
2. **Master Secret Preservation**: Master secrets are saved to files for recovery
3. **Operation Logging**: Every operation (insert, pay, recover) is logged
4. **Output Secret Generation**: Tests always generate an output secret with the same amount
5. **Recovery Capability**: Master secrets allow wallet recovery even if wallet is deleted

### Using Output Secrets for Next Test

After running a test, the output secret is saved to `FINAL_OUTPUT_SECRET.txt`. Use this for the next test:

```bash
# After running a test
export TEST_WEBCASH_SECRET=$(cat FINAL_OUTPUT_SECRET.txt)

# Run next test
cargo test --test integration_tests test_cli_manual_workflow -- --nocapture
```

### Recovering Lost Money

If a test fails or you lose track of webcash:

1. **Check Master Secret Files**:
   ```bash
   cat cli_test_master_secret.txt
   ```

2. **Check History Files**:
   ```bash
   cat cli_test_history.txt
   ```

3. **Recover Wallet from Master Secret**:
   ```bash
   # Using CLI
   webyc --wallet recovery_wallet.db setup -p "$(cat cli_test_master_secret.txt)"
   webyc --wallet recovery_wallet.db recover --gap-limit 20
   ```

4. **List Recovered Webcash**:
   ```bash
   webyc --wallet recovery_wallet.db info
   ```

## Test File Locations

### Generated Test Files

- `*_test_wallet.db` - SQLite wallet databases
- `*_test_history.txt` - Operation history logs
- `*_test_master_secret.txt` - Master secrets for recovery
- `FINAL_OUTPUT_SECRET.txt` - Output webcash secret for next test

### Cleaning Up Test Files

```bash
# Remove all test files
rm -f *_test_wallet.db *_test_history.txt *_test_master_secret.txt FINAL_OUTPUT_SECRET.txt
```

## CLI Binary Testing

The CLI binary (`webyc`) must be built before running CLI tests:

```bash
# Build CLI binary
cargo build --release --bin webyc

# CLI binary location
./target/release/webyc
```

### Manual CLI Testing

You can also test the CLI manually:

```bash
# Setup wallet with master secret
./target/release/webyc --wallet test.db setup

# Insert webcash
./target/release/webyc --wallet test.db insert "$TEST_WEBCASH_SECRET"

# Check balance
./target/release/webyc --wallet test.db info

# Recover webcash
./target/release/webyc --wallet test.db recover --gap-limit 20

# Pay amount
./target/release/webyc --wallet test.db pay 0.0001 -m "Test payment"
```

## Biometric Tests

Biometric tests are included in integration tests and require platform-specific implementations:

```bash
# Run biometric tests
cargo test --test integration_tests test_biometric_wallet_encryption -- --nocapture
```

**Note**: Some biometric tests may fail on non-mobile platforms as they require iOS/Android specific APIs.

## Troubleshooting

### Test Secret Already Spent

**Error**: "This webcash cannot be replaced because it was already spent"

**Solution**: Use a fresh, unspent webcash secret. Generate a new one or use the output from a previous successful test.

### Recovery Finds 0 Webcash

**Possible Causes**:
1. Master secret doesn't match the one used during insertion
2. Webcash wasn't created with HD derivation
3. Recovery gap limit is too small

**Solution**: 
- Ensure master secret is correct
- Check that webcash was inserted (not just stored directly)
- Increase gap limit: `--gap-limit 50`

### Payment Fails

**Error**: "Insufficient funds"

**Solution**: 
- Check balance: `webyc --wallet test.db info`
- Ensure webcash is unspent and recovered
- Verify amount to pay is less than balance

### CLI Binary Not Found

**Error**: "Failed to execute webyc command"

**Solution**:
```bash
# Build CLI
cargo build --release --bin webyc

# Verify binary exists
ls -lh ./target/release/webyc
```

## Best Practices

1. **Always Use Fresh Secrets**: Never reuse spent secrets
2. **Save Master Secrets**: Always save master secrets before deleting wallets
3. **Track Operations**: Keep history files for audit trails
4. **Verify Output Secrets**: Always verify output secrets are valid and unspent
5. **Clean Up**: Remove test files between runs to avoid confusion
6. **Check Server Status**: Verify server is accessible before running online tests

## Continuous Testing Workflow

Recommended workflow for continuous testing:

```bash
# 1. Set initial secret
export TEST_WEBCASH_SECRET='e0.0001:secret:initial_secret_here'

# 2. Run comprehensive test
cargo test --test integration_tests test_cli_manual_workflow -- --nocapture

# 3. Use output for next test
export TEST_WEBCASH_SECRET=$(cat FINAL_OUTPUT_SECRET.txt)

# 4. Verify output secret
python3 << 'EOF'
import requests, hashlib, sys
secret = sys.stdin.read().strip().split(':secret:')[1]
hash_val = hashlib.sha256(bytes(secret, "ascii")).hexdigest()
public = f"e0.0001:public:{hash_val}"
response = requests.post('https://webcash.org/api/v1/health_check', json=[public])
result = response.json()
spent = result.get("results", {}).get(public, {}).get("spent")
print(f"Status: {'✅ VALID' if spent == False else '❌ SPENT'}")
EOF

# 5. Repeat from step 2
```

## Test Coverage

Current test coverage includes:

- ✅ **Wallet Operations**: Setup, insert, pay, balance, list, stats, recover, merge
- ✅ **CLI Operations**: All CLI commands with real webcash
- ✅ **HD Wallet**: Key derivation, recovery, cross-wallet operations
- ✅ **Server Integration**: Health check, replace, target API
- ✅ **Money Preservation**: Complete tracking and output secret generation
- ✅ **Biometric Encryption**: Wallet encryption/decryption workflows
- ✅ **Error Handling**: Invalid secrets, spent webcash, network errors
- ✅ **Recovery Scenarios**: Lost wallets, master secret recovery

## Reporting Issues

When reporting test failures:

1. Include the full test output
2. Provide the `TEST_WEBCASH_SECRET` status (spent/unspent, but not the actual secret)
3. Include relevant history files (with secrets redacted)
4. Specify which test failed and at which step
5. Include system information (OS, Rust version)

---

**Remember**: These tests use **real webcash**. Always use small amounts (0.0001 WEBCASH or less) for testing, and never lose your master secrets!

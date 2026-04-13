# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-04-13

### Added
- **FFI layer** (`src/ffi/`) — full C ABI exports for cross-platform SDK consumption
  - `weby_wallet_open`, `weby_wallet_open_with_seed`, `weby_wallet_free`
  - `weby_wallet_balance`, `weby_wallet_insert`, `weby_wallet_pay`
  - `weby_wallet_check`, `weby_wallet_merge`, `weby_wallet_recover`
  - `weby_wallet_stats`, `weby_wallet_export_snapshot`
  - `weby_wallet_encrypt_seed`, `weby_amount_parse`, `weby_amount_format`
  - `weby_version`, `weby_last_error_message`, `weby_free_string`
  - Thread-local error message storage with `WebyErrorCode` enum
- `Wallet::open_with_seed()` — inject a caller-provided 32-byte seed
- `ChainCode::as_str()` — canonical string names (RECEIVE, PAY, CHANGE, MINING)
- `CheckResult` / `RecoveryResult` structured return types
- WAL mode (`PRAGMA journal_mode=WAL`) enabled on every wallet open
- `protocol.rs` — VERSION and TERMS_OF_SERVICE constants
- Feature flags: `bundled-sqlite`, `ffi`, `cli`, `passkey`
- `cdylib` + `staticlib` crate types for shared/static library builds
- CI: Windows, FreeBSD, MSRV 1.75, release builds, crates.io publish
- CI: iOS (aarch64-apple-ios) and Android (aarch64-linux-android) cross-compilation
- `log` crate integration — library never prints to stdout

### Changed
- **License changed from MPL-2.0 to MIT**
- **lib.rs is now pure re-exports** — no implementation code
- `wallet.rs` (1,615 lines) split into `wallet/` module:
  - `wallet/mod.rs` — struct definition, constructors
  - `wallet/schema.rs` — database initialization, WAL mode
  - `wallet/operations.rs` — insert, pay, merge, recover, check, balance
  - `wallet/encryption.rs` — database/seed encryption
  - `wallet/snapshot.rs` — JSON export/import
- `ChainCode` unified (was duplicated as `KeyType` in `hd.rs`)
- `HDWallet::derive_secret()` now takes `ChainCode` enum instead of raw `u64`
- All 57 `println!` calls in wallet replaced with `log::info!`/`log::debug!`
- `ServerClient` now implements `ServerClientTrait` — wallet owns it via trait object
- `check()` returns `CheckResult` (struct) instead of `()`
- `recover()` / `recover_from_wallet()` return `RecoveryResult` instead of `String`
- `HDWallet` now derives `Zeroize` + `ZeroizeOnDrop` (was manual)
- `endpoints` module moved from `lib.rs` to `server.rs`
- MSRV bumped from 1.70 to 1.75
- `clap` and `rpassword` are now optional (`cli` feature)

### Removed
- `src/server/ios.rs` — 166-line stub (every method returned error)
- `tests/unit/server_ios.rs` — tests for deleted stub
- `KeyType` enum (unified into `ChainCode`)
- `generate_receive_secret`, `generate_pay_secret`, `generate_change_secret`, `generate_mining_secret` — use `derive_secret(ChainCode::*, depth)` directly
- Dead dependencies: `aes = "0.7"`, `block-modes = "0.8"` (zero imports)

## [0.1.1] - 2025-12-01

### Added
- Biometric encryption module (password-based Argon2 + AES-256-GCM)
- HD wallet with 4-chain derivation (Python-compatible)
- SQLite-backed wallet with snapshot export/import
- CLI tool (`webyc`) with setup, insert, pay, check, recover, merge, encrypt, decrypt

## [0.1.0] - 2025-10-01

### Added
- Initial release: Amount type, SecretWebcash/PublicWebcash parsing, server client

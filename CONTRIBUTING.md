# Contributing to webylib

Thank you for your interest in contributing! This guide covers everything you need
to submit a high-quality pull request.

---

## Table of Contents

1. [Code of Conduct](#code-of-conduct)
2. [Getting Started](#getting-started)
3. [Development Setup](#development-setup)
4. [Running Tests](#running-tests)
5. [Commit Format](#commit-format)
6. [Pull Request Process](#pull-request-process)
7. [Design Principles](#design-principles)

---

## Code of Conduct

Be respectful and constructive. We welcome contributions from everyone.

---

## Getting Started

1. **Fork** the repository on GitHub.
2. **Clone** your fork:
   ```sh
   git clone https://github.com/YOUR-USERNAME/webylib.git
   cd webylib
   ```
3. **Add the upstream remote:**
   ```sh
   git remote add upstream https://github.com/webycash/webylib.git
   ```
4. **Create a branch** from `main`:
   ```sh
   git checkout -b fix/my-bug-description
   ```

Branch naming conventions:
- `fix/<description>` — bug fix
- `feat/<description>` — new feature
- `docs/<description>` — documentation only
- `refactor/<description>` — internal restructuring, no behaviour change
- `test/<description>` — tests only

---

## Development Setup

Minimum Rust version: **1.85** (MSRV — enforced in CI).

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable
```

No other dependencies required — SQLite is bundled, TLS is pure Rust.

```sh
# Build
cargo build

# Build with FFI
cargo build --features ffi
```

---

## Running Tests

```sh
# Unit tests (fast, no network)
cargo test --test unit_tests

# Format check
cargo fmt --check

# Lint (must pass with zero warnings)
cargo clippy --all-targets --no-default-features --features bundled-sqlite,cli -- -D warnings

# Full test suite (requires webcash server access)
cargo test
```

All of these must pass before a PR can be merged:
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test --test unit_tests`
- `cargo build --no-default-features --features bundled-sqlite` (MSRV check)

---

## Commit Format

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`

Scopes: `wallet`, `ffi`, `crypto`, `hd`, `server`, `amount`, `ci`

Examples:
```
feat(ffi): add weby_wallet_list_webcash export
fix(wallet): wrap pay operation in DB transaction
docs(ffi): add Kotlin JNA example
refactor(hd): unify ChainCode and KeyType enums
test(wallet): add encryption round-trip proof
```

---

## Pull Request Process

1. Keep PRs focused — one feature or fix per PR
2. Update `CHANGELOG.md` under `[Unreleased]`
3. Run the full test suite locally
4. Fill out the PR description with what changed and why
5. PRs are squash-merged to `main`

---

## Design Principles

| Principle | Description |
|-----------|-------------|
| **No singletons** | State is actor-owned. Each `Wallet` owns its DB connection and server client. |
| **No OpenSSL** | Pure Rust crypto and TLS only. Enables cross-compilation to all targets. |
| **Zeroize secrets** | All secret material implements `Zeroize + ZeroizeOnDrop`. |
| **Library never prints** | Use `log` crate, never `println!`. Consumers choose their logger. |
| **Structured returns** | Operations return typed results, not formatted strings. |
| **Pure re-export root** | `lib.rs` contains only `pub mod` and `pub use` — zero implementation. |
| **Feature-gated optionals** | CLI, FFI, passkey are behind feature flags. Core library has no optional deps. |

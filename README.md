<p align="center">
<pre>
                _           _ _ _
 __      _____ | |__  _   _| (_) |__
 \ \ /\ / / _ \| '_ \| | | | | | '_ \
  \ V  V /  __/| |_) | |_| | | | |_) |
   \_/\_/ \___||_.__/ \__, |_|_|_.__/
                      |___/
</pre>
</p>

<p align="center">
<em>Production-grade Webcash HD wallet library with full C FFI for cross-platform SDKs</em>
</p>

<p align="center">
<a href="https://crates.io/crates/webylib"><img src="https://img.shields.io/crates/v/webylib.svg" alt="crates.io"></a>
<a href="https://github.com/webycash/webylib/actions/workflows/ci.yml"><img src="https://github.com/webycash/webylib/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

---

## Install

**As a Rust dependency:**

```toml
[dependencies]
webylib = "0.3"
```

Check [crates.io/crates/webylib](https://crates.io/crates/webylib) for the latest published version.

**CLI tool (macOS / Linux):**

```bash
curl -sSf https://raw.githubusercontent.com/webycash/webylib/main/install.sh | sh
```

**CLI tool (Windows PowerShell):**

```powershell
iwr https://raw.githubusercontent.com/webycash/webylib/main/install.ps1 -UseB | iex
```

**From source (requires Rust 1.85+):**

```bash
cargo install webylib
```

## Quick Start

### Rust

```rust
use webylib::{Wallet, SecretWebcash, Amount};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create or open a wallet
    let wallet = Wallet::open("my_wallet.db").await?;

    // Insert webcash (ownership transfer via server)
    let wc = SecretWebcash::parse("e1.00000000:secret:abcdef...")?;
    wallet.insert(wc).await?;

    // Check balance
    let balance = wallet.balance().await?;
    println!("Balance: {}", balance);

    // Pay someone
    let payment = wallet.pay(Amount::from_str("0.5")?, "coffee").await?;
    println!("{}", payment);

    Ok(())
}
```

### C / FFI

```c
#include "webylib.h"

WebyWallet *wallet = NULL;
int rc = weby_wallet_open("my_wallet.db", &wallet);
if (rc != 0) {
    printf("Error: %s\n", weby_last_error_message());
    return 1;
}

char *balance = NULL;
weby_wallet_balance(wallet, &balance);
printf("Balance: %s\n", balance);
weby_free_string(balance);

weby_wallet_free(wallet);
```

### CLI

```bash
webyc setup
webyc info
webyc insert "e1.00000000:secret:abc123..."
webyc pay 0.5 --memo "coffee"
webyc check
webyc merge --group 20
```

## Features

- **HD Wallet** — BIP32-style 4-chain derivation (Receive, Pay, Change, Mining)
- **SQLite + WAL** — Crash-safe local storage with Write-Ahead Logging
- **Encryption** — Argon2 + AES-256-GCM for seed and database encryption
- **Full FFI** — C ABI exports for Python, Node.js, .NET, Go, Swift, Java, Kotlin, C++
- **Zeroize** — All secret material zeroed on drop
- **No OpenSSL** — Pure Rust TLS (rustls), cross-compiles everywhere
- **Exchange-ready** — Transactional safety, optional seed injection, structured error codes

## Multi-asset support (refactor/asset-traits)

In addition to the legacy webcash-only crate, this workspace ships an
**asset-gated wallet family** that pairs with the four-binary
[webycash-server](https://github.com/webycash/webycash-server) family
(server-webcash, server-rgb, server-rgb-collectible, server-voucher):

| Crate | Server flavor | Verbs |
|-------|---------------|-------|
| `webylib-wallet-webcash` | server-webcash | `pay`, `insert` |
| `webylib-wallet-rgb` | server-rgb / server-rgb-collectible | `transfer`, `insert` |
| `webylib-wallet-voucher` | server-voucher | `pay`, `insert` |

Plus a multi-asset CLI binary `webyca` that drives all three from one
command line:

```bash
# Read-only (flavor-agnostic — works against any server flavor)
webyca --server http://127.0.0.1:8181 target
webyca --server http://127.0.0.1:8181 check --tokens 'e1.0:public:HASH'

# Write (flavor-specific verb)
webyca --server http://127.0.0.1:8181 webcash pay --inputs e1.0:secret:abc \
    --outputs e0.6:secret:def,e0.4:secret:ghi
webyca --server http://127.0.0.1:8182 rgb transfer --inputs ... --outputs ...
webyca --server http://127.0.0.1:8183 voucher pay --inputs ... --outputs ...

# Burn / mining
webyca --server http://127.0.0.1:8181 burn --secret e1.0:secret:abc
webyca --server http://127.0.0.1:8181 mining-report --preimage '{"webcash":...}'
```

The wire format for RGB and Voucher tokens is
`e{amount}:secret:{hex}:{contract_id}:{issuer_pgp_fp}` —
the server enforces `(contract_id, issuer_fp)` namespace isolation on
every replace. Webcash stays bit-for-bit compatible with webcash.org
production. See the workspace
[ROADMAP](https://github.com/webycash/webycash-server/blob/refactor/asset-traits/ROADMAP.md#v040--asset-gated-server-family-refactorasset-traits-branch)
for the full list of asset-trait crates.

## Platform Support

| Platform | Architecture | Artifact |
|----------|-------------|----------|
| Linux | x86_64, aarch64 | `.so` + `.a` + CLI |
| macOS | x86_64, aarch64 | `.dylib` + `.a` + CLI |
| Windows | x86_64 (MSVC) | `.dll` + `.lib` + CLI |
| iOS | aarch64, aarch64-sim | `.a` (static lib) |
| Android | aarch64, armv7, x86_64 | `.so` (shared lib) |
| FreeBSD | x86_64 | CLI (test only) |

## Feature Flags

| Flag | Default | Description |
|------|---------|-------------|
| `bundled-sqlite` | yes | Compile SQLite from source (no system dep) |
| `cli` | yes | Build the `webyc` command-line tool |
| `ffi` | no | Build C FFI exports (`cdylib` + `staticlib`) |
| `passkey` | no | WebAuthn/passkey decryption support |

## Module Structure

```
src/
├── lib.rs              # Crate root — re-exports only
├── protocol.rs         # VERSION, TERMS_OF_SERVICE
├── amount.rs           # 8-decimal Amount type (wats)
├── webcash.rs          # SecretWebcash, PublicWebcash, SecureString
├── crypto.rs           # SHA256, HMAC-SHA512, CryptoSecret, AES-256-GCM
├── hd.rs               # HDWallet, ChainCode (4-chain derivation)
├── error.rs            # Error enum, Result type alias
├── server.rs           # ServerClient, ServerClientTrait, endpoints
├── biometric.rs        # BiometricEncryption, password encryption
├── wallet/
│   ├── mod.rs          # Wallet struct, open/open_with_seed/open_memory
│   ├── schema.rs       # DB schema, WAL mode, migrations
│   ├── operations.rs   # insert, pay, merge, recover, check, balance
│   ├── encryption.rs   # Database and seed encryption
│   └── snapshot.rs     # JSON export/import for backup
└── ffi/
    ├── mod.rs          # FFI module root and memory rules
    ├── error.rs        # WebyErrorCode, thread-local error messages
    ├── types.rs        # C string conversions, weby_free_string
    └── wallet_ops.rs   # All extern "C" wallet operations
```

## Documentation

| Document | Description |
|----------|-------------|
| [Architecture](docs/ARCHITECTURE.md) | System design, trust model, transaction flows |
| [FFI Reference](docs/FFI.md) | Complete C API reference with examples for 8 languages |
| [Protocol](docs/PROTOCOL.md) | Webcash protocol specification and API endpoints |
| [Security](docs/SECURITY.md) | Threat model, cryptographic guarantees, compliance |
| [Development](docs/DEVELOPMENT.md) | Build from source, run tests, CI pipeline |
| [CONTRIBUTING](CONTRIBUTING.md) | Pull request process, commit format, design principles |
| [CHANGELOG](CHANGELOG.md) | Version history and release notes |

## Building the FFI Library

```bash
# Shared library (.so / .dylib / .dll)
cargo build --release --features ffi

# Static library (.a / .lib)
cargo build --release --features ffi

# Generate C header
cbindgen --crate webylib --output include/webylib.h

# Both are produced in target/release/
```

## License

This project is licensed under the **MIT License** — see the [LICENSE](LICENSE) file for details.

## Related Projects

- [Webcash Protocol](https://webcash.org) — Official server and specification
- [kanzure/webcash](https://github.com/kanzure/webcash) — Python reference implementation
- [harmoniis-wallet](https://github.com/harmoniis/harmoniis-wallet) — Marketplace wallet built on webylib

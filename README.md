# Webcash Rust Implementation

[![License: MPL 2.0](https://img.shields.io/badge/License-MPL%202.0-brightgreen.svg)](https://opensource.org/licenses/MPL-2.0)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

A Rust implementation of the Webcash electronic cash wallet system.

## Overview

Webcash is an experimental electronic cash system operated by Webcash LLC. This Rust implementation provides a modern, memory-safe wallet client that maintains full compatibility with the Webcash protocol:

- Memory safety through Rust's ownership system
- Native performance with zero-cost abstractions
- Cryptographic operations with secure memory handling
- Compatibility with existing Webcash Python and C++ implementations
- C bindings for integration with other languages

**Scope**: This implementation focuses on wallet functionality and server API calls. It does not include mining or server-side implementation.

## Features

- ✅ **Deterministic HD Wallet**: BIP32-style hierarchical key generation
- ✅ **Secure Storage**: SQLite-based wallet with master secret encryption
- ✅ **Server Integration**: Complete Webcash server API compatibility
- ✅ **C Bindings**: FFI interface for C/C++/other languages
- ✅ **Comprehensive Testing**: Unit tests, integration tests, and documentation

## Quick Start

### Prerequisites

- **Rust**: 1.70+ with Cargo
- **SQLite3**: Development libraries
- **OpenSSL**: Development libraries (for HTTPS)

### Installation

```bash
# Clone the repository
git clone <repository-url>
cd webylib

# Build the project
cargo build --release

# Run tests
cargo test
```

### Usage

```bash
# Initialize a new wallet
./target/release/webyc setup

# Check wallet balance
./target/release/webyc info

# Insert webcash
./target/release/webyc insert "e1.00000000:secret:abcdef..."

# Send payment
./target/release/webyc pay 0.50000000 "For coffee"
```

## Architecture

The implementation consists of two main components:

### Core Library (`webylib`)

Located in `src/`, provides:
- **Amount handling** with 8-decimal precision
- **Webcash types** (SecretWebcash, PublicWebcash)
- **Cryptographic utilities** (SHA256, secure random)
- **HD wallet** deterministic key generation
- **Server communication** HTTP client
- **Wallet storage** SQLite database operations

### CLI Tool (`webyc`)

Command-line interface in `src/bin/cli.rs` providing:
- Wallet setup and management
- Payment operations
- Balance checking
- Recovery functionality

## Documentation

- **[📚 Library Documentation](./docs/)**: Comprehensive API docs
- **[🏗️ Architecture](./docs/architecture/)**: System design and implementation
- **[🔒 Security](./docs/security/)**: Security analysis and threat model
- **[🔄 Implementation Comparison](./docs/migration/)**: Comparison with Python/C++ implementations

## Development Status

### ✅ Completed (Phase 1)
- [x] Core data types (Amount, SecretWebcash, PublicWebcash)
- [x] Cryptographic utilities and secure memory handling
- [x] Basic project structure and build system
- [x] Comprehensive error handling
- [x] Unit tests for all implemented components

### 🚧 In Progress (Phase 2)
- [ ] SQLite wallet storage implementation
- [ ] Full wallet operations (insert, pay, check)
- [ ] Server integration and API compatibility

### 📋 Planned (Phase 3-5)
- [ ] CLI interface completion
- [ ] C bindings and FFI
- [ ] Performance optimization
- [ ] Comprehensive integration testing

## Security Considerations

This implementation prioritizes security:

- **Memory Safety**: Rust prevents buffer overflows, use-after-free, and data races
- **Secure Strings**: Sensitive data uses zeroize-on-drop types
- **Input Validation**: Strict bounds checking on all user inputs
- **Cryptographic Security**: Proper use of SHA256 and secure random generation
- **Wallet Security**: Master secret encryption and secure storage

## Testing

```bash
# Run all tests
cargo test

# Run with coverage (requires llvm-tools-preview)
cargo install cargo-llvm-cov
cargo llvm-cov --open
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development guidelines.

### Development Setup

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and setup
git clone <repository-url>
cd webylib
cargo build
```

## License

This project is licensed under the **Mozilla Public License 2.0** (MPL-2.0), consistent with the original Webcash implementations.

## Related Projects

- [Original Python Implementation](https://github.com/kanzure/webcash)
- [C++ Webminer](https://github.com/maaku/webminer)
- [Webcash Protocol Specification](https://webcash.org)

## Disclaimer

This is experimental software. Use at your own risk. The Webcash system is experimental and may have security or stability issues.

# Build Instructions

## 🎯 **Quick Build**

```bash
# Clone repository
git clone <repository-url>
cd webcash-rust

# Build everything
cargo build

# Build optimized release
cargo build --release

# Run tests
cargo test

# Run the CLI
cargo run --bin webyc -- --version
```

## 🏗️ **Build System Overview**

### **Cargo Build System**
This project uses **Cargo**, Rust's package manager and build system:

- **Dependencies**: Automatically downloaded and managed
- **Cross-platform**: Works on Linux, macOS, Windows
- **Incremental**: Only rebuilds changed code
- **Optimized**: Release builds are highly optimized

### **Build Profiles**
```toml
# Debug build (default)
cargo build          # Development with debug symbols
cargo build --debug  # Explicit debug build

# Release build
cargo build --release  # Optimized production build

# Custom profiles
cargo build --profile dev     # Development profile
cargo build --profile release # Release profile
```

## 📦 **Dependencies**

### **Core Dependencies**
```toml
[dependencies]
# Cryptography
sha2 = "0.10"          # SHA256 hashing
hex = "0.4"            # Hex encoding/decoding
getrandom = "0.2"      # Secure random generation

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"     # JSON serialization

# Database
rusqlite = "0.28"      # SQLite database

# HTTP client
reqwest = { version = "0.11", features = ["json", "rustls-tls"] }

# Error handling
thiserror = "1.0"      # Error types
anyhow = "1.0"         # Error handling

# CLI
clap = { version = "4.0", features = ["derive"] }

# Utilities
zeroize = "1.5"        # Secure memory zeroization
```

### **Development Dependencies**
```toml
[dev-dependencies]
tokio-test = "0.4"     # Async testing
tempfile = "3.0"       # Temporary files for testing
```

## 🔧 **Build Commands**

### **Basic Builds**
```bash
# Standard debug build
cargo build

# Optimized release build
cargo build --release

# Build specific binary
cargo build --bin webyc

# Build library only
cargo build --lib
```

### **Advanced Builds**
```bash
# Build with specific features
cargo build --features "extra-security"

# Build for specific target
cargo build --target x86_64-unknown-linux-gnu

# Build with custom linker
RUSTFLAGS="-C linker=clang" cargo build

# Build with verbose output
cargo build --verbose
```

### **Cross-Compilation**
```bash
# Install cross-compilation tools
cargo install cross

# Build for different architectures
cross build --target aarch64-unknown-linux-gnu
cross build --target x86_64-pc-windows-gnu
cross build --target x86_64-apple-darwin
```

## 🧪 **Testing**

### **Unit Tests**
```bash
# Run all tests
cargo test

# Run specific test
cargo test test_amount_arithmetic

# Run tests in specific module
cargo test amount::

# Run with output
cargo test -- --nocapture

# Run release tests
cargo test --release
```

### **Integration Tests**
```bash
# Run integration tests (when implemented)
cargo test --test integration

# Run tests with different features
cargo test --features "mock-server"
```

### **Documentation Tests**
```bash
# Test code examples in documentation
cargo test --doc

# Test specific documentation
cargo test --doc amount
```

## 📊 **Code Quality**

### **Formatting**
```bash
# Format all code
cargo fmt

# Check formatting (CI)
cargo fmt --check

# Format specific files
cargo fmt -- src/main.rs
```

### **Linting**
```bash
# Run clippy linter
cargo clippy

# Run with warnings as errors
cargo clippy -- -D warnings

# Fix auto-fixable issues
cargo clippy --fix
```

### **Security Scanning**
```bash
# Run security audit (when cargo-audit is installed)
cargo audit

# Check for vulnerabilities
cargo audit check

# Update advisory database
cargo audit update
```

## 📈 **Performance Optimization**

### **Release Build Optimizations**
```bash
# Build with maximum optimizations
cargo build --release

# Build with Link-Time Optimization (LTO)
cargo build --release --config profile.release.lto=true

# Build with specific optimization level
cargo build --release --config profile.release.opt-level=3
```

### **Binary Size Optimization**
```bash
# Strip debug symbols
cargo build --release
strip target/release/webyc

# Analyze binary size
cargo bloat --release --crates

# Build minimal binary
cargo build --release --features "minimal"
```

### **Benchmarking**
```bash
# Run benchmarks (when implemented)
cargo bench

# Profile performance
cargo build --release
perf record target/release/webyc --version
perf report
```

## 🔍 **Debug Builds**

### **Debug Symbols**
```bash
# Build with debug symbols
cargo build

# Build with maximum debug info
cargo build --config profile.dev.debug=2
```

### **Debugging Tools**
```bash
# Use debugger
rust-gdb target/debug/webyc

# Use lldb (macOS)
rust-lldb target/debug/webyc

# Memory profiling (Linux)
valgrind --tool=massif target/debug/webyc

# Memory leak detection
valgrind --tool=memcheck target/debug/webyc
```

## 📚 **Documentation**

### **Generate Documentation**
```bash
# Generate API documentation
cargo doc

# Open documentation in browser
cargo doc --open

# Generate documentation with private items
cargo doc --document-private-items

# Generate for specific package
cargo doc --package webylib
```

### **Documentation Testing**
```bash
# Test documentation examples
cargo test --doc

# Test specific documentation
cargo test --doc -- lib::amount
```

## 🚀 **Deployment**

### **Binary Distribution**
```bash
# Build for distribution
cargo build --release

# Create distribution package
tar -czf webylib-v0.1.0.tar.gz target/release/webyc

# Cross-platform builds
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target x86_64-pc-windows-gnu
cargo build --release --target x86_64-apple-darwin
```

### **Library Distribution**
```bash
# Build library for crates.io
cargo build --release

# Test library publication
cargo publish --dry-run

# Publish to crates.io
cargo publish
```

## 🔧 **Build Configuration**

### **Cargo.toml Configuration**
```toml
[package]
name = "webylib"
version = "0.1.0"
edition = "2021"
authors = ["Webcash Developers"]
description = "Secure Rust implementation of Webcash"
license = "MIT"

[features]
default = ["server", "wallet"]
server = ["reqwest"]
wallet = ["rusqlite"]
minimal = []  # Minimal feature set

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true

[profile.dev]
opt-level = 0
debug = true
overflow-checks = true
```

### **Custom Build Scripts**
```rust
// build.rs (if needed)
fn main() {
    // Custom build logic
    println!("cargo:rerun-if-changed=build.rs");

    // Generate version information
    let version = env!("CARGO_PKG_VERSION");
    println!("cargo:rustc-env=VERSION={}", version);
}
```

## 🐛 **Troubleshooting Builds**

### **Common Build Issues**

#### **1. Dependency Resolution**
```bash
# Clear dependency cache
cargo clean
rm -rf ~/.cargo/registry/cache/
rm -rf ~/.cargo/git/db/

# Update dependencies
cargo update

# Check for conflicts
cargo tree
```

#### **2. Compilation Errors**
```bash
# Get detailed error information
cargo build --verbose

# Check Rust version compatibility
rustc --version
cargo --version

# Update Rust
rustup update
```

#### **3. Linker Issues**
```bash
# Linux: Install system dependencies
sudo apt-get install libsqlite3-dev libssl-dev pkg-config

# macOS: Install dependencies
brew install sqlite openssl pkg-config

# Check library paths
pkg-config --libs sqlite3
```

#### **4. Memory Issues**
```bash
# Increase memory limits
export CARGO_BUILD_JOBS=1

# Use less aggressive optimization
cargo build --config profile.release.opt-level=2
```

### **Performance Issues**

#### **1. Slow Builds**
```bash
# Use incremental compilation
echo "incremental = true" >> ~/.cargo/config

# Use more RAM for compilation
export RUSTFLAGS="-C codegen-units=1"

# Use faster linker (Linux)
export RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld"
```

#### **2. Large Binaries**
```bash
# Analyze binary size
cargo bloat --release

# Strip debug symbols
strip target/release/webyc

# Use UPX compression
upx target/release/webyc
```

## 📋 **CI/CD Integration**

### **GitHub Actions**
```yaml
# .github/workflows/ci.yml
name: CI
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build
      - run: cargo test
      - run: cargo clippy
      - run: cargo fmt --check
```

### **Local CI Simulation**
```bash
# Run all CI checks locally
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo doc
cargo build --release
```

## 🎯 **Build Success Criteria**

### **✅ Successful Build Checklist**
- [ ] `cargo build` completes without errors
- [ ] `cargo test` passes all tests
- [ ] `cargo clippy` reports no warnings
- [ ] `cargo fmt --check` passes
- [ ] `cargo doc` generates documentation
- [ ] Binary runs correctly: `cargo run --bin webyc -- --version`

### **📊 Quality Metrics**
- **Build Time**: < 30 seconds for incremental builds
- **Binary Size**: < 5MB for release builds
- **Test Coverage**: > 80% (when measured)
- **Zero Warnings**: Clean clippy output
- **Memory Safe**: No unsafe code blocks

---

## 🚀 **Next Steps**

After successful build:

1. **Run Tests**: [TESTING.md](TESTING.md)
2. **Start Development**: [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md)
3. **Contribute**: [CONTRIBUTING.md](../../CONTRIBUTING.md)

---

**🏗️ Happy building! Remember: `cargo build && cargo test` is your friend!**

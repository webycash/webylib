# Development Environment Setup

## 🎯 **Quick Setup for New Developers**

### **Prerequisites**
```bash
# Install Rust (using rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Verify installation
rustc --version  # Should be 1.70+
cargo --version  # Should be 1.70+
```

### **Clone and Setup**
```bash
# Clone the repository
git clone <repository-url>
cd webcash-rust

# Build the project
cargo build

# Run tests
cargo test

# Verify everything works
cargo run --bin webyc -- --version
```

---

## 🛠️ **Detailed Setup Instructions**

### **1. System Requirements**

#### **Minimum Requirements**
- **OS**: Linux, macOS, or Windows
- **CPU**: x86_64, ARM64, or RISC-V
- **Memory**: 2GB RAM minimum, 4GB recommended
- **Storage**: 500MB free space

#### **Recommended Setup**
- **OS**: Ubuntu 20.04+ or macOS 12+
- **CPU**: Modern x86_64 processor
- **Memory**: 8GB+ RAM
- **Storage**: SSD with 2GB+ free space

### **2. Rust Installation**

#### **Using rustup (Recommended)**
```bash
# Install rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add Cargo to PATH (add to ~/.bashrc or ~/.zshrc)
source ~/.cargo/env

# Verify installation
rustc --version
cargo --version
```

#### **Alternative: Package Manager**

**Ubuntu/Debian:**
```bash
# Install from Ubuntu repositories (older version)
sudo apt-get update
sudo apt-get install rustc cargo

# Or use the official Rust repository
sudo apt-get install curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**macOS:**
```bash
# Using Homebrew
brew install rust

# Or using rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Windows:**
```powershell
# Download and run rustup-init.exe from https://rustup.rs/
# Or use Chocolatey
choco install rust
```

### **3. Development Tools**

#### **Essential Tools**
```bash
# Install additional development tools
rustup component add rustfmt
rustup component add clippy
rustup component add rust-docs

# Install cargo tools
cargo install cargo-edit
cargo install cargo-watch
cargo install cargo-tarpaulin  # For code coverage
```

#### **Optional Tools**
```bash
# For documentation
cargo install mdbook

# For benchmarking
cargo install cargo-criterion

# For cross-compilation
cargo install cross
```

### **4. IDE Setup**

#### **VS Code (Recommended)**
```bash
# Install recommended extensions
code --install-extension rust-lang.rust-analyzer
code --install-extension vadimcn.vscode-lldb
code --install-extension serayuzgur.crates
code --install-extension usernamehw.errorlens
```

**VS Code Settings:**
```json
{
    "rust-analyzer.checkOnSave.command": "clippy",
    "rust-analyzer.cargo.features": "all",
    "editor.formatOnSave": true,
    "[rust]": {
        "editor.defaultFormatter": "rust-lang.rust-analyzer"
    }
}
```

#### **Other Editors**
- **IntelliJ IDEA**: Install Rust plugin
- **Vim/Neovim**: Install rust.vim and coc-rust-analyzer
- **Emacs**: Install rustic-mode

### **5. Project Setup**

#### **Initial Build**
```bash
# Clone the repository
git clone <repository-url>
cd webcash-rust

# First build (downloads dependencies)
cargo build

# Check for issues
cargo check

# Run tests
cargo test

# Format code
cargo fmt

# Lint code
cargo clippy
```

#### **Development Workflow**
```bash
# Watch for changes and rebuild
cargo watch -x build

# Run specific tests
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Generate documentation
cargo doc --open

# Check code coverage
cargo tarpaulin --out Html
```

### **6. Database Setup**

#### **SQLite Development Libraries**

**Ubuntu/Debian:**
```bash
sudo apt-get install libsqlite3-dev sqlite3
```

**macOS:**
```bash
# SQLite is included with macOS
# For development headers:
brew install sqlite
```

**Windows:**
```powershell
# Using vcpkg
vcpkg install sqlite3:x64-windows

# Or download precompiled binaries from sqlite.org
```

#### **Verify SQLite Installation**
```bash
# Check SQLite version
sqlite3 --version

# Test SQLite functionality
sqlite3 :memory: "SELECT sqlite_version();"
```

### **7. Network Setup**

#### **HTTPS Certificate Validation**
The Webcash client requires HTTPS communication with the Webcash server. Ensure your system has:

- **Updated CA certificates**
- **Working DNS resolution**
- **Internet connectivity**

#### **Test Network Connectivity**
```bash
# Test connection to Webcash server
curl -I https://webcash.org/

# Should return HTTP 200 OK
```

### **8. Testing Setup**

#### **Unit Testing**
```bash
# Run all tests
cargo test

# Run specific test module
cargo test amount::

# Run with detailed output
cargo test -- --nocapture

# Run tests in release mode
cargo test --release
```

#### **Integration Testing**
```bash
# Run integration tests (when implemented)
cargo test --test integration

# Test with different features
cargo test --features "server-mock"
```

#### **Benchmarking**
```bash
# Run benchmarks (when implemented)
cargo bench

# Profile performance
cargo build --release
# Use perf, valgrind, or Instruments
```

### **9. Documentation Setup**

#### **Generate Documentation**
```bash
# Generate Rust API documentation
cargo doc --open

# Generate code coverage reports
cargo tarpaulin --out Html

# View documentation
# Open target/doc/webylib/index.html in browser
```

#### **Contributing Documentation**
```bash
# Check documentation examples
cargo test --doc

# Generate documentation for publishing
cargo doc --release
```

### **10. CI/CD Setup**

#### **GitHub Actions**
The project includes GitHub Actions workflows for:
- **Automated testing** on multiple platforms
- **Code formatting** checks
- **Security scanning**
- **Documentation deployment**

#### **Local CI Simulation**
```bash
# Run the same checks as CI
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo doc
```

---

## 🔧 **Troubleshooting**

### **Common Issues**

#### **1. Rust Version Too Old**
```bash
# Update Rust
rustup update

# Check version
rustc --version
```

#### **2. Missing Dependencies**
```bash
# Ubuntu/Debian
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
xcode-select --install
brew install openssl pkg-config
```

#### **3. SQLite Issues**
```bash
# Check SQLite installation
pkg-config --modversion sqlite3

# Rebuild with specific SQLite
cargo clean
cargo build
```

#### **4. Network Issues**
```bash
# Test HTTPS connectivity
curl -v https://webcash.org/

# Check DNS
nslookup webcash.org
```

#### **5. Permission Issues**
```bash
# Fix permissions on wallet directory
chmod 700 ~/.webcash/

# Check file permissions
ls -la ~/.webcash/
```

### **Performance Issues**

#### **1. Slow Compilation**
```bash
# Use incremental compilation
echo "incremental = true" >> ~/.cargo/config

# Use more parallel jobs
export CARGO_BUILD_JOBS=$(nproc)
```

#### **2. Large Binary Size**
```bash
# Build with size optimizations
cargo build --release --features "minimal"

# Analyze binary size
cargo bloat --release
```

### **Debugging**

#### **1. Enable Debug Symbols**
```bash
# Build with debug symbols
cargo build

# Use debugger
rust-gdb target/debug/webyc
```

#### **2. Logging**
```bash
# Enable logging (when implemented)
export RUST_LOG=debug
cargo run --bin webyc -- info
```

#### **3. Memory Profiling**
```bash
# Use valgrind (Linux)
valgrind --tool=massif target/release/webyc --version

# Use heaptrack (Linux)
heaptrack target/release/webyc --version
```

---

## 📚 **Learning Resources**

### **Rust Fundamentals**
- [The Rust Programming Language](https://doc.rust-lang.org/book/)
- [Rust by Example](https://doc.rust-lang.org/rust-by-example/)
- [Rustlings](https://github.com/rust-lang/rustlings)

### **Webcash-Specific**
- [Webcash Website](https://webcash.org/)
- [Python Implementation](python_version/)
- [C++ Implementation](cplus_version/)

### **Cryptography**
- [Practical Cryptography for Developers](https://cryptobook.nakov.com/)
- [Cryptographic Right Answers](https://latacora.micro.blog/2018/04/03/cryptographic-right-answers/)

---

## 🎯 **Next Steps**

Once your environment is set up:

1. **Read the Architecture**: [ARCHITECTURE.md](../architecture/ARCHITECTURE.md)
2. **Build the Project**: [BUILD.md](BUILD.md)
3. **Run Tests**: [TESTING.md](TESTING.md)
4. **Start Contributing**: [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md)

---

**🚀 Ready to start developing? Run `cargo build` and `cargo test` to verify everything works!**

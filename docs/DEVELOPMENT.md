# Development

## Prerequisites

- **Rust 1.85+** (MSRV enforced in CI)
- **Git**

No other system dependencies required — SQLite is bundled, TLS is pure Rust.

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Verify
rustc --version  # Should be 1.85+
```

## Building

```bash
# Library only (default features: bundled-sqlite, cli)
cargo build

# Library without CLI
cargo build --no-default-features --features bundled-sqlite

# Library with FFI exports
cargo build --features ffi

# Release build with FFI (produces .so/.dylib/.dll + .a/.lib)
cargo build --release --features ffi

# Generate C header (requires cbindgen)
cargo install cbindgen
cbindgen --crate webylib --output include/webylib.h
```

## Testing

```bash
# Unit tests
cargo test --test unit_tests

# All tests (unit + integration, requires server access)
cargo test

# Format check
cargo fmt --check

# Lint (warnings = errors in CI)
cargo clippy --all-targets --no-default-features --features bundled-sqlite,cli -- -D warnings
```

## Cross-Compilation

### iOS

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios --lib --no-default-features --features bundled-sqlite,ffi
cargo build --release --target aarch64-apple-ios-sim --lib --no-default-features --features bundled-sqlite,ffi
```

### Android

Requires Android NDK r27+:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

# Set NDK toolchain (adjust path)
export NDK=$HOME/Android/ndk/27.0.0
export TOOLCHAIN=$NDK/toolchains/llvm/prebuilt/linux-x86_64

# aarch64
export CC_aarch64_linux_android=$TOOLCHAIN/bin/aarch64-linux-android24-clang
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$CC_aarch64_linux_android
cargo build --release --target aarch64-linux-android --lib --no-default-features --features bundled-sqlite,ffi
```

## CI Pipeline

The CI runs on every push to `main` and every tag:

| Job | Platforms | What it checks |
|-----|-----------|---------------|
| **test** | Ubuntu, macOS, Windows | fmt, clippy, build, unit tests, FFI build |
| **test-freebsd** | FreeBSD 14.2 | build + unit tests |
| **msrv** | Ubuntu, Rust 1.85 | build without optional features |
| **mobile-check** | iOS, iOS-sim | library compilation |
| **release** | 10 targets | binary/library builds + GitHub Release upload |
| **publish-crate** | Ubuntu | `cargo publish` to crates.io |

### Release targets

Desktop: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`

Mobile: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`

## Bazel

Bazel builds are supported via `MODULE.bazel` (bzlmod):

```bash
bazel build //:webylib
bazel build //:webyc
bazel test //:unit_tests
```

## Release Process

1. Update version in `Cargo.toml` and `CHANGELOG.md`
2. Commit: `git commit -m "release: v0.X.Y"`
3. Tag: `git tag -a v0.X.Y -m "v0.X.Y — description"`
4. Push: `git push origin main && git push origin v0.X.Y`
5. CI automatically builds all platforms, creates GitHub Release, publishes to crates.io

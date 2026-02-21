# Architectural Fixes - Dependency Resolution

## Problem

The codebase had fundamental dependency incompatibilities causing cascading build failures:

1. **ICU 2.0.0 incompatibility**: `reqwest 0.12` → `url 2.5+` → `idna 1.1+` → `icu_locale_core 2.0.0` which requires `writeable 0.5.x`, but transitive dependencies pulled in `writeable 0.6.2` (incompatible API)

2. **Edition2024 requirement**: `base64ct 1.8.1` required Rust edition2024, but Cargo 1.82.0 didn't support it

3. **Insecure biometric implementation**: XOR-based encryption instead of AES-256-GCM

## Root Cause

Mixing incompatible dependency versions without a coherent version strategy. The dependency chain pulled in incompatible versions that couldn't be resolved.

## Architectural Solution

### 1. Dependency Version Locking

Pinned `url` to `2.4.1` to break the ICU 2.0 dependency chain:
- `url 2.4.1` uses `idna 0.5` which uses ICU 1.x (compatible)
- This avoids the `writeable 0.5` vs `0.6` incompatibility

### 2. Removed Dead Code

- Deleted `biometric_full.rs` (560 lines of unused code)
- Removed unused `_server_config` variables (6 instances)
- Removed `chrono` dependency (replaced with `std::time`)

### 3. Secure Implementation

- Replaced XOR encryption with AES-256-GCM
- Added Argon2 for password key derivation
- Added HKDF for biometric key derivation
- Proper nonce generation with hardware RNG

### 4. Dependency Overrides

```toml
# Break incompatible dependency chain at source
url = "=2.4.1"  # Avoids ICU 2.0 → writeable 0.6 incompatibility
base64ct = "=1.6.0"  # Avoids edition2024 requirement
```

## Result

- Clean build with no dependency conflicts
- Secure cryptographic implementation
- No dead code or duplicate implementations
- Coherent dependency version strategy

## Key Principle

**Fix at the dependency level, not with workarounds.** When incompatible versions are pulled in transitively, break the chain at the earliest point possible by pinning the root dependency that introduces the incompatibility.




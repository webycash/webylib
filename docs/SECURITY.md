# Security

## Trust Model

Webcash is a **centralized** system. Security depends on:

1. **Webcash LLC server integrity** — the server is the sole transaction authority
2. **Client-side cryptography** — secrets never leave the device
3. **TLS transport security** — all communication over HTTPS (rustls, no OpenSSL)
4. **User operational security** — master secret backup, password strength

## Cryptographic Primitives

| Primitive | Usage | Library |
|-----------|-------|---------|
| SHA-256 | Secret hashing, HD derivation, proof-of-work | `sha2` |
| HMAC-SHA-512 | Key derivation (reserved for future BIP32) | `hmac` |
| AES-256-GCM | Seed/database encryption | `aes-gcm` |
| Argon2id | Password-based key derivation | `argon2` |
| HKDF-SHA-256 | Biometric key derivation | `hkdf` |
| CSPRNG | Master secret generation, nonces, salts | `getrandom` (OS-level) |

## Memory Safety

- All secret material types implement `Zeroize + ZeroizeOnDrop`
- `CryptoSecret` — 32-byte key, zeroed on drop, redacted in Debug/Display
- `SecureString` — sensitive strings, zeroed on drop, redacted in Debug/Display
- `HDWallet` — derives `Zeroize + ZeroizeOnDrop` for master secret
- No secret material is ever logged, printed, or serialized without explicit intent

## Database Security

- SQLite WAL mode prevents corruption on crash
- Transactions wrap all money-moving operations (insert, pay, merge)
- Optional database encryption with Argon2 + AES-256-GCM
- Optional biometric encryption (iOS Keychain / Android Keystore hooks)

## Network Security

- Pure Rust TLS via `rustls` — no OpenSSL dependency
- No native-tls fallback — cross-compilation safe
- Connection pooling with idle timeout
- Server errors never expose secret material

## FFI Safety

- All FFI functions validate pointers before dereferencing
- NULL pointers return error codes, never crash
- Thread-local error messages avoid data races
- Opaque handles prevent direct memory access from foreign code
- Every allocation has a matching free function

## Responsible Disclosure

Report security vulnerabilities to security@weby.cash.

# Architecture

## System Overview

Webcash is a **centralized electronic cash system** operated by Webcash LLC. The `webylib` library is a client-side wallet that communicates with the Webcash server to perform transactions. All transaction validation and double-spend prevention happens server-side.

```
┌──────────────────────────────────┐
│         Your Application         │
│  (Rust, Python, C#, Swift, Go)   │
├──────────────────────────────────┤
│            webylib               │
│  ┌────────┐  ┌───────────────┐   │
│  │ Wallet │  │  Server Client │  │
│  │ SQLite │  │  (reqwest/TLS) │  │
│  └────────┘  └───────────────┘   │
├──────────────────────────────────┤
│         FFI C ABI Layer          │
│   (cdylib / staticlib exports)   │
└──────────┬───────────────────────┘
           │ HTTPS (rustls)
           ▼
┌──────────────────────────────────┐
│      webcash.org (Webcash LLC)   │
│  - Transaction validation        │
│  - Double-spend prevention       │
│  - Mining difficulty control     │
│  - Monetary supply enforcement   │
└──────────────────────────────────┘
```

## Trust Model

- The Webcash server is the sole authority for transaction validity
- Client-side secrets are never sent to the server — only SHA256 hashes
- The server cannot see your balance or link your transactions
- Loss of the server means loss of the ability to transact (centralized risk)

## Wallet Architecture

### Storage

The wallet uses SQLite with WAL mode for crash-safe local storage:

| Table | Purpose |
|-------|---------|
| `unspent_outputs` | Secret webcash with amounts, spend status |
| `spent_hashes` | SHA256 hashes of spent secrets (audit trail) |
| `wallet_metadata` | Master secret, configuration key-value pairs |
| `walletdepths` | HD derivation depth per chain code |

### HD Key Derivation

Four chain codes partition the key space:

| Chain | Code | Purpose |
|-------|------|---------|
| RECEIVE | 0 | Incoming payments — ownership transfer |
| PAY | 1 | Outgoing payment outputs |
| CHANGE | 2 | Transaction change returned to wallet |
| MINING | 3 | Mining reward collection |

Derivation algorithm (matches Python reference exactly):

```
tag    = SHA256("webcashwalletv1")
secret = SHA256(tag || tag || master_secret || chain_code_be64 || depth_be64)
```

### Transaction Flow

**Insert (receive funds):**
1. Parse incoming `e{amount}:secret:{hex}` string
2. Derive new RECEIVE secret at current depth
3. Submit `replace` request: old secret → new secret
4. Server atomically invalidates old, creates new
5. Store new secret in wallet, increment depth

**Pay (send funds):**
1. Select unspent inputs covering the amount
2. Derive PAY secret for payment output
3. Derive CHANGE secret for remainder (if any)
4. Submit `replace` request: inputs → [payment, change]
5. Mark inputs spent, store change output

**Merge (consolidate):**
1. Select multiple small outputs
2. Generate single consolidated output
3. Submit `replace`: many inputs → one output
4. Reduces wallet fragmentation

### Server Client

The wallet owns a `Box<dyn ServerClientTrait>` — no global singletons. Each wallet instance creates its own HTTP client with connection pooling. The trait enables mock injection for testing.

Four server endpoints:
- `POST /api/v1/health_check` — Query spend status of outputs
- `POST /api/v1/replace` — Atomic webcash replacement (core transaction)
- `GET /api/v1/target` — Current mining difficulty and parameters
- `POST /api/v1/mining_report` — Submit proof-of-work solution

### Encryption

Two encryption layers available:

1. **Seed encryption** — Argon2 key derivation + AES-256-GCM wrapping of the 32-byte master secret
2. **Database encryption** — Full SQLite file encryption with password or biometric keys

All secret material implements `Zeroize + ZeroizeOnDrop`.

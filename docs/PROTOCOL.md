# Webcash Protocol

## Overview

Webcash is a centralized electronic cash system. Value exists as cryptographic strings that can be spent exactly once. A central server (webcash.org) validates all transactions and prevents double-spending.

## Token Format

```
e{amount}:{type}:{value}
```

| Type | Example | Description |
|------|---------|-------------|
| `secret` | `e1.00000000:secret:abcdef...` | Private — the owner knows the secret |
| `public` | `e1.00000000:public:sha256...` | Public — SHA256 hash of the secret |

- Amounts use up to 8 decimal places (1 webcash = 100,000,000 wats)
- Secrets are 32 bytes (64 hex characters)
- Public hashes are `SHA256(secret_ascii_bytes)` — hash the ASCII hex string, not the decoded bytes

## Server API

**Base URL:** `https://webcash.org/api/v1/`

### POST `/health_check`

Query spend status of outputs.

**Request:** Array of public webcash strings
```json
["e1.00000000:public:abc123...", "e2.00000000:public:def456..."]
```

**Response:**
```json
{
  "status": "success",
  "results": {
    "e1.00000000:public:abc123...": { "spent": false, "amount": "1.00000000" },
    "e2.00000000:public:def456...": { "spent": true, "amount": null }
  }
}
```

### POST `/replace`

Atomic webcash replacement — the core transaction operation. Spends inputs and creates outputs in one atomic step. Total input amount must equal total output amount.

**Request:**
```json
{
  "webcashes": ["e3.00000000:secret:input1...", "e2.00000000:secret:input2..."],
  "new_webcashes": ["e4.00000000:secret:output1...", "e1.00000000:secret:change..."],
  "legalese": { "terms": true }
}
```

**Response:**
```json
{ "status": "success" }
```

### GET `/target`

Current mining difficulty and reward parameters.

**Response:**
```json
{
  "difficulty_target_bits": 28,
  "epoch": 12345,
  "mining_amount": "50.00000000",
  "mining_subsidy_amount": "0.00000000",
  "ratio": 0.5
}
```

### POST `/mining_report`

Submit proof-of-work solution.

**Request:**
```json
{
  "preimage": "base64-encoded-json-with-nonce",
  "legalese": { "terms": true }
}
```

## HD Key Derivation

Deterministic wallet derivation matching the Python reference implementation:

```
tag    = SHA256("webcashwalletv1")
secret = SHA256(tag || tag || master_secret || chain_code_be64 || depth_be64)
```

**Chain codes:**

| Name | Value | Purpose |
|------|-------|---------|
| RECEIVE | 0 | Incoming payments |
| PAY | 1 | Outgoing payment outputs |
| CHANGE | 2 | Transaction change |
| MINING | 3 | Mining rewards |

The master secret is a random 32-byte value. From it, an unlimited number of deterministic secrets can be derived. Recovery scans each chain code up to a configurable gap limit to find used secrets on the server.

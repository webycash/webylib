# Webcash

## What is Webcash?

**Webcash is a centralized electronic cash system** operated by Webcash LLC. Unlike decentralized cryptocurrencies, Webcash relies on a central server to validate transactions and prevent double-spending.

Money exists as cryptographically signed strings that can be spent exactly once. Each webcash token is a claim of value that must be validated by the central server.

**Format:** `e{amount}:{type}:{value}`

**Example:** `e100:secret:abc123def456...`

## How It Works

1. **Creation**: New webcash is created through CPU mining or received from others
2. **Mining Fees**: Miners pay a fee to the central authority when reporting successful mining
3. **Spending**: You replace one webcash with one or multiple new ones using the server
4. **Verification**: Server validates all transactions and prevents double-spends
5. **No Accounts**: Wallets track collections of replaced secrets, not account balances

## Mining and Economic Model

### CPU Mining Process

Webcash uses CPU mining where anyone can mine new webcash using their computer's processing power:

1. **Get Mining Target**: Request current difficulty from server
2. **CPU Mining**: Perform proof-of-work calculations
3. **Report Success**: Submit mining result to server
4. **Receive Reward**: Server creates new webcash for you

### Mining Fees and Central Authority

**How the Central Authority is Funded:**

- **Mining Fee**: When you report successful mining, the server charges a small fee
- **Central Bank Model**: Like a central bank, Webcash LLC collects fees to fund operations
- **Shared Revenue**: Unlike traditional central banks, miners create the money supply and share fees

**Economic Flow:**
```
Miner CPU Work → Mining Report → Server Fee Deduction → New Webcash Created
                                      ↓
                               Webcash LLC Operations Fund
```

### Mining Example

```bash
# 1. Get current mining target
curl https://webcash.org/api/v1/target

# 2. Mine with CPU (find nonce that meets difficulty)
# 3. Report successful mining
curl -X POST https://webcash.org/api/v1/mining_report \
  -H "Content-Type: application/json" \
  -d '{
    "mining_report": "proof_of_work_data...",
    "webcash": "e1000000000000:secret:mining_reward_secret..."
  }'
```

**Result:** Server creates new webcash minus the mining fee, which funds Webcash LLC operations.

### Key Differences from Traditional Mining

- **Centralized Control**: Server validates all mining, not decentralized consensus
- **Fee-Based Funding**: Central authority funded through mining fees, not seigniorage
- **No Mining Pools**: Individual mining only (pools not needed due to centralized validation)
- **Predictable Rewards**: Server controls mining difficulty and rewards

## Amount System

- 1 webcash = 100,000,000 wats
- 8 decimal precision
- Example: `e100000000:secret:...` = 1.00000000 webcash

## Wallets vs Traditional Banking

**Traditional Banking:**
- Accounts with balances
- Server tracks account ownership
- Transactions move money between accounts

**Webcash Wallets:**
- Collections of secret strings
- No accounts or balances
- Transactions replace spended secrets with unspended ones
- Deterministic key generation for address management

## Server API

### Endpoints

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/api/v1/health_check` | Check if webcash is valid/unspent |
| POST | `/api/v1/replace` | Spend webcash (make payment) |
| POST | `/api/v1/burn` | Permanently destroy webcash |
| GET | `/api/v1/target` | Get mining difficulty target |
| POST | `/api/v1/mining_report` | Submit proof-of-work |

### Health Check

**Request:** Array of public webcash strings
```json
["e100:public:abc123...", "e50:public:def456..."]
```

**Response:**
```json
{
  "status": "success",
  "results": {
    "e100:public:abc123...": {"spent": false, "amount": "100"},
    "e50:public:def456...": {"spent": true}
  }
}
```

### Replace (Payment)

**Request:** Secret webcash to spend + new webcash to create
```json
{
  "webcashes": ["e100:secret:abc123..."],
  "new_webcashes": ["e50:secret:def456...", "e50:secret:ghi789..."],
  "legalese": {"terms": true}
}
```

**Response:**
```json
{"status": "success"}
```

### Target (Mining)

**Response:**
```json
{
  "difficulty_target_bits": 28,
  "epoch": 0,
  "mining_amount": "20000000000000",
  "mining_subsidy_amount": "1000000000000",
  "ratio": 1.0
}
```

## The Replacement Mechanism

Webcash payments work by **replacing** old secrets with new ones:

### Example Payment Flow

You have: `e10:secret:abc123...` (10 webcash)

**Step 1:** Generate new secrets for payment and change
- Payment output: `e5:secret:def456...` (5 webcash for recipient)
- Change output: `e5:secret:ghi789...` (5 webcash back to you)

**Step 2:** Send replacement request to server
```json
{
  "webcashes": ["e10:secret:abc123..."],
  "new_webcashes": ["e5:secret:def456...", "e5:secret:ghi789..."],
  "legalese": {"terms": true}
}
```

**Step 3:** Server validates and replaces
- Old secret `abc123...` is marked as spent
- New secrets `def456...` and `ghi789...` become spendable

**Step 4:** Give payment secret to recipient
- You keep `e5:secret:ghi789...` (change)
- Recipient gets `e5:secret:def456...` (payment)

### Key Points

- **Conservation**: Input amount = output amount (no fees)
- **Server Control**: Only server can authorize replacements
- **Deterministic**: Wallets use HD keys to generate new secrets
- **No Accounts**: Just collections of secrets managed by deterministic wallets

## Implementation Comparison

| Implementation | Creator | Purpose | Platform | Language | Status |
|----------------|---------|---------|----------|----------|--------|
| **Python Wallet** | Webcash LLC | Complete wallet | Desktop/Web | Python | Reference |
| **C++ Webminer** | Community | High-performance mining | Server | C++ | Active |
| **Rust webylib** | Community | Cross-platform library | iOS/macOS/Android/Linux/BSD/Embedded | Rust | Active |
| **Webcasa** | Community | Simple web wallet | Web | JavaScript | Active |

### Python Wallet (Blueprint)
- **By:** Original Webcash LLC team
- **Purpose:** Complete wallet functionality
- **Features:** Full UI, wallet management, payment processing
- **Use case:** Reference implementation, daily use

### C++ Webminer
- **By:** Community developers
- **Purpose:** High-performance mining operations
- **Features:** CPU mining, mining pools, optimized for speed
- **Use case:** Mining webcash, pool operations

### Rust webylib
- **By:** Community developers
- **Purpose:** Platform-portable library
- **Features:** Memory-safe, C bindings, cross-platform
- **Platforms:** iOS, macOS, Android, Linux, BSD, embedded systems
- **Use case:** Building wallets, integrating webcash into apps

### Webcasa (Web Wallet)
- **By:** Community developers
- **Purpose:** Simple web-based wallet
- **Features:** Browser-based, easy to use, no installation
- **Use case:** Quick access, online payments

## CLI Usage Examples

### Check Webcash Status
```bash
# Check if webcash is valid and unspent
curl -X POST https://webcash.org/api/v1/health_check \
  -H "Content-Type: application/json" \
  -d '["e100:public:abc123..."]'
```

### Make Payment
```bash
# Spend webcash and create new ones
curl -X POST https://webcash.org/api/v1/replace \
  -H "Content-Type: application/json" \
  -d '{
    "webcashes": ["e100:secret:abc123..."],
    "new_webcashes": ["e50:secret:def456...", "e50:secret:ghi789..."],
    "legalese": {"terms": true}
  }'
```

### Get Mining Target
```bash
# Get current mining difficulty
curl https://webcash.org/api/v1/target
```

## Protocol Rules

1. **One-time use**: Each webcash can only be spent once
2. **Amount conservation**: Input amount = output amount (no fees)
3. **Server validation**: All transactions verified by server
4. **Public verification**: Anyone can check transaction validity
5. **Mining rewards**: New webcash created through proof-of-work

## Getting Started

1. **Get webcash**: Mine it or receive from someone
2. **Store securely**: Keep secret strings private
3. **Verify before use**: Check status with health_check
4. **Make payments**: Use replace endpoint
5. **Track spending**: Server prevents double-spends

## Key Concepts

- **Secret webcash**: Spendable (keep private)
- **Public webcash**: Verifiable (share publicly)
- **Mining**: Creating new webcash through computation
- **Burning**: Permanently destroying webcash
- **Replacement**: Spending old webcash to create new ones

This covers the essential mechanics, API, and protocol for working with webcash across all implementations.

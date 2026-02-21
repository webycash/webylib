# Webcash Server API Reference

## Overview

The Webcash server API is a RESTful HTTP interface for managing electronic cash transactions. All endpoints use JSON for request/response bodies.

**Base URL:** `https://webcash.org/api/v1/`

## Authentication

No authentication required. All operations are public.

## Endpoints

### Health Check

Check if webcash tokens are valid and unspent.

**Endpoint:** `POST /api/v1/health_check`

**Request Body:**
```json
[
  "e100:public:abc123...",
  "e50:public:def456..."
]
```

**Response (Success):**
```json
{
  "status": "success",
  "results": {
    "e100:public:abc123...": {
      "spent": false,
      "amount": "100"
    },
    "e50:public:def456...": {
      "spent": true
    }
  }
}
```

**Response (Error):**
```json
{
  "status": "error",
  "message": "Invalid webcash format"
}
```

**Status Codes:**
- `200` - Success
- `400` - Invalid request format
- `500` - Server error

### Replace (Payment)

Spend webcash tokens and create new ones. This is the core payment mechanism.

**Endpoint:** `POST /api/v1/replace`

**Request Body:**
```json
{
  "webcashes": [
    "e100:secret:abc123..."
  ],
  "new_webcashes": [
    "e50:secret:def456...",
    "e50:secret:ghi789..."
  ],
  "legalese": {
    "terms": true
  }
}
```

**Parameters:**
- `webcashes` (array): Secret webcash tokens to spend
- `new_webcashes` (array): New secret webcash tokens to create
- `legalese.terms` (boolean): Must be `true` to accept terms

**Constraints:**
- Sum of input amounts must equal sum of output amounts (no fees)
- All input webcash must be valid and unspent
- Server validates ownership of input webcash

**Response (Success):**
```json
{
  "status": "success"
}
```

**Response (Error):**
```json
{
  "status": "error",
  "message": "Webcash already spent"
}
```

### Burn

Permanently destroy webcash tokens.

**Endpoint:** `POST /api/v1/burn`

**Request Body:**
```json
{
  "webcashes": [
    "e100:secret:abc123..."
  ],
  "legalese": {
    "terms": true
  }
}
```

**Response (Success):**
```json
{
  "status": "success"
}
```

### Target (Mining)

Get current mining difficulty target.

**Endpoint:** `GET /api/v1/target`

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

**Parameters:**
- `difficulty_target_bits`: Mining difficulty
- `epoch`: Current mining epoch
- `mining_amount`: Total mining reward per block
- `mining_subsidy_amount`: Base mining subsidy
- `ratio`: Mining ratio multiplier

### Mining Report

Submit proof-of-work for mining rewards.

**Endpoint:** `POST /api/v1/mining_report`

**Request Body:**
```json
{
  "mining_report": "proof_of_work_data...",
  "webcash": "e1000000000000:secret:mining_reward_secret..."
}
```

## Error Handling

All endpoints return consistent error responses:

```json
{
  "status": "error",
  "message": "Human-readable error description"
}
```

**Common Errors:**
- `Invalid webcash format`
- `Webcash already spent`
- `Insufficient funds`
- `Server validation failed`
- `Terms not accepted`

## Rate Limiting

- Health checks: 100 requests/minute
- Replace operations: 10 requests/minute
- Mining operations: Unlimited

## Data Types

### Webcash String Format

All webcash uses the format: `e{amount}:{type}:{value}`

**Secret Webcash:** `e100:secret:32_character_hex_string`
- Contains the actual spendable secret
- Keep private for security

**Public Webcash:** `e100:public:64_character_hex_hash`
- Contains SHA256 hash of secret
- Safe to share publicly

### Amount Format

Amounts are in wats (1 webcash = 100,000,000 wats):
- `100000000` = 1.00000000 webcash
- `50000000` = 0.50000000 webcash
- `12345678` = 0.12345678 webcash

## Protocol Rules

1. **Conservation of Value**: Input amount = output amount
2. **One-Time Use**: Each webcash can only be spent once
3. **Server Validation**: All transactions verified by central server
4. **Public Verification**: Anyone can verify transaction validity
5. **No Fees**: Webcash transactions have zero fees

## Examples

### Check Balance
```bash
curl -X POST https://webcash.org/api/v1/health_check \
  -H "Content-Type: application/json" \
  -d '["e100:public:abc123..."]'
```

### Send Payment
```bash
curl -X POST https://webcash.org/api/v1/replace \
  -H "Content-Type: application/json" \
  -d '{
    "webcashes": ["e100:secret:abc123..."],
    "new_webcashes": ["e50:secret:def456...", "e50:secret:ghi789..."],
    "legalese": {"terms": true}
  }'
```

### Get Mining Info
```bash
curl https://webcash.org/api/v1/target
```

## SDK Integration

Most implementations provide high-level SDKs that handle:
- Request formatting
- Response parsing
- Error handling
- Connection management

See individual implementation documentation for SDK usage.

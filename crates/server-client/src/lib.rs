//! Minimal asset-agnostic HTTP client for the webycash-server family.
//!
//! Every flavor (Webcash, RGB Fungible, RGB Collectible, Voucher) speaks
//! the same wire protocol — `/api/v1/replace`, `/health_check`, `/burn`,
//! `/mining_report`, `/issue` — only the token wire format differs. This
//! crate ships ONE Client; the wallet crates (`wallet-webcash`,
//! `wallet-rgb`, `wallet-voucher`) wrap it in flavor-specific verbs
//! (`pay`, `transfer`, `insert`).
//!
//! Layers:
//! - [`wire`] — the sans-IO protocol: builds every request (method, path,
//!   headers, body) and strictly parses every response into typed results or
//!   a typed [`ClientError`]. Cross-target (`wasm32` included); async/effect
//!   runtimes (extro-node's `NetEffect::Https`, browser `fetch`) execute the
//!   [`wire::Request`] themselves and hand the status + body back to
//!   [`wire`]'s parsers.
//! - [`Client`] — the native blocking transport (ureq + rustls) over the same
//!   [`wire`] layer. Not built on `wasm32` (no blocking sockets there).
//!
//! Every POST is a CORS-simple request: `Content-Type:
//! text/plain;charset=UTF-8`, JSON-string body, no custom headers (the one
//! exception is the operator-only `/issue`, which carries
//! `X-Issuer-Signature`). webcash.org's CORS preflight allows only
//! `x-requested-with`, so a JSON content-type makes every browser call die as
//! "Failed to fetch"; webycash-server and webcash.org parse the body bytes
//! regardless of content-type.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────────────
// HTLC types — mirrored exactly from `webycash-asset-rgb::htlc` so the wallet
// can construct request bodies the RGB server will accept without depending
// on the server crate. Keep in lockstep — wire format is the contract.
// ─────────────────────────────────────────────────────────────────────────────

/// Wallet-side counterpart to `webycash-asset-rgb::htlc::LockRequest`.
/// Carries a *delta* from server-now (never an absolute timestamp) — see
/// `docs/referee-zkp-based-swap.md` §8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtlcLockRequest {
    /// Hex of `sha256(X)` for the agreed preimage.
    pub committed_h_hex: String,
    /// Seconds from server-now until refund unlocks. Server stamps
    /// `refund_after_unix = server_now + this`.
    pub refund_after_seconds_from_now: u64,
    /// Recipient secret hex (the future claim-path owner).
    pub claim_owner_secret_hex: String,
    /// Sender refund secret hex (the refund-path owner).
    pub refund_owner_secret_hex: String,
}

/// Wallet-side counterpart to `webycash-asset-rgb::htlc::HtlcWitness`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtlcWitness {
    /// Hex preimage when taking the claim path; `None` for refund.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provided_x_hex: Option<String>,
    /// Hex of `sha256(output_secret_hex_ascii)`. The server cross-checks
    /// this against the actual output's hash.
    pub output_owner_hash_hex: String,
}

impl HtlcWitness {
    /// Convenience: build the claim-path witness given the preimage and
    /// the output secret. Computes the output owner hash.
    pub fn claim(provided_x_hex: impl Into<String>, output_secret_hex: &str) -> Self {
        Self {
            provided_x_hex: Some(provided_x_hex.into()),
            output_owner_hash_hex: hex::encode(Sha256::digest(output_secret_hex.as_bytes())),
        }
    }

    /// Convenience: build the refund-path witness.
    pub fn refund(output_secret_hex: &str) -> Self {
        Self {
            provided_x_hex: None,
            output_owner_hash_hex: hex::encode(Sha256::digest(output_secret_hex.as_bytes())),
        }
    }
}

/// One entry in the `htlc_locks` array — pair an output index with the
/// lock parameters the server should stamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtlcLockEntry {
    /// Index into `new_webcashes` of the output to lock.
    pub output_index: usize,
    /// The lock parameters the server stamps.
    pub request: HtlcLockRequest,
}

/// One entry in the `htlc_witnesses` array — pair an input index with
/// its claim-or-refund witness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtlcWitnessEntry {
    /// Index into `webcashes` of the locked input.
    pub input_index: usize,
    /// The claim-or-refund witness.
    pub witness: HtlcWitness,
}

/// Failure modes when talking to a webycash-server flavor.
///
/// - `Http`: server returned a non-2xx status. Body is the raw
///   response (typically the Tornado-style HTML 500 envelope or a
///   JSON error from the asset-gated handler).
/// - `Transport`: TCP connect / read / write failed before a status
///   line came back.
/// - `Encode`: the request could not be built (bad input or
///   serialisation failure).
/// - `Decode`: a 2xx response whose body is not the documented JSON
///   shape. Fail-closed: callers never get a guessed answer.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// Server returned a non-2xx status. Body is the raw response.
    #[error("HTTP error: {status}: {body}")]
    Http {
        /// HTTP status code returned by the server.
        status: u16,
        /// Raw response body (may be JSON or Tornado-style HTML 500).
        body: String,
    },
    /// TCP connect / read / write failed before a status came back.
    #[error("transport error: {0}")]
    Transport(String),
    /// The request body could not be built.
    #[error("body encode error: {0}")]
    Encode(String),
    /// A 2xx response did not match the documented response shape.
    #[error("response decode error: {0}")]
    Decode(String),
}

/// Convenience alias used across the wallet crates for results from
/// any `Client` method.
pub type ClientResult<T> = Result<T, ClientError>;

/// Server-known state of one public token, from `/api/v1/health_check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenStatus {
    /// The server has no record of the token (`spent: null` or no row).
    Unknown,
    /// Known and unspent; `amount` as the server reports it.
    Unspent {
        /// Decimal amount string, when the server reports one.
        amount: Option<String>,
    },
    /// Known and spent.
    Spent {
        /// Decimal amount string, when the server reports one.
        amount: Option<String>,
    },
}

impl TokenStatus {
    /// True only for [`TokenStatus::Unspent`].
    pub fn is_unspent(&self) -> bool {
        matches!(self, TokenStatus::Unspent { .. })
    }
}

/// Sans-IO wire protocol for the webycash-server family. Builds requests and
/// parses responses; never performs I/O, so it is the single protocol source
/// for every transport (native [`Client`](crate::Client), extro-node's effect
/// interpreter, browser `fetch`).
pub mod wire {
    use super::{ClientError, ClientResult, TokenStatus};

    /// The one content-type every webycash POST carries: CORS-safelisted, so
    /// browser `fetch` never preflights.
    pub const POST_CONTENT_TYPE: &str = "text/plain;charset=UTF-8";

    /// HTTP method of a [`Request`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Method {
        /// `GET` (no body).
        Get,
        /// `POST` (JSON string body).
        Post,
    }

    impl Method {
        /// Upper-case method token.
        pub fn as_str(self) -> &'static str {
            match self {
                Method::Get => "GET",
                Method::Post => "POST",
            }
        }
    }

    /// One fully-specified request; the transport only prefixes the base URL.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Request {
        /// HTTP method.
        pub method: Method,
        /// Absolute path, e.g. `/api/v1/health_check`.
        pub path: &'static str,
        /// Exact header list to send (nothing else may be added).
        pub headers: Vec<(&'static str, String)>,
        /// Body bytes (empty for `GET`).
        pub body: Vec<u8>,
    }

    impl Request {
        /// `base` (trailing slashes trimmed) + `path`.
        pub fn url(&self, base: &str) -> String {
            format!("{}{}", base.trim_end_matches('/'), self.path)
        }
    }

    fn post(path: &'static str, body: &serde_json::Value) -> ClientResult<Request> {
        let body = serde_json::to_vec(body).map_err(|e| ClientError::Encode(e.to_string()))?;
        Ok(Request {
            method: Method::Post,
            path,
            headers: vec![("Content-Type", POST_CONTENT_TYPE.to_string())],
            body,
        })
    }

    fn get(path: &'static str) -> Request {
        Request {
            method: Method::Get,
            path,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// `POST /api/v1/replace` — N inputs → M outputs.
    pub fn replace(inputs: &[String], outputs: &[String]) -> ClientResult<Request> {
        post(
            "/api/v1/replace",
            &serde_json::json!({
                "webcashes": inputs,
                "new_webcashes": outputs,
                "legalese": {"terms": true},
            }),
        )
    }

    /// `POST /api/v1/replace` with the RGB HTLC extensions.
    pub fn replace_with_htlc(
        inputs: &[String],
        outputs: &[String],
        htlc_locks: &[super::HtlcLockEntry],
        htlc_witnesses: &[super::HtlcWitnessEntry],
    ) -> ClientResult<Request> {
        post(
            "/api/v1/replace",
            &serde_json::json!({
                "webcashes": inputs,
                "new_webcashes": outputs,
                "legalese": {"terms": true},
                "htlc_locks": htlc_locks,
                "htlc_witnesses": htlc_witnesses,
            }),
        )
    }

    /// `POST /api/v1/burn` — mark one secret spent permanently.
    pub fn burn(secret_token: &str) -> ClientResult<Request> {
        post(
            "/api/v1/burn",
            &serde_json::json!({"webcash": secret_token, "legalese": {"terms": true}}),
        )
    }

    /// `POST /api/v1/health_check` — bare JSON array of public tokens. Refuses
    /// anything that is not a public token, so a secret can never leave the
    /// wallet through a lookup.
    pub fn health_check(public_tokens: &[String]) -> ClientResult<Request> {
        if public_tokens.is_empty() {
            return Err(ClientError::Encode("health_check: no tokens".into()));
        }
        if let Some(bad) = public_tokens.iter().find(|t| public_hash(t).is_none()) {
            let shown: String = bad.chars().take(12).collect();
            return Err(ClientError::Encode(format!(
                "health_check: not a public token ({shown}…)"
            )));
        }
        post("/api/v1/health_check", &serde_json::json!(public_tokens))
    }

    /// `POST /api/v1/mining_report` — submit a PoW preimage.
    pub fn mining_report(preimage: &str) -> ClientResult<Request> {
        post(
            "/api/v1/mining_report",
            &serde_json::json!({"preimage": preimage, "legalese": {"terms": true}}),
        )
    }

    /// `POST /api/v1/issue` — operator-signed mint over the exact `body`
    /// bytes; the detached signature rides `X-Issuer-Signature`.
    pub fn issue(body: Vec<u8>, sig_hex: &str) -> ClientResult<Request> {
        std::str::from_utf8(&body).map_err(|e| ClientError::Encode(e.to_string()))?;
        let mut headers = vec![("Content-Type", POST_CONTENT_TYPE.to_string())];
        if !sig_hex.is_empty() {
            headers.push(("X-Issuer-Signature", sig_hex.to_string()));
        }
        Ok(Request {
            method: Method::Post,
            path: "/api/v1/issue",
            headers,
            body,
        })
    }

    /// `GET /api/v1/target`.
    pub fn target() -> Request {
        get("/api/v1/target")
    }

    /// `GET /api/v1/stats`.
    pub fn stats() -> Request {
        get("/api/v1/stats")
    }

    /// `200..300` ⇒ the body as UTF-8; otherwise [`ClientError::Http`].
    pub fn check(status: u16, body: &[u8]) -> ClientResult<String> {
        let text = String::from_utf8_lossy(body).into_owned();
        if (200..300).contains(&status) {
            Ok(text)
        } else {
            Err(ClientError::Http { status, body: text })
        }
    }

    /// Lookup key of a public token: lower-cased `H`, plus the
    /// `:{contract}:{issuer}` namespace for voucher/RGB tokens. Accepted shapes:
    /// `e{amount}:public:{H}[:{ns}…]` (webcash, voucher, RGB20) and the
    /// amount-less `public:{H}:{ns}…` (RGB21 collectibles). The amount tag is
    /// excluded because the server normalises it (`e1.0` → `e1`). `None` if
    /// `token` is not a public token.
    pub fn public_hash(token: &str) -> Option<String> {
        let rest = match token.strip_prefix("public:") {
            Some(rest) => rest,
            None => {
                let (amount, rest) = token.split_once(":public:")?;
                let amount = amount.strip_prefix('e')?;
                let amount_ok = !amount.is_empty()
                    && amount.chars().all(|c| c.is_ascii_digit() || c == '.')
                    && amount.chars().filter(|c| *c == '.').count() <= 1;
                if !amount_ok {
                    return None;
                }
                rest
            }
        };
        let (hash, ns) = match rest.split_once(':') {
            Some((h, ns)) => (h, Some(ns)),
            None => (rest, None),
        };
        let hash_ok = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit());
        let ns_ok = ns.is_none_or(|ns| !ns.is_empty() && ns.split(':').all(|seg| !seg.is_empty()));
        // Amount-less tokens only exist namespaced (RGB21).
        let shape_ok = ns.is_some() || !token.starts_with("public:");
        (hash_ok && ns_ok && shape_ok).then(|| match ns {
            Some(ns) => format!("{}:{ns}", hash.to_ascii_lowercase()),
            None => hash.to_ascii_lowercase(),
        })
    }

    /// Strictly parse a `/health_check` response into one [`TokenStatus`] per
    /// requested token (same order). Non-2xx ⇒ `Http`; a 2xx body without a
    /// `results` object, or a row with a non-boolean `spent` / non-scalar
    /// `amount` ⇒ `Decode`. Rows are matched by `H`, because the server
    /// normalises the amount tag (`e1.0` → `e1`) in the results key.
    pub fn parse_health_check(
        status: u16,
        body: &[u8],
        requested: &[String],
    ) -> ClientResult<Vec<TokenStatus>> {
        let text = check(status, body)?;
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ClientError::Decode(format!("health_check: not JSON: {e}")))?;
        let results = v
            .get("results")
            .and_then(|r| r.as_object())
            .ok_or_else(|| ClientError::Decode("health_check: missing results object".into()))?;
        let mut by_hash = std::collections::HashMap::new();
        for (k, row) in results {
            let h = public_hash(k)
                .ok_or_else(|| ClientError::Decode(format!("health_check: bad results key {k}")))?;
            by_hash.insert(h, row);
        }
        requested
            .iter()
            .map(|t| {
                let h = public_hash(t).ok_or_else(|| {
                    ClientError::Encode("health_check: not a public token".into())
                })?;
                match by_hash.get(&h) {
                    None => Ok(TokenStatus::Unknown),
                    Some(row) => parse_row(row),
                }
            })
            .collect()
    }

    fn parse_row(row: &serde_json::Value) -> ClientResult<TokenStatus> {
        let obj = row
            .as_object()
            .ok_or_else(|| ClientError::Decode("health_check: row is not an object".into()))?;
        let amount = match obj.get("amount") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            Some(_) => {
                return Err(ClientError::Decode(
                    "health_check: amount is not string|number|null".into(),
                ))
            }
        };
        match obj.get("spent") {
            None | Some(serde_json::Value::Null) => Ok(TokenStatus::Unknown),
            Some(serde_json::Value::Bool(true)) => Ok(TokenStatus::Spent { amount }),
            Some(serde_json::Value::Bool(false)) => Ok(TokenStatus::Unspent { amount }),
            Some(_) => Err(ClientError::Decode(
                "health_check: spent is not boolean|null".into(),
            )),
        }
    }

    /// Mining parameters from `GET /api/v1/target`.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Target {
        /// Required leading zero bits of `sha256(preimage)`.
        pub difficulty_target_bits: u32,
        /// Difficulty epoch.
        pub epoch: u32,
        /// Miner reward per solution, decimal string (`"195.3125"`).
        pub mining_amount: String,
        /// Subsidy per solution, decimal string.
        pub mining_subsidy_amount: String,
        /// `mining_amount / (mining_amount + subsidy)` as the server reports it.
        pub ratio: f64,
    }

    /// Server acknowledgement of an accepted `POST /api/v1/mining_report`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MiningReportAck {
        /// Difficulty the server will require next, when it reports one.
        pub difficulty_target: Option<u32>,
    }

    fn json_object(
        what: &str,
        status: u16,
        body: &[u8],
    ) -> ClientResult<serde_json::Map<String, serde_json::Value>> {
        let text = check(status, body)?;
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(serde_json::Value::Object(map)) => Ok(map),
            Ok(_) => Err(ClientError::Decode(format!(
                "{what}: body is not a JSON object"
            ))),
            Err(e) => Err(ClientError::Decode(format!("{what}: not JSON: {e}"))),
        }
    }

    fn success_object(
        what: &str,
        status: u16,
        body: &[u8],
    ) -> ClientResult<serde_json::Map<String, serde_json::Value>> {
        let map = json_object(what, status, body)?;
        if map.get("status").and_then(|v| v.as_str()) == Some("success") {
            Ok(map)
        } else {
            Err(ClientError::Decode(format!(
                "{what}: status is not \"success\""
            )))
        }
    }

    /// Parse the `{"status": "success"}` acknowledgement of a state-changing
    /// POST (`/replace`, `/burn`). Non-2xx ⇒ `Http`; any other 2xx ⇒ `Decode`,
    /// so a proxy page or truncated body is never mistaken for a settled write.
    pub fn parse_success(what: &str, status: u16, body: &[u8]) -> ClientResult<()> {
        success_object(what, status, body).map(|_| ())
    }

    fn field_u32(
        what: &str,
        map: &serde_json::Map<String, serde_json::Value>,
        key: &str,
    ) -> ClientResult<u32> {
        map.get(key)
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| ClientError::Decode(format!("{what}: {key} is not a u32")))
    }

    /// Decimal amount field: a JSON string or number of the form `d+(.d{1,8})?`.
    fn field_decimal(
        what: &str,
        map: &serde_json::Map<String, serde_json::Value>,
        key: &str,
    ) -> ClientResult<String> {
        let raw = match map.get(key) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => {
                return Err(ClientError::Decode(format!(
                    "{what}: {key} is not a decimal amount"
                )))
            }
        };
        let (whole, frac) = raw.split_once('.').unwrap_or((raw.as_str(), ""));
        let ok = !whole.is_empty()
            && whole.chars().all(|c| c.is_ascii_digit())
            && frac.len() <= 8
            && frac.chars().all(|c| c.is_ascii_digit())
            && !(raw.contains('.') && frac.is_empty());
        if ok {
            Ok(raw)
        } else {
            Err(ClientError::Decode(format!(
                "{what}: {key} is not a decimal amount: {raw:?}"
            )))
        }
    }

    /// Strictly parse a `/target` response. Non-2xx ⇒ `Http`; any missing or
    /// mistyped field ⇒ `Decode` (a miner must never hash against a guessed
    /// difficulty).
    pub fn parse_target(status: u16, body: &[u8]) -> ClientResult<Target> {
        const WHAT: &str = "target";
        let map = json_object(WHAT, status, body)?;
        let ratio = map
            .get("ratio")
            .and_then(|v| v.as_f64())
            .filter(|r| r.is_finite() && (0.0..=1.0).contains(r))
            .ok_or_else(|| ClientError::Decode(format!("{WHAT}: ratio is not in [0, 1]")))?;
        Ok(Target {
            difficulty_target_bits: field_u32(WHAT, &map, "difficulty_target_bits")?,
            epoch: field_u32(WHAT, &map, "epoch")?,
            mining_amount: field_decimal(WHAT, &map, "mining_amount")?,
            mining_subsidy_amount: field_decimal(WHAT, &map, "mining_subsidy_amount")?,
            ratio,
        })
    }

    /// Strictly parse a `/mining_report` response. Non-2xx ⇒ `Http` (the body
    /// carries the server's rejection reason); a 2xx whose `status` is not
    /// `"success"`, or whose `difficulty_target` is present but not a u32 ⇒
    /// `Decode`.
    pub fn parse_mining_report(status: u16, body: &[u8]) -> ClientResult<MiningReportAck> {
        const WHAT: &str = "mining_report";
        let map = success_object(WHAT, status, body)?;
        let difficulty_target = match map.get("difficulty_target") {
            None | Some(serde_json::Value::Null) => None,
            Some(_) => Some(field_u32(WHAT, &map, "difficulty_target")?),
        };
        Ok(MiningReportAck { difficulty_target })
    }
}

/// Minimal asset-agnostic blocking client (native only). One instance per
/// server URL; methods correspond 1:1 to the server's endpoint set and are
/// thin shells over [`wire`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct Client {
    base_url: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl Client {
    /// Construct a client bound to a server base URL, e.g.
    /// `http://localhost:8181` or `https://webcash.org`.
    ///
    /// ```
    /// use webylib_server_client::Client;
    /// // Trailing slash on the base is normalised away.
    /// let c = Client::new("http://localhost:8181/");
    /// assert_eq!(c.base_url(), "http://localhost:8181/");
    /// ```
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    /// Return the base URL the client was constructed with (verbatim;
    /// no normalisation).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    #[cfg(test)]
    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    /// `POST /api/v1/replace` — splittable: N inputs → M outputs with
    /// amount conservation. Non-splittable: 1:1 (same body shape; the
    /// server enforces arity per its compiled flavor).
    pub fn replace(&self, inputs: &[String], outputs: &[String]) -> ClientResult<()> {
        self.settle("replace", &wire::replace(inputs, outputs)?)
    }

    /// `POST /api/v1/replace` with the HTLC extensions (RGB server only;
    /// webcash and voucher servers ignore them). See
    /// `webycash-server/docs/referee-zkp-based-swap.md` §7.1.
    pub fn replace_with_htlc(
        &self,
        inputs: &[String],
        outputs: &[String],
        htlc_locks: &[HtlcLockEntry],
        htlc_witnesses: &[HtlcWitnessEntry],
    ) -> ClientResult<()> {
        self.settle(
            "replace",
            &wire::replace_with_htlc(inputs, outputs, htlc_locks, htlc_witnesses)?,
        )
    }

    /// `POST /api/v1/burn` — mark a single secret spent permanently.
    pub fn burn(&self, secret_token: &str) -> ClientResult<()> {
        self.settle("burn", &wire::burn(secret_token)?)
    }

    /// `POST /api/v1/health_check` — returns the raw 2xx response body.
    /// Prefer [`Client::health_check_status`] (typed, strict).
    pub fn health_check(&self, public_tokens: &[String]) -> ClientResult<String> {
        self.execute(&wire::health_check(public_tokens)?)
    }

    /// `POST /api/v1/health_check`, strictly parsed: one [`TokenStatus`] per
    /// requested token, in order.
    pub fn health_check_status(&self, public_tokens: &[String]) -> ClientResult<Vec<TokenStatus>> {
        let (status, body) = self.send(&wire::health_check(public_tokens)?)?;
        wire::parse_health_check(status, body.as_bytes(), public_tokens)
    }

    /// `POST /api/v1/mining_report` — submit a PoW preimage.
    pub fn mining_report(&self, preimage: &str) -> ClientResult<wire::MiningReportAck> {
        let (status, body) = self.send(&wire::mining_report(preimage)?)?;
        wire::parse_mining_report(status, body.as_bytes())
    }

    /// `POST /api/v1/issue` — operator-signed mint. Caller supplies the
    /// canonical request body bytes AND the detached Ed25519 signature.
    pub fn issue(&self, body: &[u8], sig_hex: &str) -> ClientResult<()> {
        self.execute(&wire::issue(body.to_vec(), sig_hex)?)
            .map(|_| ())
    }

    /// `GET /api/v1/target` — current mining target.
    pub fn target(&self) -> ClientResult<wire::Target> {
        let (status, body) = self.send(&wire::target())?;
        wire::parse_target(status, body.as_bytes())
    }

    /// `GET /api/v1/stats` — economy statistics.
    pub fn stats(&self) -> ClientResult<String> {
        self.execute(&wire::stats())
    }

    fn settle(&self, what: &str, req: &wire::Request) -> ClientResult<()> {
        let (status, body) = self.send(req)?;
        wire::parse_success(what, status, body.as_bytes())
    }

    fn execute(&self, req: &wire::Request) -> ClientResult<String> {
        let (status, body) = self.send(req)?;
        wire::check(status, body.as_bytes())
    }

    /// Native transport: HTTP+HTTPS via ureq (rustls). Sends exactly the
    /// [`wire::Request`] headers; non-2xx still yields `(status, body)`.
    fn send(&self, req: &wire::Request) -> ClientResult<(u16, String)> {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .build();
        let mut r = agent.request(req.method.as_str(), &req.url(&self.base_url));
        for (k, v) in &req.headers {
            r = r.set(k, v);
        }
        let result = match req.method {
            wire::Method::Get => r.call(),
            wire::Method::Post => r.send_bytes(&req.body),
        };
        let resp = match result {
            Ok(resp) => resp,
            Err(ureq::Error::Status(_, resp)) => resp,
            Err(e) => return Err(ClientError::Transport(e.to_string())),
        };
        let status = resp.status();
        let body = resp
            .into_string()
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        Ok((status, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "ab00000000000000000000000000000000000000000000000000000000000001";

    fn pubtok(amount: &str) -> String {
        format!("e{amount}:public:{H}")
    }

    #[test]
    fn endpoint_drops_trailing_slash_from_base() {
        let c1 = Client::new("http://x:1");
        let c2 = Client::new("http://x:1/");
        assert_eq!(c1.endpoint("/api/v1/replace"), "http://x:1/api/v1/replace");
        assert_eq!(c2.endpoint("/api/v1/replace"), "http://x:1/api/v1/replace");
        assert_eq!(
            wire::target().url("https://webcash.org/"),
            "https://webcash.org/api/v1/target"
        );
    }

    #[test]
    fn every_rail_post_is_cors_simple_text_plain() {
        let reqs = [
            wire::health_check(&[pubtok("1")]).unwrap(),
            wire::replace(&["a".into()], &["b".into()]).unwrap(),
            wire::burn("e1:secret:00").unwrap(),
            wire::mining_report("{}").unwrap(),
        ];
        for r in reqs {
            assert_eq!(r.method, wire::Method::Post);
            assert_eq!(
                r.headers,
                vec![("Content-Type", "text/plain;charset=UTF-8".to_string())],
                "{}",
                r.path
            );
            serde_json::from_slice::<serde_json::Value>(&r.body).expect("JSON body");
        }
    }

    #[test]
    fn no_request_ever_carries_a_json_content_type() {
        let issue = wire::issue(b"{}".to_vec(), "aa").unwrap();
        for (_, v) in issue.headers {
            assert!(!v.contains("application/json"));
        }
    }

    #[test]
    fn health_check_refuses_secrets_and_empty() {
        assert!(matches!(
            wire::health_check(&[format!("e1:secret:{H}")]),
            Err(ClientError::Encode(_))
        ));
        assert!(matches!(
            wire::health_check(&[]),
            Err(ClientError::Encode(_))
        ));
    }

    #[test]
    fn health_check_body_is_bare_array() {
        let r = wire::health_check(&[pubtok("0.5")]).unwrap();
        assert_eq!(r.path, "/api/v1/health_check");
        assert_eq!(
            String::from_utf8(r.body).unwrap(),
            format!("[\"{}\"]", pubtok("0.5"))
        );
    }

    #[test]
    fn parse_health_check_three_states_matched_by_hash() {
        let req = vec![pubtok("1.0")];
        let body = format!(
            "{{\"status\":\"success\",\"results\":{{\"{}\":{{\"spent\":false,\"amount\":\"1\"}}}}}}",
            pubtok("1")
        );
        assert_eq!(
            wire::parse_health_check(200, body.as_bytes(), &req).unwrap(),
            vec![TokenStatus::Unspent {
                amount: Some("1".into())
            }]
        );
        let spent = format!(
            "{{\"results\":{{\"{}\":{{\"spent\":true,\"amount\":null}}}}}}",
            pubtok("1")
        );
        assert_eq!(
            wire::parse_health_check(200, spent.as_bytes(), &req).unwrap(),
            vec![TokenStatus::Spent { amount: None }]
        );
        let unknown = format!("{{\"results\":{{\"{}\":{{\"spent\":null}}}}}}", pubtok("1"));
        assert_eq!(
            wire::parse_health_check(200, unknown.as_bytes(), &req).unwrap(),
            vec![TokenStatus::Unknown]
        );
        assert_eq!(
            wire::parse_health_check(200, b"{\"results\":{}}", &req).unwrap(),
            vec![TokenStatus::Unknown]
        );
    }

    #[test]
    fn parse_health_check_fails_closed() {
        let req = vec![pubtok("1")];
        assert!(matches!(
            wire::parse_health_check(500, b"boom", &req),
            Err(ClientError::Http { status: 500, .. })
        ));
        assert!(matches!(
            wire::parse_health_check(200, b"<html>", &req),
            Err(ClientError::Decode(_))
        ));
        assert!(matches!(
            wire::parse_health_check(200, b"{\"status\":\"success\"}", &req),
            Err(ClientError::Decode(_))
        ));
        let bad = format!(
            "{{\"results\":{{\"{}\":{{\"spent\":\"no\"}}}}}}",
            pubtok("1")
        );
        assert!(matches!(
            wire::parse_health_check(200, bad.as_bytes(), &req),
            Err(ClientError::Decode(_))
        ));
        let bad_amt = format!(
            "{{\"results\":{{\"{}\":{{\"spent\":false,\"amount\":[1]}}}}}}",
            pubtok("1")
        );
        assert!(matches!(
            wire::parse_health_check(200, bad_amt.as_bytes(), &req),
            Err(ClientError::Decode(_))
        ));
        assert!(matches!(
            wire::parse_health_check(200, b"{\"results\":{\"junk\":{\"spent\":true}}}", &req),
            Err(ClientError::Decode(_))
        ));
    }

    #[test]
    fn namespaced_public_tokens_are_accepted_and_matched() {
        let ns = format!("e1:public:{H}:rgb20-usdc:aabb");
        assert_eq!(wire::public_hash(&ns), Some(format!("{H}:rgb20-usdc:aabb")));
        assert!(wire::public_hash(&format!("e1:public:{H}:")).is_none());
        let rgb21 = format!("public:{H}:rgb21-tickets:ccdd");
        assert_eq!(
            wire::public_hash(&rgb21),
            Some(format!("{H}:rgb21-tickets:ccdd"))
        );
        assert!(wire::public_hash(&format!("public:{H}")).is_none());
        assert!(wire::public_hash(&format!("e1:secret:{H}")).is_none());
        let body = format!("{{\"results\":{{\"{ns}\":{{\"spent\":false,\"amount\":\"1\"}}}}}}");
        assert!(wire::parse_health_check(200, body.as_bytes(), &[ns]).unwrap()[0].is_unspent());
    }

    #[test]
    fn check_maps_status() {
        assert_eq!(wire::check(204, b"").unwrap(), "");
        assert!(matches!(
            wire::check(404, b"x"),
            Err(ClientError::Http { status: 404, .. })
        ));
    }

    #[test]
    fn parse_success_requires_success_status() {
        assert!(wire::parse_success("replace", 200, br#"{"status": "success"}"#).is_ok());
        for body in [
            &br#"{"status":"error"}"#[..],
            b"{}",
            b"<html>ok</html>",
            b"",
        ] {
            assert!(
                matches!(
                    wire::parse_success("replace", 200, body),
                    Err(ClientError::Decode(_))
                ),
                "{body:?}"
            );
        }
        assert!(matches!(
            wire::parse_success("replace", 500, b"can only be replaced by itself"),
            Err(ClientError::Http { status: 500, .. })
        ));
    }

    #[test]
    fn parse_target_is_strict() {
        let ok = br#"{"difficulty_target_bits":28,"epoch":3,"mining_amount":"200000","mining_subsidy_amount":10000.5,"ratio":0.25}"#;
        let t = wire::parse_target(200, ok).unwrap();
        assert_eq!(t.difficulty_target_bits, 28);
        assert_eq!(t.epoch, 3);
        assert_eq!(t.mining_amount, "200000");
        assert_eq!(t.mining_subsidy_amount, "10000.5");
        assert_eq!(t.ratio, 0.25);
        let bad: [&[u8]; 6] = [
            br#"{"epoch":3,"mining_amount":"1","mining_subsidy_amount":"1","ratio":0}"#,
            br#"{"difficulty_target_bits":-1,"epoch":3,"mining_amount":"1","mining_subsidy_amount":"1","ratio":0}"#,
            br#"{"difficulty_target_bits":1,"epoch":3,"mining_amount":"1e9","mining_subsidy_amount":"1","ratio":0}"#,
            br#"{"difficulty_target_bits":1,"epoch":3,"mining_amount":"1.123456789","mining_subsidy_amount":"1","ratio":0}"#,
            br#"{"difficulty_target_bits":1,"epoch":3,"mining_amount":"1","mining_subsidy_amount":"1","ratio":1.5}"#,
            b"[]",
        ];
        for body in bad {
            assert!(
                matches!(wire::parse_target(200, body), Err(ClientError::Decode(_))),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert!(matches!(
            wire::parse_target(503, b"down"),
            Err(ClientError::Http { status: 503, .. })
        ));
    }

    #[test]
    fn parse_mining_report_requires_success() {
        let ack = wire::parse_mining_report(200, br#"{"status":"success","difficulty_target":26}"#)
            .unwrap();
        assert_eq!(ack.difficulty_target, Some(26));
        let bare = wire::parse_mining_report(200, br#"{"status":"success"}"#).unwrap();
        assert_eq!(bare.difficulty_target, None);
        assert!(matches!(
            wire::parse_mining_report(200, br#"{"status":"error"}"#),
            Err(ClientError::Decode(_))
        ));
        assert!(matches!(
            wire::parse_mining_report(200, br#"{"status":"success","difficulty_target":"x"}"#),
            Err(ClientError::Decode(_))
        ));
        assert!(matches!(
            wire::parse_mining_report(400, b"Didn't use a new secret value."),
            Err(ClientError::Http { status: 400, .. })
        ));
    }

    #[test]
    fn replace_fails_with_transport_error_on_unreachable_url() {
        let c = Client::new("http://127.0.0.1:1"); // port 1 is reserved/closed
        let err = c
            .replace(&["a".into()], &["b".into()])
            .expect_err("must fail to connect");
        assert!(matches!(err, ClientError::Transport(_)), "got {err:?}");
    }
}

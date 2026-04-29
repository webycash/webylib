//! Minimal asset-agnostic HTTP client for the webycash-server family.
//!
//! Every flavor (Webcash, RGB Fungible, RGB Collectible, Voucher) speaks
//! the same wire protocol — `/api/v1/replace`, `/health_check`, `/burn`,
//! `/mining_report`, `/issue` — only the token wire format differs. This
//! crate ships ONE Client; the wallet crates (`wallet-webcash`,
//! `wallet-rgb`, `wallet-voucher`) wrap it in flavor-specific verbs
//! (`pay`, `transfer`, `insert`).
//!
//! The Client is sync over reqwest::blocking by default for
//! straightforward CLI integration; an async surface lives behind
//! the `async` feature.

#![forbid(unsafe_code)]

use thiserror::Error;

/// Failure modes when talking to a webycash-server flavor.
///
/// - `Http`: server returned a non-2xx status. Body is the raw
///   response (typically the Tornado-style HTML 500 envelope or a
///   JSON error from the asset-gated handler).
/// - `Transport`: TCP connect / read / write failed before a status
///   line came back.
/// - `Encode`: serde_json couldn't serialise the request body.
#[derive(Debug, Error)]
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
    /// `serde_json` couldn't serialise the request body.
    #[error("body encode error: {0}")]
    Encode(String),
}

/// Convenience alias used across the wallet crates for results from
/// any `Client` method.
pub type ClientResult<T> = Result<T, ClientError>;

/// Minimal asset-agnostic HTTP client. One instance per server URL;
/// methods correspond 1:1 to the server's endpoint set.
#[derive(Clone, Debug)]
pub struct Client {
    base_url: String,
}

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

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    /// `POST /api/v1/replace` — splittable: N inputs → M outputs with
    /// amount conservation. Non-splittable: 1:1 (same body shape; the
    /// server enforces arity per its compiled flavor).
    pub fn replace(&self, inputs: &[String], outputs: &[String]) -> ClientResult<()> {
        let body = serde_json::json!({
            "webcashes": inputs,
            "new_webcashes": outputs,
            "legalese": {"terms": true},
        });
        self.post_status(&self.endpoint("/api/v1/replace"), &body)?;
        Ok(())
    }

    /// `POST /api/v1/burn` — mark a single secret spent permanently.
    pub fn burn(&self, secret_token: &str) -> ClientResult<()> {
        let body = serde_json::json!({
            "webcash": secret_token,
            "legalese": {"terms": true},
        });
        self.post_status(&self.endpoint("/api/v1/burn"), &body)?;
        Ok(())
    }

    /// `POST /api/v1/health_check` — bare-array body of public tokens.
    /// Returns the raw response body (caller parses).
    pub fn health_check(&self, public_tokens: &[String]) -> ClientResult<String> {
        let body = serde_json::to_string(public_tokens)
            .map_err(|e| ClientError::Encode(e.to_string()))?;
        self.post_raw(&self.endpoint("/api/v1/health_check"), &body)
    }

    /// `POST /api/v1/mining_report` — submit a PoW preimage.
    pub fn mining_report(&self, preimage: &str) -> ClientResult<()> {
        let body = serde_json::json!({
            "preimage": preimage,
            "legalese": {"terms": true},
        });
        self.post_status(&self.endpoint("/api/v1/mining_report"), &body)?;
        Ok(())
    }

    /// `POST /api/v1/issue` — operator-signed mint (RGB / Voucher only).
    /// Caller supplies the canonical request body bytes AND the detached
    /// Ed25519 signature over those bytes.
    pub fn issue(&self, body: &[u8], sig_hex: &str) -> ClientResult<()> {
        self.post_signed(&self.endpoint("/api/v1/issue"), body, sig_hex)
    }

    /// `GET /api/v1/target` — current mining target.
    pub fn target(&self) -> ClientResult<String> {
        self.get_raw(&self.endpoint("/api/v1/target"))
    }

    /// `GET /api/v1/stats` — economy statistics (circulation, epoch,
    /// mining report count, current difficulty, mining/subsidy amounts).
    pub fn stats(&self) -> ClientResult<String> {
        self.get_raw(&self.endpoint("/api/v1/stats"))
    }

    fn get_raw(&self, url: &str) -> ClientResult<String> {
        let resp = http_get(url).map_err(|e| ClientError::Transport(e.to_string()))?;
        let (status, body) = parse_resp(&resp);
        if !(200..300).contains(&status) {
            return Err(ClientError::Http { status, body });
        }
        Ok(body)
    }

    fn post_status(&self, url: &str, body: &serde_json::Value) -> ClientResult<()> {
        let body_str =
            serde_json::to_string(body).map_err(|e| ClientError::Encode(e.to_string()))?;
        let _ = self.post_raw(url, &body_str)?;
        Ok(())
    }

    fn post_raw(&self, url: &str, body: &str) -> ClientResult<String> {
        let resp = http_post(url, body, None)
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let (status, body) = parse_resp(&resp);
        if !(200..300).contains(&status) {
            return Err(ClientError::Http { status, body });
        }
        Ok(body)
    }

    fn post_signed(&self, url: &str, body: &[u8], sig_hex: &str) -> ClientResult<()> {
        let body_str =
            std::str::from_utf8(body).map_err(|e| ClientError::Encode(e.to_string()))?;
        let resp = http_post(url, body_str, Some(("X-Issuer-Signature", sig_hex)))
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let (status, body) = parse_resp(&resp);
        if !(200..300).contains(&status) {
            return Err(ClientError::Http { status, body });
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// No-deps HTTP transport. Avoids pulling reqwest into every consumer; for
// production deployments a higher-perf transport can wrap Client.
// ─────────────────────────────────────────────────────────────────────────────

fn http_get(url: &str) -> std::io::Result<String> {
    http_send(url, "GET", "", None)
}

fn http_post(url: &str, body: &str, extra: Option<(&str, &str)>) -> std::io::Result<String> {
    http_send(url, "POST", body, extra)
}

fn http_send(
    url: &str,
    method: &str,
    body: &str,
    extra: Option<(&str, &str)>,
) -> std::io::Result<String> {
    use std::io::{Read, Write};
    let after = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = after
        .split_once('/')
        .map(|(h, p)| (h.to_string(), format!("/{p}")))
        .unwrap_or((after.to_string(), "/".into()));
    let mut s = std::net::TcpStream::connect(&host_port)?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(15)))?;
    let extra_hdr = match extra {
        Some((k, v)) if !v.is_empty() => format!("{k}: {v}\r\n"),
        _ => String::new(),
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",
        body.len(),
        extra_hdr,
    );
    s.write_all(req.as_bytes())?;
    if !body.is_empty() {
        s.write_all(body.as_bytes())?;
    }
    let mut buf = Vec::new();
    s.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn parse_resp(raw: &str) -> (u16, String) {
    let status: u16 = raw
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body_start = raw.find("\r\n\r\n").map(|i| i + 4).unwrap_or(raw.len());
    (status, raw[body_start..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_drops_trailing_slash_from_base() {
        let c1 = Client::new("http://x:1");
        let c2 = Client::new("http://x:1/");
        // Same path appended in both shapes.
        assert_eq!(
            c1.endpoint("/api/v1/replace"),
            "http://x:1/api/v1/replace"
        );
        assert_eq!(
            c2.endpoint("/api/v1/replace"),
            "http://x:1/api/v1/replace"
        );
    }

    #[test]
    fn endpoint_accepts_https_base() {
        let c = Client::new("https://webcash.org");
        assert_eq!(c.endpoint("/api/v1/target"), "https://webcash.org/api/v1/target");
    }

    #[test]
    fn parse_resp_extracts_status_and_body() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true}";
        let (status, body) = parse_resp(raw);
        assert_eq!(status, 200);
        assert_eq!(body, "{\"ok\":true}");
    }

    #[test]
    fn parse_resp_handles_500() {
        let raw = "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html\r\n\r\nboom";
        let (status, body) = parse_resp(raw);
        assert_eq!(status, 500);
        assert_eq!(body, "boom");
    }

    #[test]
    fn parse_resp_unknown_status_yields_zero() {
        let raw = "garbage\r\n\r\nbody";
        let (status, _body) = parse_resp(raw);
        assert_eq!(status, 0);
    }

    #[test]
    fn parse_resp_no_blank_line_returns_empty_body() {
        let raw = "HTTP/1.1 204 No Content\r\nServer: x\r\n";
        let (status, body) = parse_resp(raw);
        // No `\r\n\r\n` separator → body offset lands at raw.len() → empty.
        assert_eq!(status, 204);
        assert_eq!(body, "");
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

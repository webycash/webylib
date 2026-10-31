//! Server communication for Webcash operations
//!
//! This module handles HTTP communication with the Webcash server for operations
//! like health checks, replacements, target queries, and mining report submissions.

use serde::Serialize;

#[cfg(any(feature = "native", feature = "wasm"))]
use crate::error::{Error, Result};
#[cfg(any(feature = "native", feature = "wasm"))]
use reqwest::Client;

/// Webcash server API endpoints
pub mod endpoints {
    /// Health check endpoint — query spend status of outputs
    pub const HEALTH_CHECK: &str = "/api/v1/health_check";
    /// Replace endpoint — atomic webcash replacement (core transaction)
    pub const REPLACE: &str = "/api/v1/replace";
    /// Target endpoint — get current mining difficulty and parameters
    pub const TARGET: &str = "/api/v1/target";
    /// Mining report endpoint — submit proof-of-work solution
    pub const MINING_REPORT: &str = "/api/v1/mining_report";
}

/// Network mode selection — the single source of truth for which server
/// the wallet communicates with. Each Wallet instance owns its network mode.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NetworkMode {
    /// Production webcash.org server.
    #[default]
    Production,
    /// Webycash testnet at weby.cash.
    Testnet,
    /// Custom server URL (e.g., localhost:8080 for local development).
    Custom(String),
}

impl NetworkMode {
    /// Base URL for the selected network.
    pub fn base_url(&self) -> &str {
        match self {
            NetworkMode::Production => "https://webcash.org",
            NetworkMode::Testnet => "https://weby.cash/api/webcash/testnet",
            NetworkMode::Custom(url) => url.as_str(),
        }
    }
}

/// Full API URL for an endpoint on a given network.
impl NetworkMode {
    pub fn endpoint_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url(), endpoint)
    }
}

// ── Async HTTP transport over the sans-IO wire protocol ─────────

#[cfg(any(feature = "native", feature = "wasm"))]
use crate::server_client::{wire, ClientError, TokenStatus};

/// Mining parameters from `/api/v1/target` (the typed [`wire::Target`]).
pub type TargetResponse = crate::server_client::wire::Target;
/// Accepted `/api/v1/mining_report` acknowledgement ([`wire::MiningReportAck`]).
pub type MiningReportResponse = crate::server_client::wire::MiningReportAck;

/// Async webcash server client. Every method builds its request with
/// [`wire`] and parses with the matching strict `wire::parse_*`, so the
/// async (reqwest / browser fetch) and blocking ([`crate::server_client::Client`])
/// transports share one protocol definition. Implemented by [`ServerClient`];
/// a trait so wallets can be tested with an injected server.
#[cfg(any(feature = "native", feature = "wasm"))]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
pub trait ServerClientTrait {
    /// Per-token server state of public tokens, in request order.
    async fn health_check(&self, public_tokens: &[String]) -> Result<Vec<TokenStatus>>;
    /// Settle an N→M replacement; `Ok` only on `{"status": "success"}`.
    async fn replace(&self, request: &ReplaceRequest) -> Result<()>;
    /// Current mining target.
    async fn get_target(&self) -> Result<TargetResponse>;
    /// Submit a solved preimage.
    async fn submit_mining_report(
        &self,
        report: &MiningReportRequest,
    ) -> Result<MiningReportResponse>;
}

#[cfg(any(feature = "native", feature = "wasm"))]
/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Network mode (determines base URL).
    pub network: NetworkMode,
    /// Request timeout in seconds.
    pub timeout_seconds: u64,
}

#[cfg(any(feature = "native", feature = "wasm"))]
impl ServerConfig {
    /// Base URL derived from the network mode.
    pub fn base_url(&self) -> &str {
        self.network.base_url()
    }
}

#[cfg(any(feature = "native", feature = "wasm"))]
impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            network: NetworkMode::default(),
            timeout_seconds: 30,
        }
    }
}

#[cfg(any(feature = "native", feature = "wasm"))]
impl From<ClientError> for Error {
    fn from(e: ClientError) -> Self {
        Error::server(e.to_string())
    }
}

#[cfg(any(feature = "native", feature = "wasm"))]
/// Webcash server client (Clone shares connection pool)
#[derive(Clone)]
pub struct ServerClient {
    client: Client,
    config: ServerConfig,
}

#[cfg(any(feature = "native", feature = "wasm"))]
impl ServerClient {
    /// Create a new server client with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(ServerConfig::default())
    }

    /// Create a new server client with custom configuration
    pub fn with_config(config: ServerConfig) -> Result<Self> {
        let builder = Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .pool_max_idle_per_host(10000)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true);
        let client = builder.build()?;
        Ok(ServerClient { client, config })
    }

    /// Send one [`wire::Request`] verbatim (exactly its headers; on wasm32
    /// that is a CORS-simple browser `fetch`). Non-2xx still yields
    /// `(status, body)`; only a failure before a status line is `Transport`.
    pub async fn send(
        &self,
        req: &wire::Request,
    ) -> std::result::Result<(u16, Vec<u8>), ClientError> {
        let url = req.url(self.config.base_url());
        let mut rb = match req.method {
            wire::Method::Get => self.client.get(&url),
            wire::Method::Post => self.client.post(&url).body(req.body.clone()),
        };
        for (k, v) in &req.headers {
            rb = rb.header(*k, v.as_str());
        }
        let resp = rb
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        Ok((status, body.to_vec()))
    }

    /// Per-token server state of public tokens, in request order.
    pub async fn health_check(&self, public_tokens: &[String]) -> Result<Vec<TokenStatus>> {
        let (status, body) = self.send(&wire::health_check(public_tokens)?).await?;
        Ok(wire::parse_health_check(status, &body, public_tokens)?)
    }

    /// Submit a replacement request to the server
    pub async fn replace(&self, request: &ReplaceRequest) -> Result<()> {
        if !request.legalese.terms {
            return Err(Error::TermsNotAccepted);
        }
        let req = wire::replace(&request.webcashes, &request.new_webcashes)?;
        let (status, body) = self.send(&req).await?;
        Ok(wire::parse_success("replace", status, &body)?)
    }

    /// Get current mining target information
    pub async fn get_target(&self) -> Result<TargetResponse> {
        let (status, body) = self.send(&wire::target()).await?;
        Ok(wire::parse_target(status, &body)?)
    }

    /// Submit a mining report
    pub async fn submit_mining_report(
        &self,
        report: &MiningReportRequest,
    ) -> Result<MiningReportResponse> {
        if !report.legalese.terms {
            return Err(Error::TermsNotAccepted);
        }
        let (status, body) = self.send(&wire::mining_report(&report.preimage)?).await?;
        Ok(wire::parse_mining_report(status, &body)?)
    }
}

#[cfg(any(feature = "native", feature = "wasm"))]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl ServerClientTrait for ServerClient {
    async fn health_check(&self, public_tokens: &[String]) -> Result<Vec<TokenStatus>> {
        self.health_check(public_tokens).await
    }
    async fn replace(&self, request: &ReplaceRequest) -> Result<()> {
        self.replace(request).await
    }
    async fn get_target(&self) -> Result<TargetResponse> {
        self.get_target().await
    }
    async fn submit_mining_report(
        &self,
        report: &MiningReportRequest,
    ) -> Result<MiningReportResponse> {
        self.submit_mining_report(report).await
    }
}

/// Replacement request
#[derive(Debug, Serialize)]
pub struct ReplaceRequest {
    pub webcashes: Vec<String>,
    pub new_webcashes: Vec<String>,
    pub legalese: Legalese,
}

/// Terms acceptance
#[derive(Debug, Serialize)]
pub struct Legalese {
    pub terms: bool,
}

/// Mining report request
#[derive(Debug, Serialize)]
pub struct MiningReportRequest {
    pub preimage: String,
    pub legalese: Legalese,
}

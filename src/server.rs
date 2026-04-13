//! Server communication for Webcash operations
//!
//! This module handles HTTP communication with the Webcash server for operations
//! like health checks, replacements, target queries, and mining report submissions.

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::webcash::PublicWebcash;

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

/// Cross-platform server client trait
#[async_trait::async_trait]
pub trait ServerClientTrait {
    async fn health_check(&self, webcash: &[PublicWebcash]) -> Result<HealthResponse>;
    async fn replace(&self, request: &ReplaceRequest) -> Result<ReplaceResponse>;
    async fn get_target(&self) -> Result<TargetResponse>;
    async fn submit_mining_report(
        &self,
        report: &MiningReportRequest,
    ) -> Result<MiningReportResponse>;
}

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Base URL of the Webcash server
    pub base_url: String,
    /// Request timeout in seconds
    pub timeout_seconds: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            base_url: "https://webcash.org".to_string(),
            timeout_seconds: 30,
        }
    }
}

/// Webcash server client (Clone shares connection pool)
#[derive(Clone)]
pub struct ServerClient {
    client: Client,
    config: ServerConfig,
}

impl ServerClient {
    /// Create a new server client with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(ServerConfig::default())
    }

    /// Create a new server client with custom configuration
    pub fn with_config(config: ServerConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .pool_max_idle_per_host(10000)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true)
            .build()?;

        Ok(ServerClient { client, config })
    }

    /// Check the health status of webcash entries
    pub async fn health_check(&self, webcash: &[PublicWebcash]) -> Result<HealthResponse> {
        let mut request_data = Vec::new();
        for wc in webcash {
            request_data.push(wc.to_string());
        }

        let url = format!("{}{}", self.config.base_url, endpoints::HEALTH_CHECK);
        let response = self.client.post(&url).json(&request_data).send().await?;

        if !response.status().is_success() {
            return Err(Error::server("Health check request failed"));
        }

        let health_response: HealthResponse = response.json().await?;
        Ok(health_response)
    }

    /// Submit a replacement request to the server
    pub async fn replace(&self, request: &ReplaceRequest) -> Result<ReplaceResponse> {
        let url = format!("{}{}", self.config.base_url, endpoints::REPLACE);

        let response = self.client.post(&url).json(request).send().await?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            // Try to parse error response for detailed error message
            if let Ok(error_response) = serde_json::from_str::<serde_json::Value>(&response_text) {
                if let Some(error_msg) = error_response.get("error").and_then(|v| v.as_str()) {
                    return Err(Error::server(format!(
                        "Replace request failed: {}",
                        error_msg
                    )));
                }
            }
            return Err(Error::server(format!(
                "Replace request failed with status {}: {}",
                status, response_text
            )));
        }

        let replace_response: ReplaceResponse = serde_json::from_str(&response_text)?;
        Ok(replace_response)
    }

    /// Get current mining target information
    pub async fn get_target(&self) -> Result<TargetResponse> {
        let url = format!("{}{}", self.config.base_url, endpoints::TARGET);
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(Error::server("Target request failed"));
        }

        let target_response: TargetResponse = response.json().await?;
        Ok(target_response)
    }

    /// Submit a mining report
    pub async fn submit_mining_report(
        &self,
        report: &MiningReportRequest,
    ) -> Result<MiningReportResponse> {
        let url = format!("{}{}", self.config.base_url, endpoints::MINING_REPORT);
        let response = self.client.post(&url).json(report).send().await?;

        if !response.status().is_success() {
            return Err(Error::server("Mining report submission failed"));
        }

        let mining_response: MiningReportResponse = response.json().await?;
        Ok(mining_response)
    }
}

#[async_trait::async_trait]
impl ServerClientTrait for ServerClient {
    async fn health_check(&self, webcash: &[PublicWebcash]) -> Result<HealthResponse> {
        self.health_check(webcash).await
    }
    async fn replace(&self, request: &ReplaceRequest) -> Result<ReplaceResponse> {
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

/// Health check response
#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub results: std::collections::HashMap<String, HealthResult>,
}

/// Individual health check result
#[derive(Debug, Deserialize)]
pub struct HealthResult {
    pub spent: Option<bool>,
    pub amount: Option<String>,
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

/// Replacement response
#[derive(Debug, Deserialize)]
pub struct ReplaceResponse {
    pub status: String,
}

/// Target information response
#[derive(Debug, Deserialize)]
pub struct TargetResponse {
    pub difficulty_target_bits: u32,
    pub epoch: u32,
    pub mining_amount: String,
    pub mining_subsidy_amount: String,
    pub ratio: f64,
}

/// Mining report request
#[derive(Debug, Serialize)]
pub struct MiningReportRequest {
    pub preimage: String,
    pub legalese: Legalese,
}

/// Mining report response
#[derive(Debug, Deserialize)]
pub struct MiningReportResponse {
    pub status: String,
    pub difficulty_target: Option<u32>,
}

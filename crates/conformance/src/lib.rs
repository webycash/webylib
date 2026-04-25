//! Wallet-side conformance suite.
//!
//! Two harnesses, both gated behind feature flags so offline CI runs stay
//! fast and deterministic:
//!
//! - `live-webcash-org` — runs `webylib-wallet-webcash` against
//!   `https://webcash.org`. Detects wire-format drift from production.
//! - `docker-local` — drives `server-{webcash,rgb,voucher}` booted via
//!   `webycash-server/docker-compose.local.yml`. Exercises the full lifecycle
//!   per flavor: mint/issue → balance → split → transfer → recover.

pub mod ports {
    /// The local-compose port for each flavor binary.
    pub const SERVER_WEBCASH: u16 = 8181;
    pub const SERVER_RGB: u16 = 8182;
    pub const SERVER_VOUCHER: u16 = 8183;

    pub fn url(port: u16) -> String {
        format!("http://localhost:{port}")
    }
}

#[cfg(feature = "docker-local")]
pub mod docker_local {
    //! Lifecycle suite that targets the running compose. Feature-gated so the
    //! default `cargo test` doesn't try to dial localhost ports.
    //!
    //! Each flavor's lifecycle test is a separate function so failures are
    //! attributable. Test bodies land per-milestone:
    //!   - Webcash flow → M2
    //!   - RGB20 / RGB21 flow → M4
    //!   - Voucher flow → M6

    use super::ports;

    /// Reachability ping used by every per-flavor test before doing real work.
    /// Returns `Ok(())` only if the server's `/api/v1/target` responds 200.
    pub async fn assert_reachable(_url: &str) -> Result<(), String> {
        // Wired in M2 alongside the webylib-server-client native impl.
        Err("docker-local reachability check lands in M2".into())
    }

    pub async fn webcash_lifecycle() -> Result<(), String> {
        let _url = ports::url(ports::SERVER_WEBCASH);
        Err("webcash lifecycle stub — implementation lands in M2".into())
    }

    pub async fn rgb20_split_lifecycle() -> Result<(), String> {
        let _url = ports::url(ports::SERVER_RGB);
        Err("RGB20 splittable lifecycle stub — implementation lands in M4".into())
    }

    pub async fn rgb21_transfer_lifecycle() -> Result<(), String> {
        let _url = ports::url(ports::SERVER_RGB);
        Err("RGB21 non-splittable lifecycle stub — implementation lands in M4".into())
    }

    pub async fn voucher_lifecycle() -> Result<(), String> {
        let _url = ports::url(ports::SERVER_VOUCHER);
        Err("voucher lifecycle stub — implementation lands in M6".into())
    }
}

#[cfg(feature = "live-webcash-org")]
pub mod live_webcash_org {
    //! Wallet-against-production smoke. Detects protocol drift early.
    //!
    //! Lands in M2 alongside the refactored Webcash flavor of webylib.
    pub async fn run_smoke() -> Result<(), String> {
        Err("webcash.org live smoke stub — implementation lands in M2".into())
    }
}

#[cfg(test)]
mod tests {
    use super::ports;

    #[test]
    fn ports_are_distinct() {
        assert_ne!(ports::SERVER_WEBCASH, ports::SERVER_RGB);
        assert_ne!(ports::SERVER_RGB, ports::SERVER_VOUCHER);
        assert_ne!(ports::SERVER_WEBCASH, ports::SERVER_VOUCHER);
    }

    #[test]
    fn ports_match_compose_file() {
        // Hardcoded against webycash-server/docker-compose.local.yml. If that
        // file changes its host-port mapping, update this test AND the constants.
        assert_eq!(ports::SERVER_WEBCASH, 8181);
        assert_eq!(ports::SERVER_RGB, 8182);
        assert_eq!(ports::SERVER_VOUCHER, 8183);
    }
}

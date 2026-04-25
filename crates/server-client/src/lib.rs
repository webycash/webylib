//! HTTP client for the asset-gated server.
//!
//! `AssetServerClient<A: Asset>`. Endpoints that don't apply to an asset
//! (e.g., `/issue` for Webcash) are absent from the trait at compile time.
//! Native uses `reqwest`; WASM uses browser fetch via `wasm-bindgen-futures`.

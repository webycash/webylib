//! Wallet-side wire-format types and parsers.
//!
//! Mirrors server's `webycash-proto`. Either crate can publish first; the
//! grammar is canonical (the protocol must not drift between client and
//! server). Property tests cross-check both implementations against the
//! conformance fixtures.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

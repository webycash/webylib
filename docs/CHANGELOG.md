# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial Rust implementation of Webcash wallet library
- Core data types: `Amount`, `SecretWebcash`, `PublicWebcash`
- Cryptographic utilities with SHA256 and secure random generation
- SQLite wallet storage with basic operations
- Server communication with Webcash LLC API
- HD wallet key derivation framework
- Comprehensive error handling and security hardening
- Unit tests for all implemented components
- Documentation infrastructure and development guides

### Changed
- Project renamed from `webclib` to `webylib`
- Binary renamed from `webcash-cli` to `webyc`
- Corrected understanding of Webcash as centralized system operated by Webcash LLC

### Security
- Implemented `SecureString` with zeroize-on-drop for sensitive data
- Added saturating arithmetic to prevent overflow in `Amount` operations
- Removed all unsafe code blocks
- Memory-safe implementation using Rust ownership system

## [0.1.0] - 2025-09-09

### Added
- Core cryptographic primitives (SHA256, secure random)
- Amount type with 8-decimal precision and overflow protection
- Webcash data structures with proper serialization
- Basic error handling with comprehensive error types
- Project infrastructure (Cargo.toml, build system, CI)
- Initial documentation and development setup guides

### Security
- Memory-safe implementation with no unsafe code

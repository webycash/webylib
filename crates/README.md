# crates/

Workspace layout for the asset-gated wallet companion to
[webycash-server](https://github.com/webycash/webycash-server)'s
`refactor/asset-traits` branch.

## Status legend

`★` — production-ready in this branch.
`◯` — M0/M2 stub; populated in the matching milestone.

## Crate index

```
★ wallet-webcash        WebcashWallet wrapper around server-client.
                        Verbs: pay (N→M with conservation), insert
                        (1:1 receive-and-rotate). Maps to the server's
                        single /api/v1/replace endpoint.

★ wallet-rgb            RgbWallet covering BOTH RGB20 fungible and
                        RGB21 collectible. Verbs: transfer (any arity
                        for RGB20, 1:1 for RGB21), insert (1:1
                        same-namespace).

★ wallet-voucher        VoucherWallet — always-splittable
                        (contract_id, issuer_fp)-namespaced bearer
                        credits. Verbs: pay, insert.

★ server-client         Minimal asset-agnostic HTTP client for the
                        webycash-server family. One Client; the wallet
                        crates wrap it in flavor-specific verbs.
                        7 endpoints: replace, burn, health_check,
                        mining_report, issue, target, stats.

★ webylib-cli           webyca multi-asset CLI binary. 11 verbs:
                        flavor-tagged (webcash/rgb/voucher × pay-or-
                        transfer/insert), flavor-agnostic (target,
                        stats, check, burn, mining-report), local-only
                        (derive-public, verify).

★ storage               Wallet-side Store trait + three backends
                        (MemStore, JsonStore, SqliteStore). 11 cross-
                        backend conformance scenarios × 3 backends =
                        33 effective tests pinning semantic agreement
                        (atomic rollback, dedup, constraint violation).

★ webylib-wasm          WASM wallet target with client-side AluVM
                        contract execution (validation runs in-browser
                        before /replace is submitted).

◯ auth                  Wallet-side PGP keyring (issuer fingerprints).
                        rpgp 0.19 (pure-Rust, WASM-friendly).
                        Populated in M4.

◯ hd                    HD derivation — `legacy_webcash` (4-chain
                        SHA256, frozen) + `bip32` (BIP32/39/44 via
                        bitcoin::bip32, families rgb=1, voucher=6).
                        Populated in M2 / M4.

◯ crypto                Wallet crypto primitives (SHA256, Argon2,
                        AES-256-GCM, Ed25519). Migrating from the
                        legacy webylib crate.

◯ proto-client          Mirror of server's proto crate for client-side
                        wire-format parsing. Populated in M2.

◯ webylib-core          Generic Wallet<A: Asset> + ops/{insert, pay,
                        replace, recover, check, stats, issue}.

◯ webylib-ffi           cdylib + staticlib: macro-generated C ABI,
                        owns tokio runtime, native-only. Populated
                        when M2.C macro lands.

◯ macros                #[wallet_op] / #[asset_storage] /
                        #[ffi_export] proc macros. M2.C scaffolding.

◯ conformance           Live cross-check against webcash.org
                        production server. M2 follow-up.
```

## Building and testing

```bash
# Workspace lib + bin tests + doctests (no compose required)
cargo test --workspace --lib --doc

# Storage cross-backend conformance (offline)
cargo test -p webylib-storage --test backend_conformance

# Wallet-API path against running compose
docker compose -f ../webycash-server/docker-compose.local.yml up -d
cargo test --test wallet_verbs_compose

# webyca CLI end-to-end suite (basic + comprehensive)
cargo test --test cli_compose
cargo test --test full_e2e

# Full 4-flavor lifecycle (mines, replaces, burns each flavor)
cargo test --test all_flavors_compose

# WASM build sanity check
cargo build -p webylib-wasm --target wasm32-unknown-unknown
```

See the workspace [README](../README.md) for the user-facing
multi-asset CLI walkthrough and the
[server's ROADMAP](https://github.com/webycash/webycash-server/blob/refactor/asset-traits/ROADMAP.md#v040--asset-gated-server-family-refactorasset-traits-branch)
for the cross-repo milestone status.

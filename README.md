# Lumi Solana contracts

Lumi’s Anchor program for direct USDC gifts, creator memberships and capped recurring payments on Solana. Each payment sends 99.9% to the creator and 0.1% to the treasury, rounding the treasury share down in integer token base units. Network fees and account rent are separate SOL costs.

This repository contains the contract source and Rust tests. The Lumi application, onboarding service, indexer, client and full-stack E2E suite are maintained separately in the private app repository. The program and instruction names retain `infx_support` for compatibility.

## Build and test

Install Rust 1.86.0 and Solana/Agave 2.1.18 (including `cargo build-sbf`).

```sh
cargo test --locked
cargo fmt --check
cargo build-sbf --tools-version v1.51
```

Use the standard build for calendar-month billing. The app pins this repository as a Git submodule at `programs/infx-support`; initialize recursive submodules after cloning the app.

## Disposable local clock

The optional `local-clock` feature replaces each monthly interval with a build-time test interval. It retains consent, payment caps, cancellation, price/benefit checks and the 72-hour retry window. The app’s local runner builds it into a separate directory and only loads it into a fresh local validator.

```sh
LUMI_LOCAL_RENEWAL_SECONDS=5 cargo build-sbf --tools-version v1.51 --sbf-out-dir .local/clock-5 -- --features local-clock
```

Never deploy a `local-clock` artifact to a public cluster. The default build has no accelerated clock. The interval must be 5–86400 seconds. No program keypair, authority key, treasury secret or deployment credentials are included.

## Security

Findings from security review are tracked in [`docs/security/contract-findings.md`](docs/security/contract-findings.md), in priority order with a status and the test that pins each one; [`docs/security/README.md`](docs/security/README.md) says how to keep it current. `tests/calendar_properties.rs` checks the fee split and the calendar arithmetic against an independent implementation. Behavioural assertions against real transactions live in the app repository's validator suite.

## Scope

This is source publication, not an audit or a mainnet launch. There is no custody or refund instruction. Renewals require capped wallet consent, expire after their retry window, and can be stopped by cancelling membership or revoking the token allowance. The program is currently marked `UNLICENSED`; publication does not grant a software license.

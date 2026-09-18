# Lumi Solana contracts

Lumi’s Anchor program for direct USDC and wrapped-SOL gifts, creator memberships and capped recurring payments on Solana. Each payment sends 99.9% to the creator and 0.1% to the treasury, rounding the treasury share down in integer token base units. Network fees and account rent are separate SOL costs.

This repository contains the contract source and Rust tests. The Lumi application, onboarding service, indexer, client and full-stack E2E suite are maintained separately in the private app repository. The Rust crate is `lumi`; the program module, IDL and build artifact use `lumi`. The naming change preserves the program address, instruction names and discriminators, account layouts and PDA seeds, so existing accounts remain compatible.

## Build and test

Install Rust 1.86.0 and Solana/Agave 2.1.18 (including `cargo build-sbf`).

```sh
cargo test --locked
cargo fmt --check
cargo build-sbf --tools-version v1.51
```

Use the standard build for calendar-month billing. The app pins this repository as a Git submodule at `programs/lumi`; initialize recursive submodules after cloning the app.

## SOL payments (0.2.0)

Native SOL is wrapped in the supporter's own classic-token account before payment. The creator and treasury receive wrapped SOL. The program does not custody the renewal balance or convert currencies.

SOL uses the native mint `So11111111111111111111111111111111111111112`. Plan index `4294967295` is reserved for one fixed-terms SOL plan per registered creator: 0.001 SOL minimum, calendar-month periods, no perks. A supporter can pay its rent on the first payment; repeating initialization cannot reset creator closure or existing terms. Each monthly amount is capped at 50 SOL; consent permits at most twelve further payments at that exact amount. These are SOL amounts, with no dollar peg.

Existing instructions and account layouts remain compatible. Membership payments and consent select their mint from the plan index, preventing a USDC mandate from spending SOL. SOL recipients must be canonical wrapped-SOL accounts. A SOL gift, membership charge or renewal must append the currently configured USDC treasury account as a read-only remaining account: its verified token-account owner determines the SOL treasury recipient. Old USDC instructions need no extra account. SOL payments emit separate `SolSupportReceived`, `SolMembershipCharged` and `SolPlanCreated` events.

App integration tests execute the SBF program, including atomic wrapping/payment/consent, exact fees, wrong recipients and treasury proof, currency substitution, renewal expiry and exhaustion, manual-renewal reconciliation, top-ups, revocation and creator closure. Publication of this source does not establish that a cluster has been upgraded; consult the app's deployment records.

## Disposable local clock

The optional `local-clock` feature replaces each monthly interval with a build-time test interval. It retains consent, payment caps, cancellation, price/benefit checks and the 72-hour retry window. The app’s local runner builds it into a separate directory and only loads it into a fresh local validator.

```sh
LUMI_LOCAL_RENEWAL_SECONDS=5 cargo build-sbf --tools-version v1.51 --sbf-out-dir .local/clock-5 -- --features local-clock
```

Never deploy a `local-clock` artifact to a public cluster. It cannot run there in any case: the feature build declares its own program id, `CnA1TVJUnVLzh5FgWwNcNcdT6MdiTRKGgkudHihUHVun`, and Anchor refuses to execute a program at any address other than its declared id, so the artifact is inert at the public address (SEC-11 in `docs/security/contract-findings.md`). No keypair for the local id exists; a local validator loads the program at genesis by address. The default build has no accelerated clock and declares the public id. The interval must be 5–86400 seconds. No program keypair, authority key, treasury secret or deployment credentials are included.

## Security

Findings from security review are tracked in [`docs/security/contract-findings.md`](docs/security/contract-findings.md), in priority order with a status and the test that pins each one; [`docs/security/README.md`](docs/security/README.md) says how to keep it current. `tests/calendar_properties.rs` checks the fee split and the calendar arithmetic against an independent implementation. Behavioural assertions against real transactions live in the app repository's validator suite.

## Verified build

Every release tag (`devnet-*`, `mainnet-*`) runs the `Verified build` workflow: a deterministic `solana-verify build` in the pinned `solanafoundation/solana-verifiable-build:2.1.18` container, compared with the bytes on the cluster. The job summary shows two hashes for the same binary: the plain SHA-256 of the `.so` file, which the Lumi application records and checks in the browser, and solana-verify's hash (the file with trailing zero bytes removed), which Solana Explorer and Solscan show. The Lumi Contracts page (`/contracts`) displays both next to the deployed slot, the deploy transaction and the commit.

To reproduce locally with Docker running:

```sh
cargo install solana-verify --version 0.5.1 --locked
solana-verify build --base-image solanafoundation/solana-verifiable-build:2.1.18 --library-name lumi
solana-verify get-executable-hash target/deploy/lumi.so
solana-verify get-program-hash -u https://api.devnet.solana.com GkZ9HQvNe1m1KDPA3D9HtFWNdkMe2baaed2fKH8w4FUv
```

Publishing the verification on chain, so explorers show the program as verified, is a signature by the upgrade authority and is done by a person: `solana-verify verify-from-repo -u <cluster url> --program-id GkZ9HQvNe1m1KDPA3D9HtFWNdkMe2baaed2fKH8w4FUv https://github.com/lumi-fans/solana-contracts --commit-hash <tag commit> --library-name lumi -k <upgrade authority keypair>`.

The binary embeds a `security.txt` section (contact, policy, source) that explorers render; the policy is [SECURITY.md](SECURITY.md).

## Scope

This is source publication, not an audit or a mainnet launch. There is no custody or refund instruction. Renewals require capped wallet consent, expire after their retry window, and can be stopped by cancelling membership or revoking the token allowance. The program is published under the Business Source License 1.1 (`LICENSE`): anyone may read, build, test and verify it, and use it on test networks; production use is limited to Lumi's own deployment until the change date, 18 September 2030, when it becomes MIT.

## GitHub Actions

This repository is public. `Contract checks` runs on pushes, pull requests and manual dispatch. It checks formatting, Clippy and Rust tests for both normal calendar-month and accelerated local-test builds, then separately compiles the normal Solana SBF program. It uploads only the normal `.so`, never a keypair. There is no deployment workflow.

The root `action.yml` allows the private app repository to obtain contract source through GitHub's organisation-scoped private-action sharing. GitHub supplies a temporary read-only download token; no personal token or deploy key is stored in the app. The app must pin the action to the exact full SHA of its `programs/lumi` gitlink. Mismatched revisions fail before compilation. When updating that gitlink, update `.github/actions/contracts/action.yml` in the app in the same commit.

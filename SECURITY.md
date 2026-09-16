# Security policy

This is source publication, not a bug bounty and not an audit. There is no reward programme yet; if one is created it will be announced here first.

## Reporting a vulnerability

Report suspected vulnerabilities in the program privately to **hello@lumi-fans.com** with the subject line `security`. Do not open a public issue for anything that could let funds move without the owner's signature, or that weakens a cap, a cancellation or a pause.

Include the commit or the deployed program hash you looked at, the instruction and accounts involved, and how to reproduce. We acknowledge within three working days and say what we intend to do within ten.

## Scope

- The program in this repository (`lumi_support`) as deployed on the cluster named on the Lumi Contracts page, which shows the deployed hash and the commit it was built from.
- Findings are tracked in priority order in `docs/security/contract-findings.md` with a status and the test that pins each one.

Out of scope: the disposable `local-clock` build, test deployments that were never linked from the Contracts page, and third-party programs the contract calls (the SPL Token program).

## Safe harbour

Good-faith research against the devnet deployment is welcome. Do not test against mainnet-beta once it exists, do not attempt to access other people's accounts or data, and do not run denial-of-service tests against our RPC or web infrastructure.

The same contact and policy are embedded in the program binary as a `security.txt` section, readable in Solana explorers.

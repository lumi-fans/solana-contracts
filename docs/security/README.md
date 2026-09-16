# Security

Living security record for the `lumi_support` program. The threat model lives in the app repository (`docs/threat-model/phase1.md`) and says what we intend to defend; this directory says what a review actually found and what state each finding is in.

| File | What it holds |
| --- | --- |
| [`contract-findings.md`](contract-findings.md) | The findings register. One row per finding, with severity, status and the test that pins it. |

## How to keep it live

1. **Every finding has an ID (`SEC-n`), a status and a test.** Statuses are `Open`, `Fixed (commit)`, `Accepted (who, why)` or `Superseded`. A finding without a pinning test is marked `untested` in the test column and is a to-do.
2. **Fix the code, then flip the test, then flip the status.** The tests assert the *current* behaviour, including the bad behaviour, so a fix makes a named test fail. Update that test to assert the fixed behaviour in the same commit as the fix, and change the row's status.
3. **Re-walk the register whenever `src/` changes.** Any commit that touches `src/` adds a line to the review log at the bottom of `contract-findings.md`: date, commit, which rows were re-checked. A scope change may mark rows `Superseded`.
4. **New findings go in priority order**, not append order. Renumber nothing; insert the row where its severity puts it.
5. **Severity scale.** *Critical*: loss of fan or creator funds without their signature. *High*: loss requiring a privileged key, or a fan-facing money error the fan cannot avoid. *Medium*: wrong amount or wrong timing of a payment the fan did authorise, or a governance gap that blocks a safe real-money launch. *Low*: hardening, a control that does less than documented, or an operational trap. *Informational*: worth knowing, no action required.

## Where the pinning tests are

- `tests/calendar_properties.rs` in this repository: fee split and calendar properties, run by `cargo test --locked`.
- `tests/protocol/src/security.test.ts` and `tests/protocol/src/renewals.test.ts` in the app repository: real transactions on a fixture-loaded local validator, run by `anchor test` there.

## Reporting

This is source publication, not a bug bounty. Report a suspected vulnerability privately to the maintainers listed in the app repository before opening a public issue.

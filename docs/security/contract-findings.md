# Contract findings register

Scope: this program (`lumi-fans/solana-contracts`, commit `b05d647`, reviewed 16 September 2026 against app commit `b13809e`) and the registrar co-signing path in the app repository's `apps/api/src/registration.ts`. Paths under `apps/`, `packages/`, `scripts/`, `tests/protocol/`, `docs/runbooks/` and the `Q<n>` founder questions refer to the private app repository, which pins this one as the `programs/lumi` submodule; `src/` and `tests/*.rs` paths are in this repository. Method: line-by-line read of every instruction and account constraint, a differential test of the calendar against an independent implementation, and real SBF transactions on a fixture-loaded validator for every behavioural claim below.

Nothing found lets a third party move a fan's or creator's USDC without that person's signature. The findings are about what privileged keys can do, what the program promises versus what it checks, and how monthly timing treats a fan. All fifteen were worked on 16 September 2026, one commit per finding: twelve fixed in code, SEC-1 and SEC-9 mitigated by process with a founder question each (Q22, Q23), SEC-13 accepted. A change to monthly-only periods is in progress in the app; rows note where that changes the picture.

A second pass on 16 September 2026 ran an adversarial sweep (`tests/protocol/src/adversarial.test.ts` in the app repository): arithmetic overflow, stripped signatures, account type cosplay, token-program substitution, random and shuffled instruction data, a frozen treasury, forged timestamps and consent-state edge cases. It found no way to move money without the payer's consent and added SEC-16 and SEC-17, both about renewal consent state; fixing SEC-17 surfaced SEC-18, a build-time hazard. All three were fixed the same day.

## Register

| ID | Severity | Title | Status | Pinned by |
| --- | --- | --- | --- | --- |
| SEC-1 | High (before real money) | Upgrade authority is one hot key and every renewal allowance is a standing target | Mitigated in process: app `docs/runbooks/upgrade-authority.md`, Q22; the hand-over itself needs a named human | untested (process) |
| SEC-2 | Medium | `initialize` is first-come; no `set_admin` or `set_treasury` | Fixed (contracts `set_treasury`/`propose_admin`/`accept_admin` + upgrade-authority gate; app commit pending) | `security.test.ts` SEC-2 (three cases) |
| SEC-3 | Medium | Monthly periods anchor to the join day, so a late payment can buy one day | Fixed (grace window keeps the anniversary; a later payment re-anchors) | `security.test.ts` SEC-3 (lapsed and in-grace cases); `calendar_properties.rs` calendar_alone_would_shorten… |
| SEC-4 | Low | Rotation cooling does not defend against creator-key compromise, and the promised notice does not exist | Fixed (owner re-checked on every payment; request indexed, emailed and shown to admin; threat model reworded) | `security.test.ts` SEC-4; app `events.test.ts`, `indexer.test.ts`, `notices.test.ts` |
| SEC-5 | Low | Self-gift and self-membership record full income while only the fee moves | Fixed (`SelfSupport` on gifts, charges and renewals) | `security.test.ts` SEC-5 |
| SEC-6 | Low | Creator pause does not stop a rotation being requested or applied | Fixed (`CreatorPaused` on request and apply) | `security.test.ts` SEC-6 |
| SEC-7 | Low | One delegate per USDC account makes renewal consents fungible across plans; no on-chain revoke | Fixed (`revoke_renewal` per plan; wallet-level revoke documented as all-or-nothing) | `security.test.ts` SEC-7 |
| SEC-8 | Low | The registrar hot key can undo an admin's creator pause | Fixed (registrar may only set `paused = true`) | `security.test.ts` SEC-8 |
| SEC-9 | Low | A pause, a closed plan or a benefits update longer than 72 hours silently voids every live mandate | Documented (runbooks, creator guide, dashboard note); skip-forward deferred to Q23 | `renewals.test.ts` expired and changed-terms cases |
| SEC-10 | Low | Membership records the actual charge time while the mandate advances from schedule; re-consent after a late renewal can skip a month | Fixed (`last_charged_at` records the scheduled period start) | `security.test.ts` SEC-10; `calendar_properties.rs` late_renewal_across… |
| SEC-11 | Low | The `local-clock` artifact is only distinguishable from the real one by hash | Fixed (feature build declares its own program id) | `tests/program_id.rs` under both builds; app `support-flow.test.ts` SEC-11 |
| SEC-12 | Informational | Missing events and unconsumed events leave the indexer blind to plan closure, registrar change and pending rotations | Fixed (three new events; every event now indexed) | app `events.test.ts`, `indexer.test.ts` SEC-12 |
| SEC-13 | Informational | Program accounts are never closable; member rent is locked forever | Accepted for v0 | n/a |
| SEC-14 | Informational | Registrar co-signature would replay across clusters if the key were reused | Documented rule: one registrar key per cluster (`rotate-registrar.md`) | n/a |
| SEC-15 | Informational | A Token-2022 mint with transfer fees would make events overstate what arrived | Fixed (`initialize` accepts only a classic SPL Token mint) | `security.test.ts` SEC-2/SEC-15 |
| SEC-16 | Low | A revoked mandate pins its source account, so a member cannot re-consent from another USDC account | Fixed (a mandate with nothing remaining accepts a new source) | `adversarial.test.ts` SEC-16 |
| SEC-17 | Low | A manual payment inside the grace window strands a live mandate while its allowance stays delegated | Fixed (optional mandate account on `charge_membership_period`; keeper stops on a mismatch) | `adversarial.test.ts` SEC-17 (two cases); app `support-flow.test.ts` SEC-17 |
| SEC-18 | Informational | Adding an account to `ChargeMembershipPeriod` pushed its validation frame past the SBF stack limit; the compiler only warns | Fixed (accounts boxed; the build fails on the warning) | `scripts/anchor-build.sh`; `adversarial.test.ts` manual-charge cases |
| SEC-19 | Medium | SOL treasury recipient depends on the configured USDC account's owner | Constrained: exact reference address, classic-token owner, USDC mint and canonical WSOL destination | app `sol.test.ts` recipient/proof rejection; builder and keeper account-order tests |
| SEC-20 | Informational | Anyone can initialise an active SOL plan for a registered creator | Intentional for SOL support availability; fixed reserved index and terms, pause checked, closed plans never reactivated | app `sol.test.ts` closed-plan and initialization cases; app `docs/sol-payments.md` |
| SEC-21 | Medium | A renewal must not switch between USDC and SOL | Constrained: reserved index fixes native mint; all other plans use global USDC mint; source and recipient checks still apply | app `sol.test.ts` wrong-mint cases and client mismatched-mint regression |
| SEC-22 | Medium | Unwrapping a recipient's WSOL closes the account and stops renewals | App keeper recreates both canonical recipient accounts atomically before charging; keeper pays rent, subject to the existing production budget gate | app `sol.test.ts` closed-recipient recovery, keeper Worker tests, Compose E2E |

## Details

### SEC-1 · Upgrade authority is one hot key and every renewal allowance is a standing target

**Where.** Program deployment; `authorize_renewal` (`lib.rs:385-397`), `charge_renewal` (`lib.rs:447-470`).

**What.** Each consenting member approves the program's `renewal-delegate` PDA for up to twelve payments per plan, and that allowance stays on the member's USDC account until the member revokes it in a wallet. Today the program can only spend it through `charge_renewal`. Any program upgrade can spend it anywhere in one transaction. The upgrade authority and the admin are the dev deployer key. The sum of all outstanding allowances is therefore controlled by one hot key, which is a different risk profile from the "program never holds a balance" statement in the module doc.

**Impact.** A leaked deployer key, or a coerced deploy, drains every member's authorised allowance at once. Bounded per member by twelve times the plan price per plan, unbounded across members.

**Fix.** Before any real money: move the upgrade authority to a multisig with a timelock, or freeze it and plan migrations by redeploying. Say in fan-facing renewal copy that the allowance is held by the program and how it is bounded (founder decision, `MESSAGING.md`). Consider a program-side cap on `payments` lower than twelve for the first months.

**Done (16 September 2026).** Runbook app `docs/runbooks/upgrade-authority.md` with the ceremony and the rule that no renewal mandate may exist on a cluster whose upgrade authority is a single key; `PRODUCTION_SETUP.md` ties the existing Squads plus 72-hour timelock plan to renewal allowances; Q22 records the default. The hand-over on any public cluster is a human ceremony and stays open until it is recorded in `BUILD_STATUS.md`.

**Monthly-only change.** Unaffected.

### SEC-2 · `initialize` is first-come; no `set_admin` or `set_treasury`

**Where.** `Initialize` (`lib.rs:728-744`), `initialize` (`lib.rs:83-92`), the comment at `lib.rs:81-82`.

**What.** The only requirement to call `initialize` is to be first. Whoever does becomes admin with their own treasury and registrar; the `global` PDA can never be created again, so recovery means a new program ID. The comment says the treasury "can only be changed by the admin", but no instruction changes it, and none changes the admin. If the treasury USDC account is closed or frozen (Circle can freeze USDC accounts), every gift and charge fails until a program upgrade. The mainnet plan of a Squads admin only works if the multisig itself signs `initialize`.

**Impact.** Front-run on a fresh mainnet deploy costs a redeploy. No treasury or admin rotation without an upgrade.

**Fix.** Constrain `admin` in `Initialize` to the program's upgrade authority by passing the `ProgramData` account and checking `upgrade_authority_address == Some(admin.key())`, or deploy and initialise in one transaction. Add `set_treasury` (admin, mint check, emits an event) and a two-step `set_admin` (propose, accept).

**Done (16 September 2026).** `Initialize` now takes the program and its `ProgramData` account and refuses any admin that is not the upgrade authority. `set_treasury` (admin only, mint checked, emits `TreasuryChanged`) and a two-step hand-over, `propose_admin` → `accept_admin` with `cancel_admin_transfer`, kept in a separate `admin-transfer` PDA so the deployed `GlobalConfig` layout is unchanged; `accept_admin` emits `AdminChanged`. `scripts/admin/protocol.ts` gained `set-treasury`, `propose-admin`, `cancel-admin-transfer` and `accept-admin`; `scripts/local/stack.ts` loads the program with `--upgradeable-program` so the local admin can initialise. Runbook: app `docs/runbooks/rotate-admin-or-treasury.md`.

**Pinned by.** `security.test.ts` "SEC-2": an unrelated key is refused at `initialize` and the upgrade authority succeeds; the treasury can be moved only by the admin and only to an account of the configured mint; the admin hand-over needs the proposed key's signature, cannot be accepted after cancellation, and closes the proposal.

### SEC-3 · Monthly periods anchor to the join day, so a late payment can buy one day

**Where.** `charge_membership_period` (`lib.rs:270-282`), `authorize_renewal` (`lib.rs:348-350`), `calendar.rs`.

**What.** For a monthly plan the next chargeable time is `next_month(last_charged_at, anchor_day)` where the anchor is the day-of-month of `joined_at`. That is right when charges land on time. When a member pays late, the next due date is still the anchor day of the following month, however soon that is. A member with anchor day 1 who pays on 31 January is chargeable again on 1 February. If they authorise renewal in the same session, `authorize_renewal` schedules the first automatic charge for that same date, so the keeper charges a full month's price again the next day. The doc comment at `lib.rs:50-52` says a period is "measured in seconds from the previous charge", which the monthly code does not do.

**Impact.** A fan pays a full month's price for anything from one day to a month, decided by the calendar rather than by them. Not theft, but it is a wrong-timing charge the fan did not knowingly authorise, and it lands in a real-money launch review.

**Fix.** Pick one: (a) on a manual charge that lands after the previous due date, re-anchor the membership (`joined_at = now`, or store `anchor_day` on `Membership` and reset it), so a paid month is always a calendar month from payment; or (b) keep the anchor only inside a grace window matching the mandate's 72 hours, and re-anchor beyond it. Either way store `next_due_at` on `Membership` (see SEC-10) and have `authorize_renewal` use it. Correct the doc comment.

**Monthly-only change.** Becomes the only path. Fix it in that change.

**Done (16 September 2026).** `last_charged_at` now means the start of the paid period. A monthly payment inside `GRACE_SECONDS` (72 hours, the same window renewals use) of its due date records the due date as the period start, so the anniversary and time-of-day do not drift. A payment later than that re-anchors: `joined_at` and `last_charged_at` both become the payment time, so the paid month is a full calendar month from payment and any mandate authorised afterwards is first charged a month later. Fixed-second plans are unchanged (their period already runs from the previous charge). The `30 * 86400` literal became `MONTHLY_PERIOD_SECONDS` and the doc comment now describes both kinds of period. Consequence for the app: "Member since" on the creator page shows the start of the current unbroken run, since the web derives the anchor from `joined_at` on chain.

**Pinned by.** `security.test.ts` "SEC-3": a member two months behind pays today, their `joined_at` moves to today and a renewal authorised next is due a full month later; a member 36 hours past due keeps their join date and their period starts at the due date. `calendar_properties.rs` `calendar_alone_would_shorten_a_late_payers_next_period_to_one_day` keeps the calendar fact that motivated the change.

### SEC-4 · Rotation cooling does not defend against creator-key compromise, and the promised notice does not exist

**Where.** `RegisterCreator` (`lib.rs:770`), `Gift` (`lib.rs:800`), `ChargeMembershipPeriod` (`lib.rs:885`), threat model "Payment-address rotation".

**What.** The payment account must be owned by the creator key at registration, and afterwards only its address is checked. Whoever holds the creator key therefore already controls the destination: they can sweep what arrives, or change the token account's owner with one `SetAuthority` and every later gift still lands there. The 48-hour cooling period only governs changing the *address*, which a thief never needs. The threat model says the cooling period "and notice give the creator time to pause their page"; no Worker consumes `PaymentAccountRotationRequested`, so there is no notice.

**Impact.** The documented control is weaker than documented. Real protection against key theft is the creator pause, which needs a person to notice.

**Fix.** Reword the threat model row. Implement the notice (index `PaymentAccountRotationRequested`, email the creator). Optional hardening: check `creator_payment_account.owner == support_config.creator` on every gift and charge so a phished `SetAuthority` fails payments loudly instead of redirecting them.

**Done (16 September 2026).** Every gift, membership charge and renewal now requires `creator_payment_account.owner == support_config.creator`, so a payment account whose owner changed after registration is refused with `PaymentAccountNotOwnedByCreator` instead of being paid. The app indexes `PaymentAccountRotationRequested` as a `pending` row in `creator_payment_addresses` (retired when the rotation is applied), lists it in the admin creators table, and emails the creator's account address once, when the request is finalized, through the chain indexer's Email Sending binding (`apps/chain-indexer/src/notices.ts`, `NOTICE_EMAIL_FROM`). The threat model rows now say what the cooling period does and does not do.

**Pinned by.** `security.test.ts` "SEC-4": after the creator moves ownership of the payment account to another key a gift is refused and no money moves; ownership restored, payments resume. App: `events.test.ts` decodes the request, `indexer.test.ts` records it once and flags the notice once, `notices.test.ts` covers the email body.

### SEC-5 · Self-gift and self-membership record full income while only the fee moves

**Where.** `Gift` (`lib.rs:786-805`), `ChargeMembershipPeriod` (`lib.rs:856-891`), `pay` (`lib.rs:563-604`).

**What.** Nothing stops `fan == creator` with the payment account as both source and destination. SPL Token treats the 99.9% leg as a self-transfer and moves nothing; the 0.1% leg pays the treasury; `SupportReceived` reports the full `creator_amount`. The indexer writes the statement from the event.

**Impact.** A creator can put any income figure on their own statement for 0.1% of it, without a second wallet or the round trip. Same effect for `MembershipCharged` with `member == creator`.

**Fix.** `constraint = fan.key() != support_config.creator @ SelfSupport` on `Gift`, the same for `member` on membership charges and renewals. One line each.

**Done (16 September 2026).** `Gift`, `ChargeMembershipPeriod`, `AuthorizeRenewal` and `ChargeRenewal` refuse a signer (or, for renewals, a membership member) equal to `support_config.creator` with `SelfSupport`. No event is emitted, so nothing reaches the statement.

**Pinned by.** `security.test.ts` "SEC-5": a creator's self-gift and self-membership charge are both refused and neither balance moves.

### SEC-6 · Creator pause does not stop a rotation being requested or applied

**Where.** `request_payment_account_rotation` (`lib.rs:506-522`), `apply_payment_account_rotation` (`lib.rs:535-557`).

**What.** Both check only the global pause. The runbook remedy for a suspected creator compromise is to pause that creator; the pending rotation still applies and the thief's account is in place when the creator is unpaused.

**Fix.** `require!(!config.paused, SupportError::CreatorPaused)` in both. Cancelling stays allowed while paused.

**Done (16 September 2026).** `request_payment_account_rotation` and `apply_payment_account_rotation` require `!support_config.paused`. Cancelling stays allowed while paused. The pause-creator runbook now says the on-chain pause is what freezes a pending rotation.

**Pinned by.** `security.test.ts` "SEC-6": while paused, neither the creator's request nor anyone's apply changes the payment account; once the admin unpauses, the cooled-down rotation applies.

### SEC-7 · One delegate per USDC account makes renewal consents fungible across plans; no on-chain revoke

**Where.** `AuthorizeRenewal.delegate` (`lib.rs:690`), allowance arithmetic (`lib.rs:360-384`), error text at `lib.rs:1058`.

**What.** The delegate PDA is seeded by the source token account, so every plan a member renews from the same USDC account shares one allowance. `authorize_renewal` adds the new consent to whatever is already delegated, minus that mandate's own share. Consequences: revoking in a wallet, which the program's own error message tells members to do, stops all plans at once and is undone by the next `authorize_renewal` on any plan; a live mandate the member thought revoked is then charged with the new consent and the newly consented plan is left unpayable. There is no instruction to withdraw a mandate short of `cancel_membership`, and no instruction reduces the allowance.

**Impact.** A member cannot express "stop renewing plan A, keep plan B" on chain. Money charged is always for a mandate the member did sign and did not cancel, so this is consent hygiene, not theft.

**Fix.** Add `revoke_renewal` (member signs; sets `remaining = 0` and re-approves the delegate for the allowance minus this mandate's share). Let `cancel_membership` take the mandate as an optional account and do the same. Have the UI's "stop auto-renew" call it. Document that wallet-level revoke is all-or-nothing.

**Done (16 September 2026).** New `revoke_renewal` (member signs): zeroes the mandate's `remaining`, which alone guarantees no further automatic charge for that plan, and re-approves the shared delegate for the current allowance minus that mandate's remaining share, clearing the delegate when nothing is left. The membership is untouched. The web page's renewal card has a "Stop automatic renewal" button (`apps/web/src/components/renewal.tsx`, `revokeRenewalInstruction`), and the fan guide says a wallet-level revoke stops every plan at once and that a later consent for any plan restores the allowance for every plan not stopped or cancelled. Known limit of a shared allowance: if a member revoked in their wallet earlier, a per-plan revoke may leave another plan under-allowanced; the page shows that plan as needing fresh permission.

**Pinned by.** `security.test.ts` "SEC-7": with two consents on one USDC account, stopping plan 0 leaves plan 1's share and mandate intact, the keeper can no longer charge plan 0, the membership is not cancelled, and stopping the last plan clears the delegate.

### SEC-8 · The registrar hot key can undo an admin's creator pause

**Where.** `set_creator_paused` (`lib.rs:129-142`).

**What.** Admin or registrar may set either value. The registrar is the one hot key Lumi holds; if it leaks, the holder can pause every creator and can also unpause a creator the admin paused for cause.

**Fix.** Registrar may set `paused = true` only; unpausing requires admin.

**Done (16 September 2026).** `set_creator_paused` accepts the registrar only when `paused` is true; unpausing needs the admin. `docs/runbooks/rotate-registrar.md` is unaffected: a leaked registrar can still pause creators (a DoS the admin reverses by unpausing and rotating the key), but cannot reopen a creator the admin closed.

**Pinned by.** `security.test.ts` "SEC-8": the registrar pauses a creator, cannot unpause, an unrelated key cannot either, the admin can.

### SEC-9 · A pause, a closed plan or a benefits update longer than 72 hours silently voids every live mandate

**Where.** `charge_renewal` (`lib.rs:414-444`), `set_plan_benefits_hash` (`lib.rs:229-239`).

**What.** A mandate must be charged within 72 hours of `next_charge_at`, and the schedule never advances when a charge is skipped. A global pause, a creator pause or a closed plan that spans a member's window expires that mandate for good; the member must pay manually and re-consent, and their allowance stays approved. A benefits update changes `plan.benefits_hash`, so every live mandate on the plan fails the terms check from that moment and expires 72 hours after its next due date. None of this is in the pause runbook or the creator dashboard.

**Fix.** Document in `docs/runbooks/pause-protocol.md` and the creator guide. Consider letting an expired mandate skip forward without charging (`next_charge_at = next_month(...)` when the window has passed, "no catch-up debt" preserved) so a pause does not force thousands of re-consents.

**Done (16 September 2026).** `docs/runbooks/pause-protocol.md` and `pause-creator.md` state what a pause longer than 72 hours does to mandates and recommend creator-level pauses; the creator guide and the dashboard's benefits editor say that a benefits edit or a closed plan stops every member's automatic renewal for that plan and that members re-consent on their next visit. Skipping the missed month and resuming automatically was not implemented: it would charge members again after a gap they did not choose, which is a founder decision (Q23, default keeps expiry and fresh consent).

**Pinned by.** `renewals.test.ts` "expired" (past the window) and "changed-terms" (benefits hash differs) cases; the program behaviour is unchanged.

### SEC-10 · Membership records the actual charge time while the mandate advances from schedule

**Where.** `charge_renewal` (`lib.rs:472-473`), `authorize_renewal` (`lib.rs:349`).

**What.** A renewal charged late inside the window sets `membership.last_charged_at = now` but `mandate.next_charge_at = next_month(scheduled)`. Re-consenting later computes the due date from `last_charged_at`. When the late charge crossed a month boundary the two differ by a month: scheduled 30 January, charged 1 February; the mandate says 28 February, a fresh consent says 30 March.

**Impact.** The member gets a free month on re-consent; the creator loses it; statements and dashboards disagree about the next due date.

**Fix.** Keep one `next_due_at` on `Membership`, advance it from schedule in both paths, and derive everything else from it.

**Done (16 September 2026).** `charge_renewal` now writes the mandate's scheduled `next_charge_at` into `membership.last_charged_at` before advancing the schedule, so the membership and the mandate agree on when the paid period started, whatever hour inside the 72-hour window the keeper landed. Together with SEC-3, `last_charged_at` always means "start of the paid period" on every path. The event still carries the wall-clock time.

**Pinned by.** `security.test.ts` "SEC-10": a renewal charged two days late records the scheduled time, not the charge time. `calendar_properties.rs` `late_renewal_across_a_month_boundary_diverges_from_the_schedule` keeps the calendar fact that motivated it.

### SEC-11 · The `local-clock` artifact is only distinguishable from the real one by hash

**Where.** `lib.rs:24-37`, `scripts/local/stack.ts`.

**What.** A build with the `local-clock` feature accepts renewal periods of 5 seconds to a day. It has the same program ID and the same IDL as the real build; the only thing keeping it off devnet or mainnet is a separate output directory and process.

**Fix.** Under `cfg(feature = "local-clock")` use a different `declare_id!`. Anchor refuses to run a program at an address other than its declared ID, so the accelerated artifact cannot execute at the public program address even if someone uploads it.

**Done (16 September 2026).** The `local-clock` build declares `CnA1TVJUnVLzh5FgWwNcNcdT6MdiTRKGgkudHihUHVun`; the standard build keeps the public id. No keypair for the local id exists anywhere: the local validator loads the program at genesis by address. The app's local stack deploys and initialises at that id (`scripts/local/stack.ts`), the chain client exports it as `LUMI_SUPPORT_LOCAL_CLOCK_PROGRAM_ID`, and the browser's address check accepts it only when the cluster is `localnet` (`apps/web/src/lib/support-flow.ts`).

**Pinned by.** `tests/program_id.rs`: the standard build declares the public id and the feature build declares the local one (`LUMI_LOCAL_RENEWAL_SECONDS=5 cargo test --features local-clock`). App `support-flow.test.ts` "SEC-11": the local id is refused off localnet.

### SEC-12 · Missing events and unconsumed events leave the indexer blind

**Where.** `set_plan_active` (`lib.rs:222-225`), `set_registrar` (`lib.rs:100-103`), `cancel_payment_account_rotation` (`lib.rs:526-531`), `apps/chain-indexer/src/events.ts`.

**What.** The three instructions emit nothing. `PaymentAccountRotationRequested`, `CreatorPauseChanged` and `ProtocolPauseChanged` are emitted but not parsed by the indexer. The database can show a closed plan as open, admin tooling cannot see a registrar change on the statement, and nobody is told a rotation is pending (SEC-4).

**Fix.** Emit `PlanActiveChanged`, `RegistrarChanged`, `PaymentAccountRotationCancelled`; consume the rotation and pause events.

**Done (16 September 2026).** `set_plan_active` emits `PlanActiveChanged`, `set_registrar` emits `RegistrarChanged`, `cancel_payment_account_rotation` emits `PaymentAccountRotationCancelled`. The indexer now parses every event the program emits: plan closure updates `membership_plans.active`, a cancelled rotation retires the pending payment address, and the pause, registrar, treasury, admin and renewal-revocation events are stored in `chain_events` so an operator can see every governance change on the record.

**Pinned by.** App `events.test.ts` decodes the new events; `indexer.test.ts` "SEC-12" closes a plan and retires a cancelled rotation from finalized events and records both.

### SEC-13 · Program accounts are never closable

`Membership`, `RenewalMandate`, `MembershipPlan` and `SupportConfig` have no close instruction. A member's rent (about 0.0018 SOL per membership on the local run) is locked forever. Accepted for v0: closability would reopen the `init_if_needed` re-initialisation surface that the current design avoids. Revisit with a proper close path that also zeroes the mandate.

### SEC-14 · Registrar co-signature would replay across clusters if the key were reused

The program ID, seeds and therefore every PDA are identical on devnet and mainnet, and `cosignRegistration` does not check the blockhash's cluster. A `register_creator` transaction co-signed for devnet is valid on mainnet if `global.registrar` is the same key there. Rule: one registrar key per environment, never reused; written into the app's `docs/runbooks/rotate-registrar.md` on 16 September 2026. The co-signing check is otherwise tight: one instruction, legacy format only, exact program and discriminator, exact accounts and flags, creator pays and has already signed.

### SEC-15 · A Token-2022 mint with transfer fees would make events overstate what arrived

The program accepts either token program for accounts. If the USDC mint were ever a Token-2022 mint with the transfer-fee extension, `creator_amount` in events would exceed what the creator receives. USDC is classic SPL Token and the mint is fixed at `initialize`.

**Done (16 September 2026).** `initialize` refuses a mint not owned by the classic SPL Token program with `UnsupportedTokenProgram`, so a transfer-fee mint cannot be configured by mistake. Pinned by `security.test.ts` "SEC-2": a structurally valid Token-2022 mint is refused by the upgrade authority before the classic mint succeeds.

### SEC-16 · A revoked mandate pins its source account

**Where.** `authorize_renewal` (`lib.rs:461-473`), `revoke_renewal` (`lib.rs:510`).

**What.** `authorize_renewal` refuses a consent whose source token account differs from the one already stored on the mandate, so that a new consent cannot silently move a live mandate to another account. `revoke_renewal` zeroes `remaining` but leaves `source` in place. A member who stopped renewals and later wants to renew from a different USDC account is refused with `InvalidMandate` for that plan for ever; only the original account is accepted.

**Impact.** Consent hygiene, no money at risk. For most wallets the source is the associated token account, whose address is fixed per wallet, so a closed and re-created account has the same address and the check passes. A member who used a secondary token account, or a wallet that moved its USDC to a new account, cannot turn renewals back on for that plan without moving USDC back.

**Fix.** Allow the source to change when the mandate holds no remaining payments: `mandate.source == Pubkey::default() || mandate.source == source.key() || mandate.remaining == 0`. `previous` is already zero in that case, so the allowance arithmetic is unchanged. One line.

**Done (16 September 2026).** `authorize_renewal` accepts a different source when `mandate.remaining == 0`. A live mandate still cannot be moved.

**Pinned by.** `adversarial.test.ts` "SEC-16": with a revoked mandate on account A, consent from account B is accepted, B is delegated for the new consent, A is untouched, and a second consent from A against the now-live mandate is refused.

### SEC-17 · A manual payment inside the grace window strands a live mandate

**Where.** `charge_membership_period` (`lib.rs:325`), `charge_renewal` (`lib.rs:571`), app `apps/web/src/pages/creator-page.tsx` ("Pay the next period" is enabled from the due date), app `apps/chain-indexer/src/renewals.ts:340`.

**What.** `charge_renewal` requires `mandate.periods_paid == membership.periods_paid`, which correctly stops a manual payment and an automatic renewal from both landing in one window. The manual path does not take the mandate account, so it cannot advance it. If a member with a live mandate pays by hand between the due time and the keeper's run (the button is enabled from the due date, and the keeper lands some time inside the 72-hour window), the membership's counter moves one ahead of the mandate's and nothing ever brings them level again. The mandate keeps its `remaining` payments and the wallet keeps the full allowance delegated, but no later renewal can charge. The keeper throws "Membership and mandate periods differ" and the queue retries it with backoff until the dead-letter queue takes it; the renewal card shows the mandate as not active because its own check fails.

**Impact.** The member believes renewals are on and the membership lapses a month later; the delegated allowance for a mandate that can never be used stays on the wallet until the member re-consents (which resets it) or revokes. No overcharge is possible.

**Fix.** Either (a) make the mandate an optional account of `charge_membership_period` and, when present and live, treat the manual payment as that period's renewal (advance `next_charge_at`, decrement `remaining`, keep the counters level); or (b) keep the program as it is and in the app refuse the manual button while a live mandate is inside its window, and have the keeper stop the mandate with a member notice when it sees the mismatch instead of retrying. (a) is cleaner on chain; (b) needs no program change. Either way the keeper should stop, not retry, on a counter mismatch.

**Done (16 September 2026).** Option (a). `ChargeMembershipPeriod` takes the member's mandate as a trailing optional account (Anchor's program-id marker when absent, so a first payment and older clients are unaffected). When the mandate is present and live for the period being paid (monthly plan, counters level, payments remaining, terms unchanged, inside the 72-hour window) the manual payment counts as that period's renewal: the mandate's counter follows the membership and `next_charge_at` moves on a month. `remaining` is not consumed: the automatic payments the member consented to are still ahead of them, and the allowance already matches. A mandate that is exhausted, out of step, on changed terms or past its window is left alone (SEC-9 applies). The web flow looks the mandate up and passes it (`apps/web/src/lib/support-flow.ts`); `buildChargeMembershipPeriod` takes it as an option. The keeper now stops a job whose counters differ instead of retrying it into the dead-letter queue (`renewal_out_of_step`).

**Pinned by.** `adversarial.test.ts` "SEC-17": with the mandate passed, a payment an hour after the due date lands at the due date, the mandate's counter reaches the membership's, its next charge is a calendar month on and it keeps three payments; the keeper is refused with `PeriodNotElapsed` and the allowance is unchanged. Without the mandate (an older client) the payment is still accepted and the mandate is left out of step. App `support-flow.test.ts` "SEC-17": the browser passes the mandate when the account exists and the program id when it does not.

### SEC-18 · The charge instruction's account validation overran the SBF stack frame

**Where.** `ChargeMembershipPeriod` (`lib.rs`), `scripts/anchor-build.sh` in the app repository.

**What.** Adding the optional mandate account made Anchor's generated `try_accounts` for `ChargeMembershipPeriod` need 4,112 bytes of stack against a 4,096-byte frame. The SBF compiler prints "Stack offset of 4112 exceeded max offset of 4096 by 16 bytes ... may cause undefined behavior" and carries on. On the validator the effect was silent: every manual charge of an existing membership failed `Unauthorized` because `membership.member` was read from corrupted memory, while the account on chain was correct. Had the overrun landed a few bytes elsewhere it could have passed a check instead of failing one.

**Impact.** None deployed: the build that is on devnet has no such warning. The hazard is that any future account added to a large instruction can reintroduce it and nothing in CI would object.

**Fix.** Every account in `ChargeMembershipPeriod` is boxed, as `AuthorizeRenewal` and `ChargeRenewal` already were. `pnpm anchor:build` now runs `scripts/anchor-build.sh`, which fails the build on the compiler's stack-offset warning; CI's `pr-checks` uses that script.

**Pinned by.** The build guard, and every manual-charge case in `adversarial.test.ts` and `security.test.ts`, which failed under the overrun.

## What holds up

Recorded so the next reviewer does not redo it.

- **Fee arithmetic.** `split` floors the treasury share and gives the creator the residue; conservation and "never rounds toward the treasury" hold over fifty thousand random amounts up to the overflow bound (`calendar_properties.rs`). Every in-range amount produces a non-zero treasury share so both transfers always execute.
- **Calendar.** `next_month` agrees with an independent civil-date implementation on one hundred thousand random timestamps and anchors, keeps the anchor through short months (31 → 30 → 31), handles the 2100 non-leap year and rejects out-of-range input.
- **Account constraints.** Every PDA is derived from seeds plus a stored bump; `has_one` and `address =` checks bind creator, plan, membership, mandate, treasury and payment account to each other. The renewal delegate is a PDA seeded by the source account, so a foreign delegate is refused rather than overwritten.
- **No held balance.** The program owns no token account and has no instruction that moves USDC anywhere but the registered payment account and the fixed treasury.
- **Double-charge protection.** A mandate is valid only while `mandate.periods_paid == membership.periods_paid`, so a manual payment and an automatic renewal in the same window cannot both land; the 72-hour window forbids catch-up debt; `remaining` and the twelve-payment cap bound exposure.
- **Pause coverage.** The global pause blocks every money-moving and state-creating instruction and leaves cancellation open.
- **Adversarial sweep (16 September 2026, `adversarial.test.ts`, 19 cases on a fixture-loaded validator).** Gift amounts of `u64::MAX`, `u64::MAX / 10 + 1` and `2^63` fail with `MathOverflow` before any transfer; an in-range amount beyond the balance fails in the token program with nothing partially paid; landed gifts always split as floor(amount / 1000) to the treasury and the rest to the creator. A wallet that has already delegated `u64::MAX` to the renewal PDA is refused at `authorize_renewal` with `MathOverflow` (self-inflicted only). Plan prices and periods at both ends of their types, including `i64::MIN` and `i64::MAX`, and renewal counts of 0, 13 and `u16::MAX` are refused with the range errors. Forged `last_charged_at` near `i64::MAX` and a join date past the calendar's year-2200 bound fail with an error, not a panic. A gift whose fan is not marked as a signer, program accounts passed in the wrong slot (plan as config, config as global, another member's membership, a membership as a mandate, a plan under another creator's config), and any token program other than the classic one are all refused. A payment account closed and re-created for another mint at the registered address passes the program's address and owner checks and is stopped by the token program's mint check with nothing moved. Sixty rounds of random argument bytes across every instruction, forty rounds of shuffled gift accounts and every truncation of the gift data produced an error and never a panic or abort. Freezing the treasury reverts the whole gift, so the creator is never paid without the fee. Two renewal charges or two manual charges in one transaction fail as a whole with `PeriodNotElapsed`.
- **Co-signing.** `apps/api/src/registration.ts` accepts exactly one legacy `register_creator` instruction with fixed accounts and flags, the creator as fee payer with a verified signature, and signs nothing else.

## Review log

| Date | Program commit | Reviewer | What |
| --- | --- | --- | --- |
| 2026-09-16 | `b05d647` | Claude Fable 5.1 for Mark | Full read of every instruction; calendar differential test; validator assertions for SEC-2 to SEC-8; register created. Monthly-only period change was in progress in a separate process and is not reflected. |
| 2026-09-16 | `b05d647` → this commit | Claude Fable 5.1 for Mark | Fixes landed one commit per finding: SEC-2 (upgrade-authority gate, `set_treasury`, two-step admin), SEC-3 (grace window and re-anchoring), SEC-4 (payment-account owner check, rotation notice), SEC-5 (`SelfSupport`), SEC-6 (creator pause freezes rotations), SEC-7 (`revoke_renewal`), SEC-8 (registrar pause-only), SEC-10 (scheduled period start), SEC-11 (local-clock program id), SEC-12 (events), SEC-15 (classic mint only). SEC-1, SEC-9 and SEC-14 documented with runbooks and Q22/Q23. Every row re-checked against the new code; the app's validator suite runs 46 cases green. Deployed devnet still runs the pre-review program until the next release. |
| 2026-09-16 | `d0fe173` | Claude Fable 5.1 for Mark | Adversarial sweep at Mark's request (app `tests/protocol/src/adversarial.test.ts`, 19 cases): overflow, signatures, type cosplay, token-program substitution, random and shuffled instruction data, frozen treasury, forged timestamps, double charging, consent state. No fund-loss path. SEC-16 and SEC-17 added, both open with proposed fixes. |
| 2026-09-16 | `e5339d2` → this commit | Claude Fable 5.1 for Mark | SEC-16 fixed (revoked mandate accepts a new source), SEC-17 fixed (optional mandate account on `charge_membership_period`, web passes it, keeper stops on mismatch), SEC-18 found and fixed while doing so (boxed `ChargeMembershipPeriod`, build fails on the SBF stack-offset warning). Adversarial, security, renewals and Anchor-client suites pass on the rebuilt program. |
| 2026-09-16 | `cd3ddf1` → this commit | Claude Fable 5.1 for Mark | Merged the public `main` (verified-build workflow, embedded `security.txt`, `SECURITY.md`) into the SEC-2 to SEC-18 fix line. No instruction changed; the only `src/` addition is the `security_txt!` metadata block. `cargo test`, clippy and fmt re-run. |
| 2026-09-16 | `e95ec5d` → this commit | Claude Fable 5.1 for Mark | Renamed the crate, program module, IDL and artifact from `lumi_support` to `lumi` at Mark's request. No instruction, account, seed or discriminator changed (discriminators derive from instruction and account names); `cargo test`, clippy, fmt and `cargo build-sbf` re-run. |
| 2026-09-17 | `9dd4541` → this commit | Claude Fable 5.1 for Mark | Members choose what they pay: `MembershipPlan.price` is now the minimum per period, `Membership.amount` records the member's choice, `charge_membership_period(amount)` and `authorize_renewal(payments, amount, benefits_hash)` require `plan.price <= amount <= MAX_PLAN_PRICE`, and a mandate stays valid while its amount is at or above the plan minimum (`charge_membership_period` in-step check and `charge_renewal`). SEC-16/17 semantics unchanged: the mandate's own amount is what the keeper may take. Re-walked SEC-3, SEC-9, SEC-10, SEC-16, SEC-17 against the new checks; `cargo test`, clippy, fmt and `cargo build-sbf` re-run. IDL regenerated in the following commit. |

## SOL review — 18 September 2026

The 0.2.0 source adds a native-mint payment path without changing existing account layouts. These notes record the extra surface; they do not constitute an independent audit or mainnet approval.

- **Treasury proof (SEC-19):** the first remaining account must be `global.treasury_usdc_account`, owned by the classic token program and containing the configured USDC mint. Its token owner fixes the canonical WSOL treasury. The client pins the trailing account's position after all IDL accounts, including the optional mandate placeholder. A mutable API quote cannot choose another owner.
- **Plan creation (SEC-20):** any payer can fund the fixed SOL plan at `u32::MAX`. The creator need not sign a separate opt-in. It does not increment `plan_count`; repeat initialization preserves existing state, including closure. Creator and protocol pauses remain effective. The app documents this as the implementation of SOL one-off and monthly support, with public availability gated until upgrade.
- **Mint isolation (SEC-21):** authorization, manual charges and automatic charges derive the mint from the plan index. WSOL accounts do not permit debits from ordinary SOL. The browser validates the canonical SOL plan address, and the renewal builder rejects a SOL/USDC mismatch even when the amount is valid in both currencies.
- **Recipient closure (SEC-22):** unwrapping may close either recipient's WSOL account. The app keeper prepends idempotent creation of the creator and treasury accounts, using itself as payer. Creation and payment succeed or revert together; consent and source funds remain unchanged on failure. Deposits paid to create recipient accounts belong to their owners. Repeated closure can increase keeper costs; production sponsorship and limits remain gated by Q16, and authority requirements by Q22 in the application repository.

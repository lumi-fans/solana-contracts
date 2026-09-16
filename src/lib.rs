//! infx-support: the Phase 1 product.
//!
//! A fan sends a one-off gift or pays a membership period in USDC. In the same
//! instruction 99.9% goes to the creator's registered USDC account and 0.1% to the
//! INFx treasury. The program never holds a balance and has no instruction that
//! can move funds anywhere except those two accounts.
//!
//! Money is the creator's the moment it is paid. There is no escrow, no refund
//! path and no delivery check (ADR 0007).

#![allow(clippy::result_large_err)]
#![allow(deprecated)]

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    self, ApproveChecked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

mod calendar;
#[cfg(not(feature = "local-clock"))]
use calendar::next_month;

// Compiled only for the disposable local validator; never a runtime production flag.
#[cfg(feature = "local-clock")]
fn next_month(timestamp: i64, anchor: u8) -> Result<(i64, u8)> {
    let seconds: i64 = env!("LUMI_LOCAL_RENEWAL_SECONDS")
        .parse()
        .map_err(|_| error!(SupportError::InvalidMandate))?;
    require!((5..=86400).contains(&seconds), SupportError::InvalidMandate);
    let day = calendar::next_month(timestamp, anchor)?.1;
    Ok((
        timestamp
            .checked_add(seconds)
            .ok_or(SupportError::MathOverflow)?,
        day,
    ))
}

declare_id!("GkZ9HQvNe1m1KDPA3D9HtFWNdkMe2baaed2fKH8w4FUv");

/// 0.1% of every gift and membership charge. Changing this needs a human.
pub const FEE_BPS: u64 = 10;
pub const BPS_DENOMINATOR: u64 = 10_000;

/// DECISION(Q6): minimums and maximums in USDC base units (six decimals).
pub const MIN_GIFT: u64 = 1_000_000;
pub const MIN_PLAN_PRICE: u64 = 1_000_000;
pub const MAX_PLAN_PRICE: u64 = 500_000_000;

/// DECISION(Q7): a fixed-length membership period is measured in seconds from
/// the start of the previous period. A plan of exactly `MONTHLY_PERIOD_SECONDS`
/// bills by calendar month instead: the same day each month, clamped to the
/// end of shorter months. Bounded so a plan cannot be created with a period
/// that lets a creator charge every block or never.
pub const MIN_PERIOD_SECONDS: i64 = 24 * 60 * 60;
pub const MAX_PERIOD_SECONDS: i64 = 366 * 24 * 60 * 60;
pub const MONTHLY_PERIOD_SECONDS: i64 = 30 * 86400;

/// How long after a due date a charge still counts for that period. A manual
/// payment inside the window starts its month at the due date; a later one
/// starts a fresh month from the payment. Renewal mandates expire past it.
pub const GRACE_SECONDS: i64 = 3 * 86400;

/// Cooling period before a creator's payment-account rotation takes effect.
pub const ROTATION_COOLING_SECONDS: i64 = 48 * 60 * 60;

pub const GLOBAL_SEED: &[u8] = b"global";
pub const CREATOR_SEED: &[u8] = b"creator";
pub const PLAN_SEED: &[u8] = b"plan";
pub const MEMBERSHIP_SEED: &[u8] = b"membership";
pub const ADMIN_TRANSFER_SEED: &[u8] = b"admin-transfer";

/// The treasury share rounds down and the creator receives the residue, so
/// the two always sum exactly to the amount and rounding never favours INFx.
pub fn split(amount: u64) -> Result<(u64, u64)> {
    let treasury = amount
        .checked_mul(FEE_BPS)
        .ok_or(SupportError::MathOverflow)?
        / BPS_DENOMINATOR;
    let creator = amount
        .checked_sub(treasury)
        .ok_or(SupportError::MathOverflow)?;
    Ok((creator, treasury))
}

#[program]
pub mod infx_support {
    use super::*;

    /// One-time setup. Only the program's upgrade authority can call it, so a
    /// fresh deployment cannot be claimed by whoever reaches it first. The
    /// treasury and admin can later be changed by the admin (`set_treasury`,
    /// `propose_admin` then `accept_admin`), which on mainnet is the Squads
    /// multisig.
    pub fn initialize(ctx: Context<Initialize>, registrar: Pubkey) -> Result<()> {
        let global = &mut ctx.accounts.global;
        global.admin = ctx.accounts.admin.key();
        global.registrar = registrar;
        global.usdc_mint = ctx.accounts.usdc_mint.key();
        global.treasury_usdc_account = ctx.accounts.treasury_usdc_account.key();
        global.paused = false;
        global.bump = ctx.bumps.global;
        Ok(())
    }

    pub fn set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        ctx.accounts.global.paused = paused;
        emit!(ProtocolPauseChanged { paused });
        Ok(())
    }

    pub fn set_registrar(ctx: Context<AdminOnly>, registrar: Pubkey) -> Result<()> {
        ctx.accounts.global.registrar = registrar;
        Ok(())
    }

    /// Points the fee share at a different USDC account, for example after the
    /// current one is frozen or closed. Every later gift and charge pays it.
    pub fn set_treasury(ctx: Context<SetTreasury>) -> Result<()> {
        let global = &mut ctx.accounts.global;
        let previous = global.treasury_usdc_account;
        global.treasury_usdc_account = ctx.accounts.treasury_usdc_account.key();
        emit!(TreasuryChanged {
            previous_treasury_usdc_account: previous,
            treasury_usdc_account: global.treasury_usdc_account,
        });
        Ok(())
    }

    /// First half of an admin hand-over. Nothing changes until the proposed
    /// key signs `accept_admin`, so a typo cannot strand the protocol.
    pub fn propose_admin(ctx: Context<ProposeAdmin>, new_admin: Pubkey) -> Result<()> {
        require!(new_admin != Pubkey::default(), SupportError::Unauthorized);
        let transfer = &mut ctx.accounts.admin_transfer;
        transfer.pending_admin = new_admin;
        transfer.bump = ctx.bumps.admin_transfer;
        Ok(())
    }

    /// The current admin withdraws a proposal.
    pub fn cancel_admin_transfer(_ctx: Context<CancelAdminTransfer>) -> Result<()> {
        Ok(())
    }

    /// Second half: the proposed key takes over and the proposal is closed.
    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        let global = &mut ctx.accounts.global;
        let previous = global.admin;
        global.admin = ctx.accounts.new_admin.key();
        emit!(AdminChanged {
            previous_admin: previous,
            admin: global.admin,
        });
        Ok(())
    }

    /// Registers a verified creator. Both the INFx registrar and the creator
    /// sign: the registrar attests to verification, the creator attests to the
    /// payment account. Neither can do it alone.
    pub fn register_creator(ctx: Context<RegisterCreator>) -> Result<()> {
        let global = &ctx.accounts.global;
        require!(!global.paused, SupportError::ProtocolPaused);

        let config = &mut ctx.accounts.support_config;
        config.creator = ctx.accounts.creator.key();
        config.payment_account = ctx.accounts.payment_account.key();
        config.pending_payment_account = Pubkey::default();
        config.rotation_effective_at = 0;
        config.plan_count = 0;
        config.paused = false;
        config.bump = ctx.bumps.support_config;

        emit!(CreatorRegistered {
            creator: config.creator,
            payment_account: config.payment_account,
        });
        Ok(())
    }

    /// The registrar or the admin can pause a creator: no new gifts or charges.
    pub fn set_creator_paused(ctx: Context<SetCreatorPaused>, paused: bool) -> Result<()> {
        let signer = ctx.accounts.authority.key();
        let global = &ctx.accounts.global;
        require!(
            signer == global.admin || signer == global.registrar,
            SupportError::Unauthorized
        );
        ctx.accounts.support_config.paused = paused;
        emit!(CreatorPauseChanged {
            creator: ctx.accounts.support_config.creator,
            paused,
        });
        Ok(())
    }

    /// A one-off gift. 99.9% to the creator, 0.1% to the treasury, nothing else.
    pub fn gift(ctx: Context<Gift>, amount: u64) -> Result<()> {
        require!(!ctx.accounts.global.paused, SupportError::ProtocolPaused);
        require!(
            !ctx.accounts.support_config.paused,
            SupportError::CreatorPaused
        );
        require!(amount >= MIN_GIFT, SupportError::BelowMinimum);

        let (creator_amount, treasury_amount) = split(amount)?;
        pay(
            &ctx.accounts.token_program,
            &ctx.accounts.usdc_mint,
            &ctx.accounts.fan,
            &ctx.accounts.fan_usdc_account,
            &ctx.accounts.creator_payment_account,
            &ctx.accounts.treasury_usdc_account,
            creator_amount,
            treasury_amount,
        )?;

        emit!(SupportReceived {
            creator: ctx.accounts.support_config.creator,
            fan: ctx.accounts.fan.key(),
            amount,
            creator_amount,
            treasury_amount,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    /// A membership plan: a price per period and the hash of the benefits
    /// description shown to members. The hash is a record, not a contract:
    /// INFx does not enforce it (ADR 0007).
    pub fn create_membership_plan(
        ctx: Context<CreateMembershipPlan>,
        price: u64,
        period_seconds: i64,
        benefits_hash: [u8; 32],
    ) -> Result<()> {
        require!(!ctx.accounts.global.paused, SupportError::ProtocolPaused);
        require!(
            (MIN_PLAN_PRICE..=MAX_PLAN_PRICE).contains(&price),
            SupportError::PriceOutOfRange
        );
        require!(
            (MIN_PERIOD_SECONDS..=MAX_PERIOD_SECONDS).contains(&period_seconds),
            SupportError::PeriodOutOfRange
        );

        let config = &mut ctx.accounts.support_config;
        let plan = &mut ctx.accounts.plan;
        plan.creator = config.creator;
        plan.index = config.plan_count;
        plan.price = price;
        plan.period_seconds = period_seconds;
        plan.benefits_hash = benefits_hash;
        plan.active = true;
        plan.bump = ctx.bumps.plan;
        config.plan_count = config
            .plan_count
            .checked_add(1)
            .ok_or(SupportError::MathOverflow)?;

        emit!(PlanCreated {
            creator: plan.creator,
            plan: plan.key(),
            index: plan.index,
            price,
            period_seconds,
            benefits_hash,
        });
        Ok(())
    }

    /// Closing a plan stops new charges. Existing members keep what they paid
    /// for; nothing is refunded because nothing is held.
    pub fn set_plan_active(ctx: Context<CreatorOnlyPlan>, active: bool) -> Result<()> {
        ctx.accounts.plan.active = active;
        Ok(())
    }

    /// A new benefits description creates a new version. Members keep the hash
    /// they joined under so they can show what was promised when they paid.
    pub fn set_plan_benefits_hash(
        ctx: Context<CreatorOnlyPlan>,
        benefits_hash: [u8; 32],
    ) -> Result<()> {
        ctx.accounts.plan.benefits_hash = benefits_hash;
        emit!(PlanBenefitsUpdated {
            plan: ctx.accounts.plan.key(),
            benefits_hash,
        });
        Ok(())
    }

    /// The member pays one period. Manual in Phase 1: the member signs each
    /// time. A charge cannot exceed the plan price, cannot happen inside the
    /// period, and cannot happen on a closed plan or a paused creator.
    ///
    /// `last_charged_at` is the start of the paid period, not the wall-clock
    /// time of the payment. A monthly payment inside `GRACE_SECONDS` of its
    /// due date keeps the anniversary; a later one re-anchors the membership
    /// to the payment date, so a member never buys less than a month (SEC-3).
    ///
    /// Paying again after cancelling is a deliberate act by the member and
    /// re-activates the membership.
    pub fn charge_membership_period(ctx: Context<ChargeMembershipPeriod>) -> Result<()> {
        require!(!ctx.accounts.global.paused, SupportError::ProtocolPaused);
        require!(
            !ctx.accounts.support_config.paused,
            SupportError::CreatorPaused
        );
        require!(ctx.accounts.plan.active, SupportError::PlanClosed);

        let now = Clock::get()?.unix_timestamp;
        let plan = &ctx.accounts.plan;
        let membership = &mut ctx.accounts.membership;

        let period_start = if membership.member == Pubkey::default() {
            membership.member = ctx.accounts.member.key();
            membership.plan = plan.key();
            membership.joined_at = now;
            membership.benefits_hash_at_join = plan.benefits_hash;
            membership.bump = ctx.bumps.membership;
            now
        } else {
            require!(
                membership.member == ctx.accounts.member.key(),
                SupportError::Unauthorized
            );
            let monthly = plan.period_seconds == MONTHLY_PERIOD_SECONDS;
            let earliest = if monthly {
                next_month(
                    membership.last_charged_at,
                    next_month(membership.joined_at, 0)?.1,
                )?
                .0
            } else {
                membership
                    .last_charged_at
                    .checked_add(plan.period_seconds)
                    .ok_or(SupportError::MathOverflow)?
            };
            require!(now >= earliest, SupportError::PeriodNotElapsed);
            let in_grace = now
                <= earliest
                    .checked_add(GRACE_SECONDS)
                    .ok_or(SupportError::MathOverflow)?;
            if monthly && in_grace {
                earliest
            } else {
                if monthly {
                    // Lapsed: the paid month runs from today and the
                    // anniversary moves with it.
                    membership.joined_at = now;
                }
                now
            }
        };

        let amount = plan.price;
        let (creator_amount, treasury_amount) = split(amount)?;
        pay(
            &ctx.accounts.token_program,
            &ctx.accounts.usdc_mint,
            &ctx.accounts.member,
            &ctx.accounts.member_usdc_account,
            &ctx.accounts.creator_payment_account,
            &ctx.accounts.treasury_usdc_account,
            creator_amount,
            treasury_amount,
        )?;

        membership.last_charged_at = period_start;
        membership.periods_paid = membership
            .periods_paid
            .checked_add(1)
            .ok_or(SupportError::MathOverflow)?;
        membership.cancelled = false;

        emit!(MembershipCharged {
            creator: ctx.accounts.support_config.creator,
            plan: plan.key(),
            member: membership.member,
            amount,
            creator_amount,
            treasury_amount,
            period_index: membership.periods_paid,
            timestamp: now,
        });
        Ok(())
    }

    /// Opt-in permission for at most twelve calendar-month renewals. The source
    /// stays member-owned; only this program's PDA can use its allowance.
    pub fn authorize_renewal(
        ctx: Context<AuthorizeRenewal>,
        payments: u16,
        expected_price: u64,
        expected_benefits_hash: [u8; 32],
    ) -> Result<()> {
        require!(!ctx.accounts.global.paused, SupportError::ProtocolPaused);
        require!(
            !ctx.accounts.support_config.paused,
            SupportError::CreatorPaused
        );
        require!(ctx.accounts.plan.active, SupportError::PlanClosed);
        require!((1..=12).contains(&payments), SupportError::InvalidMandate);
        require!(
            ctx.accounts.plan.price == expected_price
                && ctx.accounts.plan.benefits_hash == expected_benefits_hash,
            SupportError::InvalidMandate
        );
        require!(
            ctx.accounts.plan.period_seconds == MONTHLY_PERIOD_SECONDS,
            SupportError::InvalidMandate
        );
        let membership = &ctx.accounts.membership;
        require!(
            !membership.cancelled && membership.periods_paid > 0,
            SupportError::InvalidMandate
        );
        let now = Clock::get()?.unix_timestamp;
        let anchor_day = next_month(membership.joined_at, 0)?.1;
        let (due, _) = next_month(membership.last_charged_at, anchor_day)?;
        require!(now < due, SupportError::InvalidMandate);
        let source = &ctx.accounts.member_usdc_account;
        if let anchor_lang::solana_program::program_option::COption::Some(delegate) =
            source.delegate
        {
            require!(
                delegate == ctx.accounts.delegate.key(),
                SupportError::OtherDelegate
            );
        }
        let total = ctx
            .accounts
            .plan
            .price
            .checked_mul(u64::from(payments))
            .ok_or(SupportError::MathOverflow)?;
        let mandate = &mut ctx.accounts.mandate;
        let previous = if mandate.source == source.key() {
            mandate
                .price
                .checked_mul(u64::from(mandate.remaining))
                .ok_or(SupportError::MathOverflow)?
        } else {
            0
        };
        // A new consent cannot silently move an existing mandate to another source.
        require!(
            mandate.source == Pubkey::default() || mandate.source == source.key(),
            SupportError::InvalidMandate
        );
        let allowance = source
            .delegated_amount
            .saturating_sub(previous)
            .checked_add(total)
            .ok_or(SupportError::MathOverflow)?;
        token_interface::approve_checked(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                ApproveChecked {
                    to: source.to_account_info(),
                    mint: ctx.accounts.usdc_mint.to_account_info(),
                    delegate: ctx.accounts.delegate.to_account_info(),
                    authority: ctx.accounts.member.to_account_info(),
                },
            ),
            allowance,
            ctx.accounts.usdc_mint.decimals,
        )?;
        mandate.member = membership.member;
        mandate.plan = ctx.accounts.plan.key();
        mandate.source = source.key();
        mandate.price = ctx.accounts.plan.price;
        mandate.benefits_hash = ctx.accounts.plan.benefits_hash;
        mandate.next_charge_at = due;
        mandate.anchor_day = anchor_day;
        mandate.remaining = payments;
        mandate.periods_paid = membership.periods_paid;
        mandate.bump = ctx.bumps.mandate;
        Ok(())
    }

    /// Permissionless submission: caller pays SOL but can only execute the exact
    /// signed mandate. Chain time and account locks prevent early/duplicate charges.
    pub fn charge_renewal(ctx: Context<ChargeRenewal>) -> Result<()> {
        require!(!ctx.accounts.global.paused, SupportError::ProtocolPaused);
        require!(
            !ctx.accounts.support_config.paused,
            SupportError::CreatorPaused
        );
        require!(ctx.accounts.plan.active, SupportError::PlanClosed);
        let now = Clock::get()?.unix_timestamp;
        let mandate = &mut ctx.accounts.mandate;
        let membership = &mut ctx.accounts.membership;
        require!(!membership.cancelled, SupportError::InvalidMandate);
        require!(
            mandate.remaining > 0 && mandate.periods_paid == membership.periods_paid,
            SupportError::InvalidMandate
        );
        require!(
            mandate.price == ctx.accounts.plan.price
                && mandate.benefits_hash == ctx.accounts.plan.benefits_hash,
            SupportError::InvalidMandate
        );
        require!(
            now >= mandate.next_charge_at,
            SupportError::PeriodNotElapsed
        );
        // Missed renewals expire after the grace window. No catch-up debt.
        require!(
            now <= mandate
                .next_charge_at
                .checked_add(GRACE_SECONDS)
                .ok_or(SupportError::MathOverflow)?,
            SupportError::MandateExpired
        );
        let (creator_amount, treasury_amount) = split(mandate.price)?;
        let source_key = mandate.source;
        let delegate_seeds: &[&[u8]] = &[
            b"renewal-delegate",
            source_key.as_ref(),
            &[ctx.bumps.delegate],
        ];
        for (destination, amount) in [
            (&ctx.accounts.creator_payment_account, creator_amount),
            (&ctx.accounts.treasury_usdc_account, treasury_amount),
        ] {
            token_interface::transfer_checked(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    TransferChecked {
                        from: ctx.accounts.member_usdc_account.to_account_info(),
                        mint: ctx.accounts.usdc_mint.to_account_info(),
                        to: destination.to_account_info(),
                        authority: ctx.accounts.delegate.to_account_info(),
                    },
                    &[delegate_seeds],
                ),
                amount,
                ctx.accounts.usdc_mint.decimals,
            )?;
        }
        mandate.remaining -= 1;
        mandate.next_charge_at = next_month(mandate.next_charge_at, mandate.anchor_day)?.0;
        membership.last_charged_at = now;
        membership.periods_paid = membership
            .periods_paid
            .checked_add(1)
            .ok_or(SupportError::MathOverflow)?;
        mandate.periods_paid = membership.periods_paid;
        emit!(MembershipCharged {
            creator: ctx.accounts.support_config.creator,
            plan: ctx.accounts.plan.key(),
            member: membership.member,
            amount: mandate.price,
            creator_amount,
            treasury_amount,
            period_index: membership.periods_paid,
            timestamp: now,
        });
        Ok(())
    }

    /// Cancellation prevents future charges and does nothing else.
    pub fn cancel_membership(ctx: Context<CancelMembership>) -> Result<()> {
        let membership = &mut ctx.accounts.membership;
        membership.cancelled = true;
        emit!(MembershipCancelled {
            plan: membership.plan,
            member: membership.member,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    /// The creator asks to move where they are paid. It takes effect after the
    /// cooling period, and the pending change is visible on-chain meanwhile.
    pub fn request_payment_account_rotation(
        ctx: Context<RequestPaymentAccountRotation>,
    ) -> Result<()> {
        require!(!ctx.accounts.global.paused, SupportError::ProtocolPaused);
        let now = Clock::get()?.unix_timestamp;
        let config = &mut ctx.accounts.support_config;
        config.pending_payment_account = ctx.accounts.new_payment_account.key();
        config.rotation_effective_at = now
            .checked_add(ROTATION_COOLING_SECONDS)
            .ok_or(SupportError::MathOverflow)?;
        emit!(PaymentAccountRotationRequested {
            creator: config.creator,
            new_payment_account: config.pending_payment_account,
            effective_at: config.rotation_effective_at,
        });
        Ok(())
    }

    /// The creator can withdraw a pending rotation at any time before it applies,
    /// including while the protocol is paused: cancelling only reduces risk.
    pub fn cancel_payment_account_rotation(ctx: Context<CreatorOnlyConfig>) -> Result<()> {
        let config = &mut ctx.accounts.support_config;
        config.pending_payment_account = Pubkey::default();
        config.rotation_effective_at = 0;
        Ok(())
    }

    /// Anyone may apply a rotation once the cooling period has passed; only
    /// the creator's earlier signature decided what it is.
    pub fn apply_payment_account_rotation(ctx: Context<ApplyPaymentAccountRotation>) -> Result<()> {
        require!(!ctx.accounts.global.paused, SupportError::ProtocolPaused);
        let now = Clock::get()?.unix_timestamp;
        let config = &mut ctx.accounts.support_config;
        require!(
            config.pending_payment_account != Pubkey::default(),
            SupportError::NoPendingRotation
        );
        require!(
            now >= config.rotation_effective_at,
            SupportError::RotationNotYetEffective
        );
        let previous = config.payment_account;
        config.payment_account = config.pending_payment_account;
        config.pending_payment_account = Pubkey::default();
        config.rotation_effective_at = 0;
        emit!(PaymentAccountRotated {
            creator: config.creator,
            previous_payment_account: previous,
            payment_account: config.payment_account,
        });
        Ok(())
    }
}

/// Two `transfer_checked` calls in one instruction. If either fails the whole
/// transaction reverts, so the split is atomic by construction.
#[allow(clippy::too_many_arguments)]
fn pay<'info>(
    token_program: &Interface<'info, TokenInterface>,
    mint: &InterfaceAccount<'info, Mint>,
    payer: &Signer<'info>,
    from: &InterfaceAccount<'info, TokenAccount>,
    creator_account: &InterfaceAccount<'info, TokenAccount>,
    treasury_account: &InterfaceAccount<'info, TokenAccount>,
    creator_amount: u64,
    treasury_amount: u64,
) -> Result<()> {
    if creator_amount > 0 {
        token_interface::transfer_checked(
            CpiContext::new(
                token_program.to_account_info(),
                TransferChecked {
                    from: from.to_account_info(),
                    mint: mint.to_account_info(),
                    to: creator_account.to_account_info(),
                    authority: payer.to_account_info(),
                },
            ),
            creator_amount,
            mint.decimals,
        )?;
    }
    if treasury_amount > 0 {
        token_interface::transfer_checked(
            CpiContext::new(
                token_program.to_account_info(),
                TransferChecked {
                    from: from.to_account_info(),
                    mint: mint.to_account_info(),
                    to: treasury_account.to_account_info(),
                    authority: payer.to_account_info(),
                },
            ),
            treasury_amount,
            mint.decimals,
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct GlobalConfig {
    pub admin: Pubkey,
    pub registrar: Pubkey,
    pub usdc_mint: Pubkey,
    pub treasury_usdc_account: Pubkey,
    pub paused: bool,
    pub bump: u8,
}

/// A pending admin hand-over. Kept in its own PDA so the deployed
/// `GlobalConfig` layout is unchanged.
#[account]
#[derive(InitSpace)]
pub struct AdminTransfer {
    pub pending_admin: Pubkey,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct SupportConfig {
    pub creator: Pubkey,
    pub payment_account: Pubkey,
    pub pending_payment_account: Pubkey,
    pub rotation_effective_at: i64,
    pub plan_count: u32,
    pub paused: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct MembershipPlan {
    pub creator: Pubkey,
    pub index: u32,
    pub price: u64,
    pub period_seconds: i64,
    pub benefits_hash: [u8; 32],
    pub active: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Membership {
    pub member: Pubkey,
    pub plan: Pubkey,
    pub joined_at: i64,
    pub last_charged_at: i64,
    pub periods_paid: u32,
    pub benefits_hash_at_join: [u8; 32],
    pub cancelled: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct RenewalMandate {
    pub member: Pubkey,
    pub plan: Pubkey,
    pub source: Pubkey,
    pub price: u64,
    pub benefits_hash: [u8; 32],
    pub next_charge_at: i64,
    pub remaining: u16,
    pub periods_paid: u32,
    pub anchor_day: u8,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct AuthorizeRenewal<'info> {
    #[account(mut)]
    pub member: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Box<Account<'info, GlobalConfig>>,
    #[account(seeds = [CREATOR_SEED, support_config.creator.as_ref()], bump = support_config.bump)]
    pub support_config: Box<Account<'info, SupportConfig>>,
    #[account(seeds = [PLAN_SEED, plan.creator.as_ref(), &plan.index.to_le_bytes()], bump = plan.bump, constraint = plan.creator == support_config.creator @ SupportError::PlanCreatorMismatch)]
    pub plan: Box<Account<'info, MembershipPlan>>,
    #[account(seeds = [MEMBERSHIP_SEED, plan.key().as_ref(), member.key().as_ref()], bump = membership.bump, has_one = member, has_one = plan)]
    pub membership: Box<Account<'info, Membership>>,
    #[account(init_if_needed, payer = member, space = 8 + RenewalMandate::INIT_SPACE, seeds = [b"renewal", membership.key().as_ref()], bump)]
    pub mandate: Box<Account<'info, RenewalMandate>>,
    #[account(mut, constraint = member_usdc_account.owner == member.key() @ SupportError::Unauthorized, constraint = member_usdc_account.mint == global.usdc_mint @ SupportError::WrongMint)]
    pub member_usdc_account: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: PDA authority, not a data account; the seeds fix the only delegate accepted.
    #[account(seeds = [b"renewal-delegate", member_usdc_account.key().as_ref()], bump)]
    pub delegate: UncheckedAccount<'info>,
    #[account(address = global.usdc_mint @ SupportError::WrongMint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ChargeRenewal<'info> {
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Box<Account<'info, GlobalConfig>>,
    #[account(seeds = [CREATOR_SEED, support_config.creator.as_ref()], bump = support_config.bump)]
    pub support_config: Box<Account<'info, SupportConfig>>,
    #[account(seeds = [PLAN_SEED, plan.creator.as_ref(), &plan.index.to_le_bytes()], bump = plan.bump, constraint = plan.creator == support_config.creator @ SupportError::PlanCreatorMismatch)]
    pub plan: Box<Account<'info, MembershipPlan>>,
    #[account(mut, seeds = [MEMBERSHIP_SEED, plan.key().as_ref(), membership.member.as_ref()], bump = membership.bump, has_one = plan)]
    pub membership: Box<Account<'info, Membership>>,
    #[account(mut, seeds = [b"renewal", membership.key().as_ref()], bump = mandate.bump, has_one = plan, constraint = mandate.member == membership.member @ SupportError::Unauthorized)]
    pub mandate: Box<Account<'info, RenewalMandate>>,
    #[account(mut, address = mandate.source, constraint = member_usdc_account.owner == mandate.member @ SupportError::Unauthorized, constraint = member_usdc_account.mint == global.usdc_mint @ SupportError::WrongMint)]
    pub member_usdc_account: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: seeds bind this transfer authority to the source account.
    #[account(seeds = [b"renewal-delegate", member_usdc_account.key().as_ref()], bump)]
    pub delegate: UncheckedAccount<'info>,
    #[account(address = global.usdc_mint @ SupportError::WrongMint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,
    // Raw constraints run in order, so a substituted address reports
    // WrongPaymentAccount before the owner check below.
    #[account(
        mut,
        constraint = creator_payment_account.key() == support_config.payment_account @ SupportError::WrongPaymentAccount,
        // SEC-4: an account whose owner changed after registration is refused,
        // so a phished SetAuthority stops payments instead of redirecting them.
        constraint = creator_payment_account.owner == support_config.creator @ SupportError::PaymentAccountNotOwnedByCreator,
    )]
    pub creator_payment_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, address = global.treasury_usdc_account @ SupportError::WrongTreasuryAccount)]
    pub treasury_usdc_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

// ---------------------------------------------------------------------------
// Instruction account sets
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        init,
        payer = admin,
        space = 8 + GlobalConfig::INIT_SPACE,
        seeds = [GLOBAL_SEED],
        bump,
    )]
    pub global: Account<'info, GlobalConfig>,
    pub usdc_mint: InterfaceAccount<'info, Mint>,
    #[account(constraint = treasury_usdc_account.mint == usdc_mint.key() @ SupportError::WrongMint)]
    pub treasury_usdc_account: InterfaceAccount<'info, TokenAccount>,
    /// This program, so its ProgramData account can be checked.
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ SupportError::Unauthorized)]
    pub program: Program<'info, crate::program::InfxSupport>,
    /// Only the upgrade authority may initialise.
    #[account(constraint = program_data.upgrade_authority_address == Some(admin.key()) @ SupportError::Unauthorized)]
    pub program_data: Account<'info, ProgramData>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetTreasury<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [GLOBAL_SEED], bump = global.bump, has_one = admin @ SupportError::Unauthorized)]
    pub global: Account<'info, GlobalConfig>,
    #[account(constraint = treasury_usdc_account.mint == global.usdc_mint @ SupportError::WrongMint)]
    pub treasury_usdc_account: InterfaceAccount<'info, TokenAccount>,
}

#[derive(Accounts)]
pub struct ProposeAdmin<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump, has_one = admin @ SupportError::Unauthorized)]
    pub global: Account<'info, GlobalConfig>,
    #[account(
        init_if_needed,
        payer = admin,
        space = 8 + AdminTransfer::INIT_SPACE,
        seeds = [ADMIN_TRANSFER_SEED],
        bump,
    )]
    pub admin_transfer: Account<'info, AdminTransfer>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelAdminTransfer<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump, has_one = admin @ SupportError::Unauthorized)]
    pub global: Account<'info, GlobalConfig>,
    #[account(mut, seeds = [ADMIN_TRANSFER_SEED], bump = admin_transfer.bump, close = admin)]
    pub admin_transfer: Account<'info, AdminTransfer>,
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    #[account(mut)]
    pub new_admin: Signer<'info>,
    #[account(mut, seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(
        mut,
        seeds = [ADMIN_TRANSFER_SEED],
        bump = admin_transfer.bump,
        constraint = admin_transfer.pending_admin == new_admin.key() @ SupportError::Unauthorized,
        close = new_admin,
    )]
    pub admin_transfer: Account<'info, AdminTransfer>,
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [GLOBAL_SEED], bump = global.bump, has_one = admin @ SupportError::Unauthorized)]
    pub global: Account<'info, GlobalConfig>,
}

#[derive(Accounts)]
pub struct RegisterCreator<'info> {
    #[account(mut)]
    pub registrar: Signer<'info>,
    pub creator: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump, has_one = registrar @ SupportError::Unauthorized)]
    pub global: Account<'info, GlobalConfig>,
    #[account(
        init,
        payer = registrar,
        space = 8 + SupportConfig::INIT_SPACE,
        seeds = [CREATOR_SEED, creator.key().as_ref()],
        bump,
    )]
    pub support_config: Account<'info, SupportConfig>,
    #[account(
        constraint = payment_account.mint == global.usdc_mint @ SupportError::WrongMint,
        constraint = payment_account.owner == creator.key() @ SupportError::PaymentAccountNotOwnedByCreator,
    )]
    pub payment_account: InterfaceAccount<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetCreatorPaused<'info> {
    pub authority: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(mut, seeds = [CREATOR_SEED, support_config.creator.as_ref()], bump = support_config.bump)]
    pub support_config: Account<'info, SupportConfig>,
}

#[derive(Accounts)]
pub struct Gift<'info> {
    pub fan: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(seeds = [CREATOR_SEED, support_config.creator.as_ref()], bump = support_config.bump)]
    pub support_config: Account<'info, SupportConfig>,
    #[account(address = global.usdc_mint @ SupportError::WrongMint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,
    #[account(
        mut,
        constraint = fan_usdc_account.mint == global.usdc_mint @ SupportError::WrongMint,
        constraint = fan_usdc_account.owner == fan.key() @ SupportError::Unauthorized,
    )]
    pub fan_usdc_account: InterfaceAccount<'info, TokenAccount>,
    // Raw constraints run in order, so a substituted address reports
    // WrongPaymentAccount before the owner check below.
    #[account(
        mut,
        constraint = creator_payment_account.key() == support_config.payment_account @ SupportError::WrongPaymentAccount,
        // SEC-4: an account whose owner changed after registration is refused,
        // so a phished SetAuthority stops payments instead of redirecting them.
        constraint = creator_payment_account.owner == support_config.creator @ SupportError::PaymentAccountNotOwnedByCreator,
    )]
    pub creator_payment_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, address = global.treasury_usdc_account @ SupportError::WrongTreasuryAccount)]
    pub treasury_usdc_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct CreateMembershipPlan<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(
        mut,
        seeds = [CREATOR_SEED, creator.key().as_ref()],
        bump = support_config.bump,
        has_one = creator @ SupportError::Unauthorized,
    )]
    pub support_config: Account<'info, SupportConfig>,
    #[account(
        init,
        payer = creator,
        space = 8 + MembershipPlan::INIT_SPACE,
        seeds = [PLAN_SEED, creator.key().as_ref(), &support_config.plan_count.to_le_bytes()],
        bump,
    )]
    pub plan: Account<'info, MembershipPlan>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreatorOnlyPlan<'info> {
    pub creator: Signer<'info>,
    #[account(
        mut,
        seeds = [PLAN_SEED, creator.key().as_ref(), &plan.index.to_le_bytes()],
        bump = plan.bump,
        has_one = creator @ SupportError::Unauthorized,
    )]
    pub plan: Account<'info, MembershipPlan>,
}

#[derive(Accounts)]
pub struct CreatorOnlyConfig<'info> {
    pub creator: Signer<'info>,
    #[account(
        mut,
        seeds = [CREATOR_SEED, creator.key().as_ref()],
        bump = support_config.bump,
        has_one = creator @ SupportError::Unauthorized,
    )]
    pub support_config: Account<'info, SupportConfig>,
}

#[derive(Accounts)]
pub struct ChargeMembershipPeriod<'info> {
    #[account(mut)]
    pub member: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(seeds = [CREATOR_SEED, support_config.creator.as_ref()], bump = support_config.bump)]
    pub support_config: Account<'info, SupportConfig>,
    #[account(
        seeds = [PLAN_SEED, support_config.creator.as_ref(), &plan.index.to_le_bytes()],
        bump = plan.bump,
        constraint = plan.creator == support_config.creator @ SupportError::PlanCreatorMismatch,
    )]
    pub plan: Account<'info, MembershipPlan>,
    #[account(
        init_if_needed,
        payer = member,
        space = 8 + Membership::INIT_SPACE,
        seeds = [MEMBERSHIP_SEED, plan.key().as_ref(), member.key().as_ref()],
        bump,
    )]
    pub membership: Account<'info, Membership>,
    #[account(address = global.usdc_mint @ SupportError::WrongMint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,
    #[account(
        mut,
        constraint = member_usdc_account.mint == global.usdc_mint @ SupportError::WrongMint,
        constraint = member_usdc_account.owner == member.key() @ SupportError::Unauthorized,
    )]
    pub member_usdc_account: InterfaceAccount<'info, TokenAccount>,
    // Raw constraints run in order, so a substituted address reports
    // WrongPaymentAccount before the owner check below.
    #[account(
        mut,
        constraint = creator_payment_account.key() == support_config.payment_account @ SupportError::WrongPaymentAccount,
        // SEC-4: an account whose owner changed after registration is refused,
        // so a phished SetAuthority stops payments instead of redirecting them.
        constraint = creator_payment_account.owner == support_config.creator @ SupportError::PaymentAccountNotOwnedByCreator,
    )]
    pub creator_payment_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, address = global.treasury_usdc_account @ SupportError::WrongTreasuryAccount)]
    pub treasury_usdc_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelMembership<'info> {
    pub member: Signer<'info>,
    #[account(
        mut,
        seeds = [MEMBERSHIP_SEED, membership.plan.as_ref(), member.key().as_ref()],
        bump = membership.bump,
        has_one = member @ SupportError::Unauthorized,
    )]
    pub membership: Account<'info, Membership>,
}

#[derive(Accounts)]
pub struct RequestPaymentAccountRotation<'info> {
    pub creator: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(
        mut,
        seeds = [CREATOR_SEED, creator.key().as_ref()],
        bump = support_config.bump,
        has_one = creator @ SupportError::Unauthorized,
    )]
    pub support_config: Account<'info, SupportConfig>,
    #[account(
        constraint = new_payment_account.mint == global.usdc_mint @ SupportError::WrongMint,
        constraint = new_payment_account.owner == creator.key() @ SupportError::PaymentAccountNotOwnedByCreator,
    )]
    pub new_payment_account: InterfaceAccount<'info, TokenAccount>,
}

#[derive(Accounts)]
pub struct ApplyPaymentAccountRotation<'info> {
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(
        mut,
        seeds = [CREATOR_SEED, support_config.creator.as_ref()],
        bump = support_config.bump
    )]
    pub support_config: Account<'info, SupportConfig>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[event]
pub struct CreatorRegistered {
    pub creator: Pubkey,
    pub payment_account: Pubkey,
}

#[event]
pub struct CreatorPauseChanged {
    pub creator: Pubkey,
    pub paused: bool,
}

#[event]
pub struct ProtocolPauseChanged {
    pub paused: bool,
}

#[event]
pub struct TreasuryChanged {
    pub previous_treasury_usdc_account: Pubkey,
    pub treasury_usdc_account: Pubkey,
}

#[event]
pub struct AdminChanged {
    pub previous_admin: Pubkey,
    pub admin: Pubkey,
}

#[event]
pub struct SupportReceived {
    pub creator: Pubkey,
    pub fan: Pubkey,
    pub amount: u64,
    pub creator_amount: u64,
    pub treasury_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct PlanCreated {
    pub creator: Pubkey,
    pub plan: Pubkey,
    pub index: u32,
    pub price: u64,
    pub period_seconds: i64,
    pub benefits_hash: [u8; 32],
}

#[event]
pub struct PlanBenefitsUpdated {
    pub plan: Pubkey,
    pub benefits_hash: [u8; 32],
}

#[event]
pub struct MembershipCharged {
    pub creator: Pubkey,
    pub plan: Pubkey,
    pub member: Pubkey,
    pub amount: u64,
    pub creator_amount: u64,
    pub treasury_amount: u64,
    pub period_index: u32,
    pub timestamp: i64,
}

#[event]
pub struct MembershipCancelled {
    pub plan: Pubkey,
    pub member: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct PaymentAccountRotationRequested {
    pub creator: Pubkey,
    pub new_payment_account: Pubkey,
    pub effective_at: i64,
}

#[event]
pub struct PaymentAccountRotated {
    pub creator: Pubkey,
    pub previous_payment_account: Pubkey,
    pub payment_account: Pubkey,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[error_code]
pub enum SupportError {
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("The protocol is paused")]
    ProtocolPaused,
    #[msg("This creator is paused")]
    CreatorPaused,
    #[msg("Signer is not authorised for this action")]
    Unauthorized,
    #[msg("Amount is below the minimum")]
    BelowMinimum,
    #[msg("Plan price is outside the allowed range")]
    PriceOutOfRange,
    #[msg("Plan period is outside the allowed range")]
    PeriodOutOfRange,
    #[msg("Plan is closed to new charges")]
    PlanClosed,
    #[msg("The membership period has not elapsed since the last charge")]
    PeriodNotElapsed,
    #[msg("Token account is for the wrong mint")]
    WrongMint,
    #[msg("Payment account is not the creator's registered account")]
    WrongPaymentAccount,
    #[msg("Treasury account does not match the global configuration")]
    WrongTreasuryAccount,
    #[msg("Payment account must be owned by the creator")]
    PaymentAccountNotOwnedByCreator,
    #[msg("Plan does not belong to this creator")]
    PlanCreatorMismatch,
    #[msg("No payment-account rotation is pending")]
    NoPendingRotation,
    #[msg("The rotation cooling period has not elapsed")]
    RotationNotYetEffective,
    #[msg("Renewal permission is inactive or its accepted terms have changed")]
    InvalidMandate,
    #[msg("The renewal retry window has expired; fresh consent is required")]
    MandateExpired,
    #[msg("This USDC account already grants permission to another application; revoke it in your wallet first")]
    OtherDelegate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_five_dollars() {
        let (creator, treasury) = split(5_000_000).unwrap();
        assert_eq!(creator, 4_995_000);
        assert_eq!(treasury, 5_000);
    }

    #[test]
    fn split_rounds_treasury_down_and_conserves() {
        for amount in (0..2_000_000u64).step_by(9_973) {
            let (creator, treasury) = split(amount).unwrap();
            assert_eq!(creator + treasury, amount);
            assert!(treasury <= amount / 1000);
            assert!(treasury * BPS_DENOMINATOR <= amount * FEE_BPS);
        }
        let (creator, treasury) = split(1001).unwrap();
        assert_eq!((creator, treasury), (1000, 1));
    }

    #[test]
    fn split_max_does_not_overflow() {
        assert!(split(u64::MAX / FEE_BPS).is_ok());
        assert!(split(u64::MAX).is_err());
    }
}

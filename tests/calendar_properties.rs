//! Property and differential tests for the hand-rolled calendar and the fee
//! split. These pin the behaviour that the renewal schedule and every fan
//! charge depend on, without touching `src/lib.rs`.
//!
//! `calendar` is a private module, so the source file is compiled into this
//! test crate directly; `crate::SupportError` resolves through the re-export.

pub use lumi::SupportError;

#[allow(dead_code)]
#[path = "../src/calendar.rs"]
mod calendar;

use calendar::next_month;
use lumi::{split, BPS_DENOMINATOR, FEE_BPS, MAX_PLAN_PRICE, MIN_GIFT, MIN_PLAN_PRICE};

const DAY: i64 = 86_400;

/// Independent civil-date arithmetic (Howard Hinnant's algorithms), used as
/// the oracle for the program's loop-based implementation.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)) as u32
}

fn ts(y: i64, m: u32, d: u32, secs_into_day: i64) -> i64 {
    days_from_civil(y, m, d) * DAY + secs_into_day
}

/// What the program should return, computed independently.
fn oracle(timestamp: i64, anchor: u8) -> (i64, u8) {
    let (y, m, d) = civil_from_days(timestamp.div_euclid(DAY));
    let day = if anchor == 0 { d as u8 } else { anchor };
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let clamped = u32::from(day).min(days_in_month(ny, nm));
    (
        days_from_civil(ny, nm, clamped) * DAY + timestamp.rem_euclid(DAY),
        day,
    )
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }
}

#[test]
fn fee_constants_are_the_agreed_split() {
    // 0.1% to the treasury. Changing this needs a named human (CLAUDE.md).
    assert_eq!(FEE_BPS, 10);
    assert_eq!(BPS_DENOMINATOR, 10_000);
    assert_eq!(MIN_GIFT, 1_000_000);
    assert_eq!(MIN_PLAN_PRICE, 1_000_000);
    assert_eq!(MAX_PLAN_PRICE, 500_000_000);
}

#[test]
fn split_conserves_and_never_rounds_toward_the_treasury() {
    let mut rng = Lcg(7);
    for _ in 0..50_000 {
        let amount = rng.next() % (u64::MAX / FEE_BPS);
        let (creator, treasury) = split(amount).unwrap();
        assert_eq!(creator + treasury, amount);
        assert_eq!(treasury, amount / 1000);
        assert!(treasury as u128 * BPS_DENOMINATOR as u128 <= amount as u128 * FEE_BPS as u128);
    }
    // Every in-range gift and plan price produces a non-zero treasury share,
    // so `pay` always executes both transfers.
    assert!(split(MIN_GIFT).unwrap().1 > 0);
    assert!(split(MIN_PLAN_PRICE).unwrap().1 > 0);
    assert_eq!(split(999).unwrap(), (999, 0));
}

#[test]
fn next_month_matches_an_independent_calendar() {
    let mut rng = Lcg(2026);
    for _ in 0..100_000 {
        let timestamp = (rng.next() % 7_258_118_400) as i64;
        let anchor = (rng.next() % 32) as u8;
        let expected = oracle(timestamp, anchor);
        let actual = next_month(timestamp, anchor).unwrap();
        assert_eq!(actual, expected, "timestamp {timestamp} anchor {anchor}");
    }
}

#[test]
fn next_month_is_monotonic_and_bounded() {
    let mut rng = Lcg(99);
    for _ in 0..20_000 {
        let a = (rng.next() % 7_000_000_000) as i64;
        let b = a + (rng.next() % (400 * DAY as u64)) as i64;
        for anchor in [0u8, 1, 15, 28, 29, 30, 31] {
            let (na, _) = next_month(a, anchor).unwrap();
            let (nb, _) = next_month(b, anchor).unwrap();
            // Monotonic at day granularity: the returned timestamp keeps the
            // time-of-day of its input, so two charges in the same month can
            // order differently by hours.
            assert!(
                na.div_euclid(DAY) <= nb.div_euclid(DAY),
                "not monotonic at {a}..{b} anchor {anchor}"
            );
            let gap = na - a;
            // Always strictly in the future, at most two calendar months away.
            assert!(
                (DAY..=62 * DAY).contains(&gap),
                "gap {gap} at {a} anchor {anchor}"
            );
        }
    }
}

#[test]
fn next_month_preserves_time_of_day_and_anchor() {
    let base = ts(2027, 5, 31, 13 * 3600 + 7 * 60 + 9);
    let (next, day) = next_month(base, 0).unwrap();
    assert_eq!(day, 31);
    assert_eq!(next.rem_euclid(DAY), 13 * 3600 + 7 * 60 + 9);
    assert_eq!(next, ts(2027, 6, 30, 13 * 3600 + 7 * 60 + 9));
    // The anchor survives a clamped month: 31 -> 30 -> 31.
    let (after, day) = next_month(next, day).unwrap();
    assert_eq!(day, 31);
    assert_eq!(after, ts(2027, 7, 31, 13 * 3600 + 7 * 60 + 9));
}

#[test]
fn next_month_handles_year_rollover_and_the_2100_non_leap_year() {
    assert_eq!(
        next_month(ts(2026, 12, 31, 0), 0).unwrap().0,
        ts(2027, 1, 31, 0)
    );
    assert_eq!(
        next_month(ts(2099, 12, 15, 0), 0).unwrap().0,
        ts(2100, 1, 15, 0)
    );
    // 2100 is divisible by 100 but not 400, so February has 28 days.
    assert_eq!(
        next_month(ts(2100, 1, 31, 0), 0).unwrap().0,
        ts(2100, 2, 28, 0)
    );
    assert_eq!(
        next_month(ts(2096, 1, 31, 0), 0).unwrap().0,
        ts(2096, 2, 29, 0)
    );
}

#[test]
fn next_month_rejects_out_of_range_input() {
    assert!(next_month(-1, 0).is_err());
    assert!(next_month(7_258_118_400, 0).is_err());
    assert!(next_month(7_258_118_399, 0).is_ok());
    // An anchor above 31 can only come from corrupted state; refuse it.
    assert!(next_month(ts(2026, 9, 16, 0), 32).is_err());
    assert!(next_month(ts(2026, 9, 16, 0), 255).is_err());
}

/// SEC-3: the calendar alone would let a late payer's next due date fall on
/// the anchor day of the following month, however soon that is. Paying on
/// 31 January with an anchor of 1 would buy a single day. This is why
/// `charge_membership_period` re-anchors a payment that lands more than
/// `GRACE_SECONDS` after its due date; the validator suite in the app
/// repository pins that behaviour.
#[test]
fn calendar_alone_would_shorten_a_late_payers_next_period_to_one_day() {
    let paid_late = ts(2027, 1, 31, 12 * 3600);
    let (earliest_next, _) = next_month(paid_late, 1).unwrap();
    assert_eq!(earliest_next - paid_late, DAY);

    let paid_late = ts(2027, 2, 28, 9 * 3600);
    let (earliest_next, _) = next_month(paid_late, 15).unwrap();
    assert_eq!(earliest_next - paid_late, 15 * DAY);
}

/// Documents finding: `charge_renewal` records the actual (possibly late)
/// charge time on the membership while advancing the mandate from the
/// scheduled time. Re-authorising after a late renewal that crossed a month
/// boundary computes the next due date one month later than the mandate would.
#[test]
fn late_renewal_across_a_month_boundary_diverges_from_the_schedule() {
    let scheduled = ts(2027, 1, 30, 10 * 3600);
    let anchor = 30;
    let mandate_next = next_month(scheduled, anchor).unwrap().0;
    assert_eq!(mandate_next, ts(2027, 2, 28, 10 * 3600));

    // Charged two days late, still inside the 72-hour retry window.
    let charged_at = scheduled + 2 * DAY;
    let membership_next = next_month(charged_at, anchor).unwrap().0;
    assert_eq!(membership_next, ts(2027, 3, 30, 10 * 3600));
    assert_eq!(membership_next - mandate_next, 30 * DAY);
}

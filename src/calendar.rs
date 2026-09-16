use crate::SupportError;
use anchor_lang::prelude::*;

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
fn days_in_month(year: i64, month: usize) -> i64 {
    [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ][month]
}

/// Calendar months at the same UTC time, clamping short months to their last day.
/// Keep the original day so Jan 31 -> Feb 28 -> Mar 31.
pub fn next_month(timestamp: i64, anchor: u8) -> Result<(i64, u8)> {
    require!(
        (0..7_258_118_400).contains(&timestamp),
        SupportError::InvalidMandate
    );
    let mut days = timestamp / 86400;
    let time = timestamp % 86400;
    let mut year = 1970;
    while days >= if leap(year) { 366 } else { 365 } {
        days -= if leap(year) { 366 } else { 365 };
        year += 1;
    }
    let mut month = 0;
    while days >= days_in_month(year, month) {
        days -= days_in_month(year, month);
        month += 1;
    }
    let day = if anchor == 0 {
        (days + 1) as u8
    } else {
        anchor
    };
    require!((1..=31).contains(&day), SupportError::InvalidMandate);
    month += 1;
    if month == 12 {
        month = 0;
        year += 1;
    }
    let mut result = 0;
    for y in 1970..year {
        result += if leap(y) { 366 } else { 365 };
    }
    for m in 0..month {
        result += days_in_month(year, m);
    }
    result += i64::from(day).min(days_in_month(year, month)) - 1;
    Ok((result * 86400 + time, day))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_anniversaries() {
        let (feb, day) = next_month(1_706_659_200, 0).unwrap(); // 2024-01-31
        assert_eq!(feb, 1_709_164_800); // leap-year Feb 29
        assert_eq!(next_month(feb, day).unwrap().0, 1_711_843_200); // March 31
        assert_eq!(next_month(1_738_281_600, 0).unwrap().0, 1_740_700_800); // 2025 Jan31 -> Feb28
    }
}

//! UTC calendar dates without a date-crate dependency.

/// Whole days since 1970-01-01 (UTC), now.
pub fn today_epoch_days() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86400
}

/// `YYYY-MM-DD` for a count of days since 1970-01-01, via Howard Hinnant's
/// civil-from-days algorithm.
pub fn epoch_days_to_date(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::epoch_days_to_date;

    #[test]
    fn known_dates() {
        assert_eq!(epoch_days_to_date(0), "1970-01-01");
        assert_eq!(epoch_days_to_date(19_723), "2024-01-01");
        assert_eq!(epoch_days_to_date(19_782), "2024-02-29"); // leap day
        assert_eq!(epoch_days_to_date(19_783), "2024-03-01");
        assert_eq!(epoch_days_to_date(20_757), "2026-10-31");
        assert_eq!(epoch_days_to_date(10_956), "1999-12-31");
    }
}

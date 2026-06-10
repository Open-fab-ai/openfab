//! Minimal UTC timestamp formatting without pulling a date crate.
//!
//! Why hand-rolled: the dependency budget (AGENTS.md) prefers the smallest design.
//! We only need a stable ISO-8601 string and unix seconds for the decision log and
//! attestations; a 40-line civil-from-days conversion beats adding `chrono`/`time`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the unix epoch.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// ISO-8601 UTC, e.g. `2026-06-09T04:15:30Z`. Deterministic given the input second.
pub fn iso8601(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    let (h, m, s) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Current time as ISO-8601 UTC.
pub fn iso_now() -> String {
    iso8601(unix_now())
}

/// Howard Hinnant's civil_from_days: days since 1970-01-01 -> (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_timestamp() {
        // 1_700_000_000 = 2023-11-14T22:13:20Z
        assert_eq!(iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
    }
}

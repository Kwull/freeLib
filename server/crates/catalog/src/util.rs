//! Small helpers shared with the importer (no date/time crate needed).

use std::time::{SystemTime, UNIX_EPOCH};

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Days since 1970-01-01 of a civil date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Current UTC time as RFC 3339 (`2024-05-01T12:34:56Z`).
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// Milliseconds since the Unix epoch.
pub fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Validate / normalise an INPX date: returns `YYYY-MM-DD` or `""`.
/// Accepts `YYYY-MM-DD`, `YYYY-M-D` and a trailing time part.
pub fn parse_date(s: &str) -> String {
    let s = s.trim();
    let s = s.split([' ', 'T']).next().unwrap_or("");
    let mut it = s.split('-');
    let (Some(y), Some(m), Some(d), None) = (it.next(), it.next(), it.next(), it.next()) else {
        return String::new();
    };
    match (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) {
        (Ok(y), Ok(m), Ok(d)) if (1000..=9999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d) => {
            format!("{y:04}-{m:02}-{d:02}")
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        for z in [-1000, 0, 59, 60, 11_016, 20_000, 30_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert!(now_rfc3339().ends_with('Z'));
    }

    #[test]
    fn dates() {
        assert_eq!(parse_date("2009-01-02"), "2009-01-02");
        assert_eq!(parse_date(" 2009-1-2 "), "2009-01-02");
        assert_eq!(parse_date("2009-01-02 12:00:00"), "2009-01-02");
        assert_eq!(parse_date("2009-13-02"), "");
        assert_eq!(parse_date("garbage"), "");
        assert_eq!(parse_date(""), "");
    }
}

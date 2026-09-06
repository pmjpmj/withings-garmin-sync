//! Timestamp conversions: Withings epoch seconds → Garmin's millisecond
//! ISO-8601 local/UTC strings, and `YYYY-MM-DD` date parsing for the window
//! flags.

use chrono::{NaiveDate, Utc};

use crate::AppError;

/// Format epoch seconds as `YYYY-MM-DDTHH:MM:SS.SSS` in the system's local
/// timezone. Garmin's write endpoints require fractional seconds on the
/// timestamps: a no-fraction payload returned HTTP 500 (ticket 11), while
/// `.SSS` values are accepted.
pub fn local_ms(epoch: i64) -> String {
    match chrono::DateTime::from_timestamp(epoch, 0) {
        Some(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string(),
        None => String::new(),
    }
}

/// Format epoch seconds as `YYYY-MM-DD` in UTC. See
/// [`local_ms`] for why the fractional seconds are required.
pub fn gmt_ms(epoch: i64) -> String {
    match chrono::DateTime::from_timestamp(epoch, 0) {
        Some(dt) => dt
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string(),
        None => String::new(),
    }
}

/// Parse a `YYYY-MM-DD` date (interpreted as midnight UTC) into epoch seconds.
pub fn parse_date(value: &str) -> Result<i64, AppError> {
    let date = NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map_err(|_| {
        AppError::new(
            crate::EXIT_USAGE,
            format!("invalid date {value:?}: expected YYYY-MM-DD"),
        )
    })?;
    Ok(date
        .and_hms_opt(0, 0, 0)
        .expect("midnight is valid")
        .and_utc()
        .timestamp())
}

/// Current time as epoch seconds (UTC).
pub fn now_epoch() -> i64 {
    Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-01-02T08:30:00Z.
    const EPOCH: i64 = 1767342600;

    #[test]
    fn gmt_ms_formats_millisecond_precision_utc() {
        assert_eq!(gmt_ms(EPOCH), "2026-01-02T08:30:00.000");
    }

    #[test]
    fn parse_date_handles_valid_and_invalid_input() {
        // 2026-01-02T00:00:00Z (midnight, as parse_date interprets dates).
        assert_eq!(parse_date("2026-01-02").unwrap(), 1767312000);
        assert!(parse_date("2026-13-99").is_err());
        assert!(parse_date("yesterday").is_err());
    }
}

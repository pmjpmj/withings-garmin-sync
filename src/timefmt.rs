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

/// Format epoch seconds as the canonical floor form (ADR-0009):
/// `YYYY-MM-DDTHH:MM:SSZ`, RFC 3339 UTC at whole-second precision.
pub fn format_floor(epoch: i64) -> String {
    match chrono::DateTime::from_timestamp(epoch, 0) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        None => String::new(),
    }
}

/// Parse a sync floor value (ADR-0009) into epoch seconds. Accepts, in
/// addition to the canonical [`format_floor`] form: RFC 3339 datetimes with
/// offsets (converted to the UTC instant), fractional seconds (truncated
/// toward the floor, never rounded up), plain `YYYY-MM-DD` dates (midnight
/// UTC, like [`parse_date`]), and legacy epoch-second integers. Anything
/// else — including naive datetimes and space separators — is a config
/// error (exit 3) naming the accepted formats.
pub fn parse_floor(value: &str) -> Result<i64, AppError> {
    let trimmed = value.trim();

    // Legacy integer epoch seconds; the DateTime check rejects values the
    // formatter cannot represent, so a loaded floor always round-trips.
    if let Ok(epoch) = trimmed.parse::<i64>() {
        if chrono::DateTime::from_timestamp(epoch, 0).is_some() {
            return Ok(epoch);
        }
    }

    // RFC 3339 datetime: offsets convert to the UTC instant, and the
    // timestamp's fractional part is dropped (toward the floor). chrono's
    // parser leniently accepts a space separator, so reject any whitespace
    // up front (the spec's accepted forms have none).
    if !trimmed.chars().any(char::is_whitespace) {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
            return Ok(dt.timestamp());
        }
    }

    // Plain date: midnight UTC, the same interpretation as --since/--until.
    if let Ok(epoch) = parse_date(trimmed) {
        return Ok(epoch);
    }

    Err(AppError::config(format!(
        "invalid floor value {value:?}: expected an RFC 3339 datetime \
         (e.g. \"2026-01-02T08:30:00Z\"), a \"YYYY-MM-DD\" date, \
         or an epoch-second integer"
    )))
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

    // ------------------------------------------------------------------
    // Floor parse/format (ADR-0009)
    // ------------------------------------------------------------------

    #[test]
    fn format_floor_renders_canonical_rfc3339_utc() {
        assert_eq!(format_floor(EPOCH), "2026-01-02T08:30:00Z");
        assert_eq!(format_floor(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn floor_canonical_form_round_trips() {
        for epoch in [EPOCH, 0, 1767312000, 1767342720, 1767342721] {
            assert_eq!(parse_floor(&format_floor(epoch)).unwrap(), epoch);
        }
    }

    #[test]
    fn parse_floor_converts_offsets_to_utc() {
        // Both spell the same UTC instant as 2026-01-02T08:30:00Z.
        assert_eq!(parse_floor("2026-01-02T10:30:00+02:00").unwrap(), EPOCH);
        assert_eq!(parse_floor("2026-01-02T03:00:00-05:30").unwrap(), EPOCH);
    }

    #[test]
    fn parse_floor_truncates_fractional_seconds_toward_the_floor() {
        assert_eq!(parse_floor("2026-01-02T08:30:00.999Z").unwrap(), EPOCH);
        assert_eq!(parse_floor("2026-01-02T08:30:00.000Z").unwrap(), EPOCH);
        assert_eq!(parse_floor("2026-01-02T08:30:00.5Z").unwrap(), EPOCH);
        // Pre-epoch: truncation is toward the floor (negative infinity),
        // never rounded up.
        assert_eq!(parse_floor("1969-12-31T23:59:59.5Z").unwrap(), -1);
    }

    #[test]
    fn parse_floor_accepts_plain_dates_as_midnight_utc() {
        assert_eq!(parse_floor("2026-01-02").unwrap(), 1767312000);
        assert_eq!(
            parse_floor("2026-01-02").unwrap(),
            parse_date("2026-01-02").unwrap()
        );
    }

    #[test]
    fn parse_floor_accepts_legacy_integer_epochs() {
        assert_eq!(parse_floor("1767342600").unwrap(), EPOCH);
        assert_eq!(parse_floor("0").unwrap(), 0);
    }

    #[test]
    fn parse_floor_rejects_non_floor_values_naming_the_accepted_formats() {
        for bad in [
            "2026-01-02T08:30:00",   // naive, no timezone
            "2026-01-02 08:30:00Z",   // space separator
            "junk",
            "99999-01-01T00:00:00Z",  // out-of-range year
            "2026-13-45",             // invalid date
            "9223372036854775807",    // i64 epoch beyond DateTime's range
            "",
        ] {
            let error = parse_floor(bad).unwrap_err();
            assert_eq!(error.code, crate::EXIT_CONFIG);
            assert!(
                error.message.contains("RFC 3339"),
                "message for {bad:?}: {}",
                error.message
            );
            assert!(
                error.message.contains("YYYY-MM-DD"),
                "message for {bad:?}: {}",
                error.message
            );
        }
    }
}

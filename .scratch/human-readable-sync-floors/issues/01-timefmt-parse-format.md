# 01: timefmt — RFC 3339 floor parse/format

**What to build:** `src/timefmt.rs` gains the floor conversion pair used by
the config boundary. `format_floor(epoch: i64) -> String` renders the
canonical form `YYYY-MM-DDTHH:MM:SSZ` (UTC, whole seconds, trailing `Z`).
`parse_floor(value: &str) -> Result<i64, AppError>` accepts, in addition to
the canonical form: RFC 3339 datetimes with offsets (`+02:00`, `-05:30`,
converted to the UTC instant), fractional seconds (truncated toward the
floor — never rounded up), plain `YYYY-MM-DD` dates (midnight UTC, same
interpretation as `parse_date`), and legacy bare integers (epoch seconds).
Naive datetimes without a timezone, space separators, and junk are errors
whose message names the accepted formats; the error is built with
`AppError::config` so a bad floor surfaces as exit 3 via `load_config`.

**Blocked by:** none

**Status:** resolved

- [ ] `format_floor(1767342600)` == `"2026-01-02T08:30:00Z"`; canonical form round-trips through `parse_floor`
- [ ] Offsets convert to UTC: `2026-01-02T10:30:00+02:00` parses as the same epoch as `08:30:00Z`
- [ ] Fractional seconds truncate toward the floor: `08:30:00.999Z` parses as `1767342600`, never the next second
- [ ] Plain dates parse as midnight UTC, matching `parse_date`
- [ ] Legacy integer strings (`"1767342600"`) parse as epoch seconds
- [ ] Rejected: naive datetime (`2026-01-02T08:30:00`), space separator, junk, out-of-range; error names the accepted formats
- [ ] Unit tests in `timefmt`'s tests module cover every accepted and rejected form

## Answer

Implemented `format_floor` / `parse_floor` in `src/timefmt.rs` with 6 new
unit tests. Space separators are rejected explicitly (chrono's RFC 3339
parser accepts them leniently). Legacy integers are range-checked against
`DateTime::from_timestamp` so every loaded floor round-trips through the
formatter.

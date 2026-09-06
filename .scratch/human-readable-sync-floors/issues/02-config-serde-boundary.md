# 02: Config serde boundary — ISO floors in MetricFloor

**What to build:** `MetricFloor.since` stays `Option<i64>` internally but
(de)serializes through the ticket-01 functions: serialize as the canonical
RFC 3339 UTC string, deserialize accepting string or integer (integer =
legacy epoch). Implement as a serde module (a `#[serde(with = ...)]` module
alongside `config.rs`) so `Config`, `SyncConfig`, `load_config`,
`write_config`, `MetricBound::resolve`, and the floor-advancement path in
`lib.rs` are untouched. Migration is automatic: integer floors deserialize
as before, and the next `write_config` (successful apply, or an `auth`
command rewriting the file) emits the canonical string — `auth` therefore
preserves floors and canonicalizes them.

**Blocked by:** 01 - timefmt RFC 3339 floor parse/format

**Status:** resolved

- [ ] `since = "2026-01-02T08:30:00Z"` loads as epoch `1767342600`; `write_config` round-trips the exact canonical string
- [ ] `since = 1767342600` still loads (legacy) and serializes as the canonical string
- [ ] Hand-edit forms from ticket 01 (offset, fractional, plain date) load and normalize on the next write
- [ ] Malformed string fails `load_config` with the existing invalid-config exit-3 error naming `sync.<metric>.since`
- [ ] `MetricFloor::is_empty` / `skip_serializing_if` behavior unchanged: absent floors still serialize away
- [ ] Legacy shared `sync.since` remains ignored (no schema change to `SyncConfig`)

## Answer

`MetricFloor.since` stays `Option<i64>` and (de)serializes through a `floor`
serde module in `config.rs` (`#[serde(with)]` on the field). `SyncConfig`
gains per-metric `deserialize_with` wrappers only so errors name
`sync.weight.since` / `sync.bp.since`. Six unit tests in `config.rs` pin the
boundary. Legacy `sync.since` remains ignored (pinned black-box and unit).

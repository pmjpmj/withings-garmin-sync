# 03: floor_label — ISO in reports

**What to build:** `floor_label` in `src/lib.rs` (and the combined
`window_label` paths that call it) print the canonical ISO floor via the
ticket-01 formatter instead of the raw epoch. Flag-free dry-run and apply
reports read `weight floor 2026-01-02T08:30:00Z` (UTC, matching config and
ADR-0009). Window math and flag-derived labels are untouched; the
"last 24 hours" bootstrap label is unchanged.

**Blocked by:** 01 - timefmt RFC 3339 floor parse/format

**Status:** resolved

- [ ] Flag-free weight/bp reports print floors in canonical ISO UTC
- [ ] Combined report (`weight floor …, bp floor …`) uses ISO for both metrics
- [ ] "last 24 hours" bootstrap label and flag-derived labels unchanged

## Answer

`floor_label` prints the canonical ISO floor via `timefmt::format_floor`;
window math and flag-derived labels untouched. Pinned by
`flag_free_report_labels_show_iso_floors` in `tests/floors.rs`.

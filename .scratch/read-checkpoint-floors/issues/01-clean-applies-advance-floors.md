# 01: Clean applies advance floors to the Withings query timestamp

**What to build:** the ADR-0010 rule end-to-end. On `sync --apply` (or a
metric-scoped apply) run *without* `--since`/`--until`, each included metric
whose writes all succeeded advances its sync floor to the Withings query
timestamp — the local clock captured at the moment the Withings request
fires (the `enddate` actually sent, before pagination) — whether the metric
wrote data, wrote nothing, or had every reading skipped. Flag-driven applies
(`--since`/`--until`) never touch floors. A metric with no floor yet gains
one on its first clean flag-free apply. A floor never moves backwards
(monotonic guard); an inverted window (nothing read) advances nothing. The
apply report makes every moved floor visible: `weight: 2 written, 0 skipped,
0 failed — floor advanced to 2026-01-02T08:30:00Z` (per metric, only when it
moved). Dry runs never write config, failed metrics never advance (per-metric
all-or-nothing, unchanged), and a config-write failure still warns and
leaves floors unadvanced.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] An empty clean apply (no Withings data) writes the metric's floor to
  the query timestamp, and the report says so
- [x] A clean apply with data lands the floor on the query timestamp, not on
  the newest written measurement timestamp
- [x] An apply where every reading was skipped still advances the floor
- [x] `--since`/`--until` applies leave all floors byte-identical
- [x] The floor never moves backwards, even when the query timestamp is
  older than the stored floor
- [x] A floorless metric gains a floor on its first clean flag-free apply
- [x] A metric with failed writes never advances; a clean metric in the same
  run does (existing per-metric isolation, unchanged)
- [x] Dry runs still never write the config
- [x] The output note appears per metric only when that metric's floor
  moved; dry-run output is unchanged
- [x] Black-box tests pin all of the above (the old
  "empty apply never writes config" and floor-value pins are rewritten to
  the query-timestamp behavior)

## Answer

Implemented ADR-0010 end-to-end.

- `src/withings.rs`: `read_measures` takes `enddate: Option<i64>` (`None` =
  capture the local clock at the moment the first request fires) and
  returns `MeasuresRead { groups, query_ts }` — the `enddate` actually
  sent, captured before pagination.
- `src/lib.rs`: on a clean flag-free apply, each included metric's floor
  advances to the query timestamp with a monotonic guard
  (`floor.map_or(ts, |f| f.max(ts))`); flag-driven applies and inverted
  windows (no request fired) advance nothing; a config-write failure warns
  and clears the advancement claims so the report never claims an
  unpersisted floor. The report line appends
  ` — floor advanced to <RFC 3339>` per metric, only when it moved and was
  persisted.
- `tests/floors.rs`: rewrote the floor-value pins to the query-timestamp
  behavior and added `apply_with_every_reading_skipped_still_advances_the_floor`,
  `inverted_window_reads_nothing_and_advances_nothing`,
  `monotonic_guard_never_moves_a_floor_backwards`, and
  `until_flag_alone_leaves_floors_byte_identical` (30 tests).
- Full suite green (155 tests), `cargo check`/`cargo clippy` clean, touched
  files rustfmt-clean.

## Comments

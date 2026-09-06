# 01: Warn when a sync floor is in the future

**What to build:** a startup diagnostic for wedged floors. A stored floor
ahead of the local clock can never advance — the ADR-0010 monotonic guard
refuses to lower it — so the metric's strictly-newer reads see nothing
forever: every run reports `0 written, 0 skipped, 0 failed` and exits clean
while the metric is silently dead (`sync all` reads at the other metric's
bound and filters this one away; a metric-scoped run fires no Withings
request at all — inverted window). Before this ticket the operator got no
signal that anything was wrong; the only remedy (hand-lowering the floor in
`config.toml`) was undiscoverable. On any sync run, when an included
metric's floor is ahead of the local clock, warn on stderr naming the
metric, the canonical RFC 3339 floor, and the config key to hand-lower.

**Blocked by:** None.

**Status:** resolved

- [x] `sync all --apply` with a future bp floor warns on stderr, still
      advances the healthy metric's floor, and leaves the wedged one put
      with no advancement note on its report line
- [x] A dry run warns too and stays byte-identical
- [x] The warning is scoped to the metrics in play (ADR-0005): `sync bp`
      does not warn about a future weight floor and vice versa
- [x] Existing future-floor tests (`inverted_window_…`,
      `monotonic_guard_…`) now pin the warning as well
- [x] README documents the wedge and the remedy

## Comments

Reported live 2026-09-06: `sync all` stopped advancing the bp floor.
Diagnosis: the stored `sync.bp.since` was `2028-09-06T08:57:09Z` (two years
in the future — a hand-edit typo; Withings data held no future-dated
groups and no machine-clock skew was in evidence). The advancement logic was
correct: ADR-0010's monotonic guard was doing its job. The real gap was the
silent failure mode. Fixed with a startup warning plus regression tests
(`tests/floors.rs`: `sync_all_warns_when_a_floor_is_in_the_future`,
`dry_run_warns_about_a_future_floor_without_writing_anything`,
`future_floor_warning_is_scoped_to_the_metrics_in_play`).

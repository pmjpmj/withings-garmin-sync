# 14 - Split sync into per-metric runs (weight every 2h, BP once daily)

Type: task
Status: resolved

## Question

The operator wants weight and blood-pressure to sync at different cadences:
weight every 2 hours (Garmin dedups weight by timestamp, so frequent runs are
safe) and blood pressure once a day (its dedup is the day-granular read-back,
so a second same-day run would skip genuinely new readings). Today `sync`
always does both metrics in one run. How should the CLI separate them?

## Facts so far

- `run_sync` (src/lib.rs) reads one Withings window (`read_measures`,
  meastypes `1,9,10,11` in a single call), transforms into weights + bps,
  then in apply mode writes weight (no read-back) and BP (day-granular
  read-back-and-skip, ticket 12). Per-metric failure isolation already
  exists inside the combined run.
- Garmin dedup asymmetry (observed live): weight dedups identical-timestamp
  writes (ticket 01); BP does not (ticket 12).
- Day-granular BP dedup means a second BP run on the same day skips
  everything on already-populated days — including new same-day readings —
  and those readings are unrecoverable (re-running with `--since` hits the
  same populated day). Hence once-a-day evening scheduling.
- The CLI is deliberately stateless (no cursor/watermark; ticket 12 rejected
  one).
- CLI today: `Command::{Auth, Sync}` (src/cli.rs); `SyncArgs` carries
  `--apply`/`--dry-run`/`--since`/`--until`/`--config-dir`/`--verbose`.
- `transform::Metric::{Weight, BloodPressure}` already exists
  (src/transform.rs).

## Decisions (grilling, 2026-09-05 — see ADR-0005)

1. **CLI shape**: metric subcommands under `sync`: `sync weight`,
   `sync bp`, `sync all`; bare `sync` = `all` (back-compatible).
   Subcommand is `bp` (operator's choice); prose stays "blood-pressure".
2. **No state**: cadence is external scheduling (cron/systemd). Ticket 12's
   no-cursor decision stands; once-a-day is an accepted constraint, not a
   CLI-enforced guard.
3. **Window resolution unchanged** (ADR-0001) and shared by all runs; no
   per-metric window config.
4. **Withings reads scoped per metric**: `sync weight` → meastype `1`;
   `sync bp` → `9,10,11`; `sync all` → `1,9,10,11`.
5. **Combined run unchanged**: one read, weight writes then BP
   (read-back-and-skip), per-metric failure isolation.
6. **Reporting**: single-metric runs report only their metric;
   `summary:` names the metric(s) in play. Exit codes unchanged.
7. **Docs**: README scheduling examples — weight `0 */2 * * *`, BP daily
   evening (`15 21 * * *`) — plus the BP same-day-drop limitation note.

## Implementation checklist (unclaimed)

- [x] `cli.rs`: `sync` subcommands (`weight`, `bp`, `all`), shared flattened
      args; bare `sync` = `all`.
- [x] `withings.rs`: `read_measures` takes a meastype set per metric.
- [x] `lib.rs`: metric-scoped read → transform → write; weight path never
      touches BP read-back and vice versa; report lines metric-aware.
- [x] Tests: subcommand parsing; scoped Withings request meastypes;
      single-metric runs don't hit the other metric's endpoints;
      bare `sync` == `sync all`.

## Answer

Implemented per ADR-0005.

- `cli.rs`: `SyncArgs` gained an optional `SyncMetric` subcommand
  (`weight`/`bp`/`all`) plus shared flattened `SyncOptions`; a new plain
  `MetricScope` enum carries the resolved scope into `lib.rs`.
  `args_conflicts_with_subcommands` makes flags-before-subcommand
  (`sync --apply weight`) exit 2 loudly instead of silently dropping the
  flag.
- `withings.rs`: `read_measures` now takes the `meastypes` string;
  `WEIGHT_MEASTYPES` (`1`), `BP_MEASTYPES` (`9,10,11`), `ALL_MEASTYPES`
  (`1,9,10,11`). `MEASURE_TYPES` is gone.
- `lib.rs`: `run_sync` resolves the scope (bare `sync` = `all`), scopes the
  Withings read, gates the BP read-back and both write loops on the scope,
  and reports metric-aware: single-metric runs print only their metric and
  `summary: weight synced` / `summary: blood-pressure synced` (dry runs:
  `summary: weight dry-run complete` / `summary: blood-pressure dry-run
  complete`); combined runs keep the old lines and exit codes.
  `MetricScope::meastypes()` owns the Withings mapping and
  `SyncMetric::into_parts()` splits the parsed subcommand.
- Tests (black-box, TDD at the ticket's seams): subcommand parsing + help,
  flags-vs-subcommand conflict, per-scope `meastypes`, bare `sync` ==
  `sync all`, weight apply never touches the BP endpoints, BP apply never
  touches the weight endpoint, scoped report lines. Full suite: 103 tests,
  clippy clean.
- Docs (README scheduling + CONTEXT glossary + ADR-0005) were already in
  place from the planning phase.

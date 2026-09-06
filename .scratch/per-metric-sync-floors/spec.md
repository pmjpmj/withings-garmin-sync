# Spec: per-metric-sync-floors

Status: ready-for-agent

## Problem Statement

The operator runs `sync` on a schedule (weight every 2 hours, blood pressure
daily in the evening) to copy Withings weight and blood-pressure measurements
into Garmin Connect. Because Garmin's blood-pressure endpoint does not dedup
writes by timestamp, the CLI compensates with a day-granular read-back: before
writing BP it asks Garmin which days already have a measurement and skips
those days entirely. That coarse guard caps BP at one effective run per day:
a second run on the same day skips everything, including genuinely new
readings, and a reading taken after the day's run can never be recovered.

The operator wants blood pressure synced every 2 hours like weight, so a BP
reading taken any time of day lands in Garmin quickly and same-day
multi-readings are preserved. Withings is the source of truth for the data
and returns each measurement with an epoch-second timestamp, so the dedup
boundary can be made exact on the Withings side instead of approximate on the
Garmin side.

## Solution

The sync window's lower bound becomes **per-metric and machine-maintained**.
The config file gains two floors — `sync.weight.since` and `sync.bp.since`,
each an epoch-second timestamp at Withings' precision — and after every
successful apply run the CLI rewrites each metric's floor to the timestamp of
the newest measurement it wrote for that metric. The next run reads only data
strictly newer than the floor, so the same measurement is never fetched
twice, and no Garmin-side dedup is needed: the day-granular BP read-back is
deleted. Window precedence per metric becomes: `--since`/`--until` flags,
then the metric's floor, then the built-in rolling last 24 hours (which now
only bootstraps a metric that has no floor yet).

The CLI stays one-shot and stateless at runtime — the floor is config, not
memory. Blood pressure moves to a 2-hourly cadence, and any cadence is now
safe. Because Garmin does not dedup BP writes, forcing an older window
(`--since`, or hand-lowering a floor) re-writes already-synced BP and
duplicates it; this is documented explicitly rather than silently guarded.

## User Stories

1. As an operator, I want each metric to remember where its last successful sync ended (a per-metric floor stored in config), so that the next run starts exactly where the last one stopped.
2. As an operator, I want the floors stored as epoch seconds (the precision Withings returns), so that no measurement is skipped or duplicated by rounding.
3. As an operator, I want the floors kept in config.toml as `sync.weight.since` and `sync.bp.since`, so that I can inspect and hand-adjust them.
4. As an operator, I want a metric's first-ever run (no floor in config) to fall back to the rolling last 24 hours, so that a fresh install behaves like it does today.
5. As an operator, I want `--since`/`--until` flags to override the floors, so that I can do targeted backfills.
6. As an operator, I want `--until` on its own to keep its "from the beginning" semantics, so that flag behavior doesn't surprise me.
7. As an operator, I want a run to read data strictly newer than the floor (never re-reading the last-written measurement), so that normal scheduled runs never re-send anything.
8. As an operator, I want a metric's floor to advance only after all of that metric's writes succeed in an apply run, so that a failed run retries its data on the next run instead of losing it.
9. As an operator, I want a dry run to leave the config file untouched, so that previewing costs nothing and never moves the floors.
10. As an operator, I want a run that writes nothing for a metric to leave that metric's floor untouched, so that no gap is opened that could swallow late data.
11. As an operator, I want `sync all` to advance the weight and BP floors independently, so that one metric's failure never blocks the other metric's progress.
12. As an operator, I want `sync weight` and `sync bp` to touch only their own floor (and their own endpoints), so that metric-scoped runs stay metric-scoped.
13. As an operator, I want to schedule blood pressure every 2 hours, so that afternoon and evening readings land in Garmin the same day.
14. As an operator, I want a second BP reading on the same day to be uploaded, so that multiple daily readings are no longer dropped.
15. As an operator, I want a reading taken after a run to be picked up by the next run, so that nothing is lost because of when the measurement was taken relative to the schedule.
16. As an operator, I want normal scheduled runs never to duplicate BP entries, so that my Garmin history stays clean without any day-granular skipping.
17. As an operator, I want scheduled runs to make no read-back call to Garmin's BP range endpoint, so that runs are faster and use fewer Garmin requests.
18. As an operator, I want the docs to state plainly that Garmin does not dedup BP writes, so that I understand why forcing an older window duplicates entries.
19. As an operator doing a backfill with `--since`, I want the floors still to advance to the newest written measurement afterward, so that later scheduled runs don't redo the backfill.
20. As an operator, I want to hand-lower a floor in config.toml to re-sync a range, so that I have a config-level backfill lever besides the flags.
21. As an operator upgrading, I want an existing config.toml to keep loading (floors absent, legacy shared `sync.since` ignored), so that the upgrade doesn't break my setup.
22. As an operator, I want `auth` commands to preserve the floors when they rewrite config.toml, so that re-authenticating never resets sync progress.
23. As an operator, I want the floors written with the same 0600 permissions as the rest of the config, so that my data stays private.
24. As an operator, I want a clear warning if the config file can't be written after Garmin writes succeeded, so that I know the floor didn't advance and the next run may re-send.
25. As an operator, I want exit codes and per-metric reports to keep their existing semantics, so that my cron alerts and logs keep working unchanged.
26. As an operator, I want the README to show the new config keys and the BP no-dedup caveat, so that I can migrate my config and understand the trade-off.
27. As an operator, I want the one-second exclusivity rule documented (a hypothetical second different measurement within the same second as the floor is skipped), so that the boundary semantics are explicit.
28. As an implementer, I want the whole feature testable through the existing black-box harness against fake servers, so that behavior is pinned without touching internals.

## Implementation Decisions

- **Config schema.** The sync section gains two per-metric floors, optional
  epoch-second integers (absent = that metric has no floor yet):

  ```toml
  [sync.weight]
  since = 1767342600   # epoch seconds, machine-updated

  [sync.bp]
  since = 1767342600
  ```

  The legacy shared `sync.since` key is dropped from the schema; if present in
  an existing file it is ignored (serde ignores unknown keys). No new file is
  introduced — the floors live in the existing config file.
- **Per-metric window resolution.** Each metric resolves its lower bound
  independently: `--since`/`--until` flags (unchanged, ADR-0001), else that
  metric's floor, else the rolling last 24 hours (`DEFAULT_WINDOW_SECS`
  unchanged). The upper bound is "now" as today. An explicit flag bounds the
  window by itself exactly as ADR-0001 describes.
- **Strictly-newer reads.** The Withings read for a metric starts at
  `floor + 1` second, so the last-written measurement is never re-fetched.
  Same-second different measurements are thereby excluded (documented; see
  Further Notes). For a metric without a floor, the resolved fallback bound is
  used as today.
- **Combined runs.** `sync` / `sync all` scopes Withings reads per metric as
  before (meastypes `1` for weight; `9,10,11` for BP) but each metric now has
  its own lower bound. The implementation reads once at the earliest resolved
  bound and filters each metric client-side to its own floor (strictly
  newer), keeping one Withings round-trip for the combined run.
- **Floor update.** After a successful apply run, each metric's floor is set
  to the maximum Withings timestamp among that metric's successfully written
  measurements, then the config file is rewritten via the existing
  whole-file serialize-and-write-0600 path. This happens for every successful
  apply, including flag-driven backfills. Dry runs, failed metrics, and
  empty runs never write the config. A config-write failure after successful
  Garmin writes produces a warning and does not change the run's reported
  outcome.
- **Read-back removal.** The BP day-granular read-back (Garmin BP range
  read, day-skip logic in the apply path, and any CLI surface only it uses)
  is deleted. No run calls Garmin's BP range endpoint anymore.
- **Reporting and exit codes.** Dry-run/apply output and exit codes keep
  their current semantics; metric-scoped runs keep reporting only their own
  metric. The dry-run report can show which floor bound each metric used.
- **Docs.** README gains the new config keys, the BP no-dedup /
  backfill-duplicates warning, and the 2-hourly BP cadence (replacing the
  daily-evening recommendation and the same-day-drop limitation). CONTEXT.md
  glossary and cadence entries are updated, and the change is recorded as
  ADR-0008 (amending ADR-0001's precedence table and ADR-0005's dedup
  mechanism and cadence; the runtime-stateless clause stands, with config
  now carrying machine-maintained floors).

## Testing Decisions

- **What makes a good test.** Black-box only: run the built binary against
  in-process fake Withings/Garmin servers (base URLs overridden via the
  existing `WGS_*` environment variables) with a fresh temp config dir per
  test, and assert on external behavior — exit codes, stdout/stderr, files
  written, and the requests the fakes received. No test may depend on
  internal function names or module layout.
- **Seam.** One seam, the existing one: the CLI binary plus the fake-server
  harness in the shared test helpers. No new seams.
- **Modules tested.** The CLI as a whole, through `sync weight`, `sync bp`,
  and `sync all` in dry-run and apply modes, plus the config file the runs
  produce. Nothing else needs direct testing.
- **Prior art.** `tests/sync.rs` (window-resolution and per-metric scoping
  tests, the apply rerun test), the fake servers in the shared test helpers
  (including the recorded-request assertions), and the existing config/token
  round-trip tests. The rerun test and the fake server's BP date-range route
  pin the deleted read-back and are removed or replaced by floor-based
  equivalents.
- **Scenarios to pin.** First run per metric uses the rolling 24-hour
  `startdate`; apply writes both floors (values equal the newest written
  Withings timestamps); the next run sends `startdate = floor + 1` and writes
  nothing when Withings returns the same data; a hand-lowered floor re-sends
  the older measurement; `--since` bypasses the floor and a successful apply
  still advances it; a failed metric leaves its floor untouched while the
  other metric's floor advances; dry-run and empty runs never modify config;
  a config containing the legacy shared `sync.since` still loads; `auth`
  preserves floor values when rewriting config; no run ever sends a Garmin BP
  range request.

## Out of Scope

- **Late-arriving measurements.** A Withings measurement that reaches the
  cloud only after its timestamp has fallen behind the floor is dropped; no
  overlap window or update-time filtering rescues it (accepted trade-off).
- **Concurrent-run protection.** No locking or single-flight guard; two
  overlapping runs can interleave floor updates and cause a one-off BP
  duplicate (known limitation, documented).
- **Atomic config replacement.** The existing write path (truncate-and-write)
  is reused unchanged; crash-during-write recovery is not addressed.
- **Garmin-side BP dedup.** Impossible by construction; the design removes
  the CLI's need for it rather than providing it.
- **Weight dedup change.** Weight continues to rely on Garmin's verified
  timestamp dedup; its behavior is unchanged except for the floor.
- **Cadence enforcement.** Scheduling stays external (cron/systemd); the CLI
  still runs one-shot and never schedules itself.
- **Flag semantics.** `--since`/`--until` parsing and precedence relative to
  each other are unchanged.

## Further Notes

- The decision is recorded as ADR-0008; CONTEXT.md glossary changes (sync
  window resolution, cadence rationale, apply description) and README updates
  accompany the implementation.
- Ticket 12's chosen strategy (day-granular read-back) is superseded by the
  floors; its accepted "known limitation" (same-day readings dropped) no
  longer applies.
- Same-second exclusivity: `floor + 1` means a second, different measurement
  sharing the exact second of the last written one would be skipped. For BP
  that is effectively the same reading; for weight, same-second weigh-ins do
  not occur in practice.

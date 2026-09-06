# ADR-0008: Per-metric machine-updated sync floors

- **Status:** Accepted
- **Date:** 2026-09-06

## Context

`sync` copies Withings weight and blood-pressure measurements into Garmin
Connect. The sync window's lower bound was shared and operator-set
(ADR-0001): `--since`/`--until` flags, then the optional `sync.since` config
key, then the built-in rolling last 24 hours. Re-running the same window
re-sends everything in it.

Weight tolerates that: Garmin's weight endpoint deduplicates writes by
timestamp. Blood pressure does not — Garmin's BP endpoint duplicates
byte-identical re-writes — so the CLI compensated with a day-granular
read-back (ADR-0005, ticket 12): before writing BP it asked Garmin which
days already had a measurement and skipped those days. That guard capped BP
at one effective run per day: a second run skipped everything, including
genuinely new readings, and a reading taken after the day's run could never
be recovered. The operator wants blood pressure synced every 2 hours like
weight, with same-day multi-readings preserved.

Withings is the source of truth and returns each measurement with an
epoch-second timestamp, so the dedup boundary can be made exact on the
Withings side instead of approximate on the Garmin side.

## Decision

- The config file gains two per-metric sync floors — `sync.weight.since` and
  `sync.bp.since`, each an optional epoch-second timestamp at Withings'
  precision. After every successful apply, the CLI rewrites each metric's
  floor to the timestamp of the newest measurement it wrote for that metric.
  The floors are config, not runtime state: the CLI stays one-shot and
  stateless, with config now carrying machine-maintained progress.
- Window precedence becomes per metric: `--since`/`--until` flags, then that
  metric's floor, then the built-in rolling last 24 hours (which now only
  bootstraps a metric that has no floor yet). This **amends ADR-0001**,
  whose precedence table listed the shared `sync.since` config key at level
  2; that key is dropped from the schema and ignored if present in an
  existing file.
- Reads are strictly newer than the floor: the Withings read for a floored
  metric starts at `floor + 1`, so the last-written measurement is never
  re-fetched and a run never re-sends anything. A second, different
  measurement sharing the exact second of the floor is thereby excluded —
  for BP that is effectively the same reading; for weight, same-second
  weigh-ins do not occur in practice. `--since`/`--until` bypass the floors
  for the run's reads (the backfill path); a successful flag-driven apply
  still advances the floors. Hand-lowering a floor in config.toml is a
  documented backfill lever.
- Combined runs (`sync` / `sync all`) keep one Withings round-trip: the read
  starts at the earliest in-play bound and each metric is filtered
  client-side to its own floor (strictly newer).
- Floor advancement is per metric and all-or-nothing: a metric's floor moves
  to the newest written timestamp only when every one of that metric's
  writes succeeded. Dry runs, empty runs, and failed metrics never write the
  config; `sync all` advances the two floors independently. A config-write
  failure after successful Garmin writes produces a warning and does not
  change the run's reported outcome.
- The day-granular Garmin BP read-back is deleted: no run calls Garmin's BP
  range endpoint anymore. This **amends ADR-0005** (whose dedup mechanism
  was the read-back) and **supersedes ticket 12's chosen read-back
  strategy** and its accepted "same-day readings dropped" limitation.
- Blood pressure moves to a 2-hourly cadence like weight; any cadence is
  now safe for both metrics. This **amends ADR-0005**'s cadence
  recommendation (BP once daily in the evening).

## Consequences

- Scheduled runs never duplicate BP entries and never re-fetch written
  measurements; a second same-day BP reading uploads normally.
- Because Garmin does not dedup BP writes, forcing an older window
  (`--since`, or hand-lowering a floor) re-writes already-synced BP and
  duplicates it; this is documented explicitly rather than silently guarded.
- A Withings measurement that reaches the cloud only after its timestamp has
  fallen behind the floor is dropped (accepted trade-off); overlapping runs
  can interleave floor updates and cause a one-off duplicate (no locking,
  documented).
- `auth` commands preserve the floors when they rewrite `config.toml`, so
  re-authenticating never resets sync progress.
- Exit codes and per-metric reporting keep their existing semantics; the
  dry-run report labels the per-metric floor bounds used.
- Tests pin the behavior black-box: first run bootstraps from the rolling
  window; the next run sends `startdate = floor + 1` and writes nothing;
  hand-lowering a floor re-sends the older measurement; failed metrics leave
  their floor untouched; no run ever sends a Garmin BP range request.

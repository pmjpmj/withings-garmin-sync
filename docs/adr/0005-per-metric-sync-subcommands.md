# ADR-0005: Per-metric sync subcommands and cadences

- **Status:** Accepted
- **Date:** 2026-09-05

## Context

Two observed facts about the Garmin write endpoints make the two metrics
asymmetric:

- The weight endpoint deduplicates identical-timestamp writes (spike ticket
  01 verified `numOfWeightEntries: 1` after two identical writes).
- The blood-pressure endpoint does **not** dedup at all (ticket 12 observed
  duplicates from byte-identical re-writes). The CLI compensates with a
  day-granular read-back-and-skip: any day on which Garmin already has at
  least one BP measurement is skipped entirely.

The single combined `sync` run bundles both metrics under one window and one
cadence. That cadence is bounded by the stricter metric: because BP dedup is
day-granular, a second BP run on the same day skips everything — including
genuinely new readings — so the combined run is effectively once-a-day.
Weight, which Garmin dedups safely at any cadence, is dragged down with it.
The operator wants weight every 2 hours but blood pressure once daily.

## Decision

- `sync` gains per-metric subcommands: `sync weight`, `sync bp`, `sync all`;
  bare `sync` remains equivalent to `sync all` (back-compatible).
- Cadence becomes an operator concern: scheduling stays external
  (cron/systemd). The CLI stays **stateless** — no cursor or watermark;
  ticket 12's rejection of stored checkpoints stands.
- Window resolution is unchanged (ADR-0001) and shared by all runs:
  `--since`/`--until` flags, then `sync.since` config, then the rolling
  last 24 hours.
- Withings reads are scoped per metric: `sync weight` requests meastype `1`
  only; `sync bp` requests `9,10,11`; `sync all` requests all four. A
  single-metric run never fetches or reports the other metric.
- The combined run's behavior is unchanged: one Withings read, weight writes
  first, then BP writes guarded by the day read-back, with per-metric failure
  isolation (one failing metric never blocks the other).

## Consequences

- Weight may run at any cadence (operator example: every 2 hours); Garmin
  dedups re-sends by timestamp.
- Blood pressure is recommended once daily, in the evening: a Withings BP
  reading taken after that day's run is skipped by the day-granular dedup on
  every later run and cannot be recovered by re-running with `--since`.
- Single-metric runs report and exit-code only their own metric; exit code
  semantics are unchanged.
- The README scheduling section carries both cadences and the BP
  same-day-drop limitation.

# ADR-0001: Sync window resolution

- **Status:** Accepted
- **Date:** 2026-09-04

## Context

`sync` reads a window of measurements from Withings and writes them to Garmin.
Three sources can bound that window: the `--since`/`--until` CLI flags, the
optional `sync.since` config value, and a built-in fallback. The built-in
fallback was originally the last 30 days.

The default was found to be too broad for routine use: a typical run only
needs to pick up recent weigh-ins and blood-pressure readings, and a large
window means more work and more noise on every run.

## Decision

The window is resolved by strict precedence:

1. `--since` / `--until` flags (ISO `YYYY-MM-DD`). `--since` alone means
   "since … until now"; `--until` alone means "from the beginning until …"
   (`since = 0`). An explicit flag bounds the window by itself.
2. `sync.since` config, only when no window flag is given.
3. Built-in fallback: the **rolling last 24 hours**
   (`DEFAULT_WINDOW_SECS = 86400` in `src/lib.rs`), a rolling window like its
   30-day predecessor rather than a calendar day, so no timezone or
   midnight-boundary logic is introduced.

The fallback window is labeled `"last 24 hours"` in dry-run and apply output.
`--until` alone keeps its "from the beginning" semantics; the fallback only
fills an empty window.

## Consequences

- `tests/sync.rs` asserts the default `startdate` is ~1 day ago.
- README and `.scratch/withings-garmin-sync/spec.md` describe the default as
  24 hours.
- No config files change; `sync.since` still overrides the fallback.

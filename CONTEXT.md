# CONTEXT

One-shot, stateless Rust CLI (`withings-garmin-sync`) that reads weight and
blood-pressure measurements from Withings and writes them to Garmin Connect.

## Glossary

- **sync window** — the date range `sync` reads from Withings. Resolved by
  precedence: `--since`/`--until` flags, then `sync.since` config, then the
  built-in rolling last 24 hours. See ADR-0001.
- **rolling window** — a window anchored to "now minus a fixed duration"
  (24 hours by default), not to calendar boundaries; no timezone logic.
- **dry run** — the default `sync` mode: read, transform, print a would-write
  report, perform no writes. `--apply` turns writes on.
- **apply** — the write mode (`sync --apply`): writes weight and
  blood-pressure to Garmin, with per-metric failure isolation and
  blood-pressure dedup-by-day read-back.

## Decisions

- `docs/adr/` — see ADR-0001 for sync window resolution.

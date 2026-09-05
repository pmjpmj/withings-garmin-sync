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
- **apply** — the write mode (`sync --apply`, or metric-scoped like
  `sync bp --apply`): writes weight and blood-pressure to Garmin, with
  per-metric failure isolation and blood-pressure dedup-by-day read-back.
- **metric-scoped run** — a `sync` invocation restricted to one metric by
  subcommand: `sync weight` or `sync bp` (prose says "blood-pressure"; the
  subcommand is `bp`). `sync` and `sync all` sync both metrics (ADR-0005).
- **cadence** — how often the operator schedules a sync (cron/systemd).
  Weight runs every 2 hours; blood-pressure once daily, in the evening,
  because its dedup is day-granular.
- **versioned release** — a GitHub Release created by pushing a `vX.Y.Z` tag
  whose version matches `Cargo.toml`. Carries the release assets (tarballs
  plus `SHA256SUMS`) built by the release pipeline.
- **release asset** — a per-target tarball attached to a versioned release
  (persistent, versioned), as opposed to a workflow artifact (CI-to-CI
  handoff, expires after 90 days).
- **tag/manifest contract** — the version tag must equal the `Cargo.toml`
  version; the pipeline verifies it and hard-fails on mismatch. Version
  bumps happen on `main` before tagging; the pipeline never edits the
  manifest.
- **per-service auth** — re-authenticating one service without the other:
  `auth withings` or `auth garmin`. Bare `auth` authenticates both (the
  first-run path). Each command replaces only its own section of
  `tokens.json`. See ADR-0006.

## Decisions

- `docs/adr/` — ADR-0001: sync window resolution. ADR-0002: release
  pipeline for Linux and macOS. ADR-0003: aarch64 Linux release asset for
  Raspberry Pi 4. ADR-0004: static musl arm64 asset (Bullseye-compatible).
  ADR-0005: per-metric sync subcommands and cadences. ADR-0006: per-service
  auth subcommands.

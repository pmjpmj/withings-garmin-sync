# withings-garmin-sync

A one-shot Rust CLI for Linux and macOS that copies **body weight** (Withings smart scale)
and **blood pressure** (Withings BP monitor) from the Withings API into Garmin
Connect, preserving each measurement's original timestamp.

Garmin has no public write API for these metrics, so the CLI writes through
Garmin's undocumented internal JSON endpoints (no FIT encoding). After a
one-time interactive `auth` step, every subsequent `sync` is non-interactive
and safe to re-run: each metric remembers its progress as an ISO-8601 floor
in `config.toml` (`sync.weight.since` / `sync.bp.since`), and the next run
reads only measurements strictly newer than the floor.

Key properties:

- **One-shot and synchronous** — no daemon, no TUI, no scheduling. Run it
  manually or from cron/systemd.
- **Dry-run by default** — `sync` prints what it would write and only touches
  Garmin when you pass `--apply`.
- **Independent metrics** — a failing weight write never blocks blood-pressure
  writes (and vice versa); the exit code reports partial success.
- **Token handling** — expired Withings tokens are refreshed before reads;
  Garmin tokens are refreshed lazily on `401` and retried once.
- **Garmin MFA support** — the auth flow pauses and prompts for the code.
- **Withings scope is minimal** — only `user.metrics`.

## Limitations

- **No body-composition metrics** (fat %, muscle mass, BMI, etc.): the JSON
  endpoints cannot carry them.
- **Undocumented API:** the Garmin write endpoints and mobile-SSO handshake
  are internal and may change without notice. Treat the tool as best-effort.
- **No bi-directional sync**, no other Withings metrics (sleep, activity,
  heart-rate series, ECG, SpO₂), no multi-account support.
- **Plaintext credentials:** tokens are stored as plaintext under your config
  directory. File permissions (`0600`/`0700`) are the only protection — keep
  them private.

## Prerequisites

1. **Rust toolchain** (stable) — only needed when building from source; see prebuilt binaries below.
2. **A Withings developer app:**
   - Register an app in the [Withings developer portal](https://developer.withings.com/).
   - Note the **client id** and **client secret** (prompted for by `auth`).
   - Register the redirect URI `http://localhost:8765/` for the app. The CLI
     never listens on this port; it's just the URI your browser is sent to
     after you authorize, so the code can be pasted back.
3. **A Garmin Connect account.** If it is located in the EU, grant
   **"upload consent"** in Garmin Connect account settings before the first
   `--apply`, or every write will fail with `412`. The CLI cannot grant this
   for you.
4. If your Garmin account has two-factor authentication, have your second
   factor (email or TOTP) handy during `auth`.

## Installation

### Prebuilt binaries

Each [versioned release](https://github.com/pmjpmj/withings-garmin-sync/releases)
(triggered by pushing a `vX.Y.Z` tag matching the `Cargo.toml` version) ships
per-target tarballs plus a `SHA256SUMS`:

- `withings-garmin-sync-<version>-linux-x86_64.tar.gz` — Linux, glibc,
  dynamically linked (no system crypto needed: TLS is rustls).
- `withings-garmin-sync-<version>-linux-arm64.tar.gz` — Linux ARM64
  (e.g. Raspberry Pi 4 on 64-bit Raspberry Pi OS), **fully static musl
  build** — no glibc or OpenSSL requirement; runs on Bullseye, Bookworm,
  and newer.
- `withings-garmin-sync-<version>-macos-arm64.tar.gz` — Apple Silicon,
  unsigned. A browser download gets quarantined by Gatekeeper; clear it with
  `xattr -d com.apple.quarantine <file>` (or use `curl`, which never
  quarantines).

Example:

```sh
curl -LO https://github.com/pmjpmj/withings-garmin-sync/releases/download/v0.2.0/withings-garmin-sync-v0.2.0-macos-arm64.tar.gz
shasum -a 256 withings-garmin-sync-v0.2.0-macos-arm64.tar.gz   # verify against SHA256SUMS
tar -xzf withings-garmin-sync-v0.2.0-macos-arm64.tar.gz
./withings-garmin-sync --help
```

### Build from source

```sh
cargo install --path .
```

This builds the native `withings-garmin-sync` binary (no runtime dependencies).

## Configuration

Run the interactive one-time auth flow:

```sh
withings-garmin-sync auth
```

It will:

1. Prompt for your Withings client id and client secret (if not already in
   `config.toml`) and save them.
2. Print a Withings authorization URL — open it in a browser and authorize
   the app.
3. Ask you to paste back the redirect URL (or bare code) your browser was
   sent to.
4. Prompt for your Garmin username (email) and password (never stored).
5. If MFA is required, prompt for the code (up to three attempts).
6. Exchange both logins for tokens and store them.

The two services' sessions die in different ways (Withings tokens expire on
their own clock; Garmin kills sessions on external events like a password
change or MFA policy). Repair one service without re-doing the other (ADR-0006):

```sh
withings-garmin-sync auth withings   # re-do only the Withings browser OAuth
withings-garmin-sync auth garmin     # re-do only the Garmin username/password/MFA login
```

Each command replaces only its own section of `tokens.json`; the other
service's tokens are left untouched. `auth garmin` needs no `config.toml`.
Bare `auth` stays the first-run path: it authenticates both services.

Files are written under `~/.config/withings-garmin-sync/` (override with
`--config-dir <dir>`):

| File | Contents | Permissions |
|---|---|---|
| `config.toml` | Static settings: Withings OAuth client id/secret, per-metric sync floors (machine-updated) | `0600` |
| `tokens.json` | Secrets: Withings + Garmin access/refresh tokens | `0600` |

Example `config.toml`:

```toml
[withings]
client_id = "your-client-id"
client_secret = "your-client-secret"

# Optional per-metric sync floors (RFC 3339 UTC datetimes). The CLI rewrites
# these after every clean flag-free apply; you normally never edit them by hand.
[sync.weight]
since = "2026-01-02T08:30:00Z"   # everything at or before this is handled

[sync.bp]
since = "2026-01-02T08:30:00Z"
```

Each floor marks the Withings query timestamp of that metric's last clean
flag-free apply (ADR-0010): everything at or before it is handled — written
or verified absent. A clean `sync --apply` advances the floor to the query
timestamp whether the metric wrote data, wrote nothing, or had every
reading skipped; applies with `--since`/`--until` never touch floors. The
next run reads strictly newer (`floor + 1`). A metric without a floor
bootstraps from the built-in rolling window (the last 24 hours), exactly
like a fresh install.

**Migration note:** the old shared `sync.since` key (a `YYYY-MM-DD` date) is
removed. Existing configs keep loading — the key is simply ignored — and the
first apply after upgrading bootstraps from the rolling window as if the
floors were absent. Configs carrying the pre-ADR-0009 integer-epoch floors
also keep loading with identical behavior; the next clean flag-free apply
(or an `auth` rewrite) stores them in the canonical ISO form.

## Usage

```sh
# Dry run (default): read the window and print what would be written.
withings-garmin-sync sync

# Actually write to Garmin Connect.
withings-garmin-sync sync --apply

# Restrict a run to one metric (the other is untouched):
withings-garmin-sync sync weight --apply   # weight only
withings-garmin-sync sync bp --apply       # blood-pressure only
withings-garmin-sync sync all --apply      # both (same as bare `sync`)

# Bound the window (ISO YYYY-MM-DD). Default: the per-metric floors, or the
# rolling last 24 hours for a metric with no floor yet.
withings-garmin-sync sync --since 2026-01-01 --until 2026-06-01
withings-garmin-sync sync --since 2026-01-01   # since ... until now
withings-garmin-sync sync --until 2026-01-01   # from the beginning until ...

# Use an alternate config directory and/or log every request/response.
withings-garmin-sync --config-dir /tmp/wgs-test sync --verbose
```

### Backfills and the BP no-dedup caveat

Normal scheduled runs never re-send anything: each metric reads only
measurements strictly newer than its floor. Two ways to force an older
window exist, and both can **duplicate blood-pressure entries** when they
re-write an already-synced BP measurement — Garmin's BP endpoint does not
deduplicate writes (weight is unaffected: Garmin dedups weight writes by
timestamp).

- `--since 2026-01-01` (optionally with `--until`) bypasses the floors for
  one run and re-reads from the flag date. Flag-driven applies leave the
  floors untouched. A backfill of data older than the floor has no knock-on
  effect: the floor stays put and the next scheduled run skips that data as
  before. A backfill that reaches data *newer* than the floor is different:
  the next scheduled run re-sends that newer data, duplicating BP (no Garmin
  dedup). The remedy is a hand-adjusted floor in `config.toml` — set the
  floor to the newest backfilled timestamp so the next run skips it.
- Hand-lowering a floor in `config.toml` (e.g. `sync.bp.since` → an earlier
  datetime like `"2026-01-01T00:00:00Z"`, or a plain `YYYY-MM-DD` date,
  which means midnight UTC) makes every subsequent apply re-send the older
  measurements. Prefer the flags for one-off backfills.

One-second exclusivity: a run reads `floor + 1` onward, so a second,
different measurement sharing the exact second of the stored floor is
skipped. For BP that is effectively the same reading; for weight,
same-second weigh-ins do not occur in practice.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Success — both metrics synced, or a dry run completed cleanly |
| `1` | Partial or total metric failure (the report says which metric and how many) |
| `2` | Usage error (unknown flag/subcommand, invalid window) |
| `3` | Config error — missing/invalid config or tokens; run `auth` first |
| `4` | Auth error — a token refresh was rejected; the message names the service: re-run `auth withings` or re-run `auth garmin` |

### Scheduling

`sync --apply` is non-interactive after `auth`, so it runs fine from cron or a
systemd timer. The per-metric floors make any cadence safe (ADR-0008): each
run reads only measurements strictly newer than its floor, so scheduled runs
never duplicate entries or re-send data — as long as the floors are
machine-maintained (flag backfills and hand-lowered floors are the
exceptions; see the backfill caveat above).

```cron
# Weight every 2 hours.
0 */2 * * * /home/you/.cargo/bin/withings-garmin-sync sync weight --apply >> /home/you/.local/log/wgs-weight.log 2>&1

# Blood pressure every 2 hours: afternoon and evening readings land the
# same day, and multiple same-day readings are all preserved.
30 */2 * * * /home/you/.cargo/bin/withings-garmin-sync sync bp --apply >> /home/you/.local/log/wgs-bp.log 2>&1
```

Do not force an older window on a schedule (see the backfill caveat above):
because Garmin does not dedup BP writes, re-sending old blood-pressure
measurements duplicates them. Reserve `--since` and hand-lowered floors for
deliberate, one-off backfills.

## Environment variables

All external base URLs are overridable (mainly for testing against local fake
servers; defaults point at the real endpoints):

| Variable | Default |
|---|---|
| `WGS_WITHINGS_API_BASE` | `https://wbsapi.withings.net` |
| `WGS_GARMIN_SSO_BASE` | `https://sso.garmin.com` |
| `WGS_GARMIN_DIAUTH_BASE` | `https://diauth.garmin.com` |
| `WGS_GARMIN_API_BASE` | `https://connectapi.garmin.com` |

## Development

```sh
cargo build              # debug build
cargo test               # integration tests (fake HTTP servers via the env-var seam)
cargo run -- sync        # dry run
```

The single test seam is the injectable HTTP base URLs above: integration
tests point the compiled binary at in-process fake servers via a temp
`--config-dir` and assert on requests received, stdout, exit code, and files
written.

### Releasing

Push a tag `vX.Y.Z` that matches the `version` in `Cargo.toml` (bump it on
`main` first). `.github/workflows/release.yml` runs the test suite, builds
Linux (x86_64 glibc, arm64 static musl) and macOS (arm64) binaries
natively, and
attaches the tarballs plus `SHA256SUMS` to the GitHub Release. See ADR-0002,
ADR-0003, and ADR-0004.

## License

Licensed under either of

- Apache License, Version 2.0 (https://www.apache.org/licenses/LICENSE-2.0)
- MIT license (https://opensource.org/licenses/MIT)

at your option.

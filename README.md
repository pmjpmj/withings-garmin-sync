# withings-garmin-sync

A one-shot Rust CLI for Linux and macOS that copies **body weight** (Withings smart scale)
and **blood pressure** (Withings BP monitor) from the Withings API into Garmin
Connect, preserving each measurement's original timestamp.

Garmin has no public write API for these metrics, so the CLI writes through
Garmin's undocumented internal JSON endpoints (no FIT encoding). After a
one-time interactive `auth` step, every subsequent `sync` is non-interactive
and safe to re-run: weight writes deduplicate by timestamp, and blood-pressure
writes are guarded by a read-back that skips any day Garmin already has.

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
  dynamically linked against system OpenSSL 3 (`libssl.so.3`, present on
  current distros).
- `withings-garmin-sync-<version>-linux-arm64.tar.gz` — Linux ARM64
  (e.g. Raspberry Pi 4 on 64-bit Raspberry Pi OS), glibc, dynamically linked
  against system OpenSSL 3 (`libssl.so.3`, present on Raspberry Pi OS
  bookworm+).
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

Files are written under `~/.config/withings-garmin-sync/` (override with
`--config-dir <dir>`):

| File | Contents | Permissions |
|---|---|---|
| `config.toml` | Static settings: Withings OAuth client id/secret, optional sync defaults | `0600` |
| `tokens.json` | Secrets: Withings + Garmin access/refresh tokens | `0600` |

Example `config.toml`:

```toml
[withings]
client_id = "your-client-id"
client_secret = "your-client-secret"

[sync]
# Optional: default lower bound for the sync window when no --since/--until
# flags are given (omit for the built-in rolling 24-hour window).
since = "2026-01-01"
```

## Usage

```sh
# Dry run (default): read the window and print what would be written.
withings-garmin-sync sync

# Actually write to Garmin Connect.
withings-garmin-sync sync --apply

# Bound the window (ISO YYYY-MM-DD). Defaults: last 24 hours.
withings-garmin-sync sync --since 2026-01-01 --until 2026-06-01
withings-garmin-sync sync --since 2026-01-01   # since ... until now
withings-garmin-sync sync --until 2026-01-01   # from the beginning until ...

# Use an alternate config directory and/or log every request/response.
withings-garmin-sync --config-dir /tmp/wgs-test sync --verbose
```

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Success — both metrics synced, or a dry run completed cleanly |
| `1` | Partial or total metric failure (the report says which metric and how many) |
| `2` | Usage error (unknown flag/subcommand, invalid window) |
| `3` | Config error — missing/invalid config or tokens; run `auth` first |
| `4` | Auth error — a token refresh was rejected; re-run `auth` |

### Scheduling

`sync --apply` is non-interactive after `auth`, so it runs fine from cron or a
systemd timer, e.g.:

```cron
# Daily at 08:15
15 8 * * * /home/you/.cargo/bin/withings-garmin-sync sync --apply >> /home/you/.local/log/wgs.log 2>&1
```

Safe to re-run: weight re-writes never double-count (Garmin deduplicates them
by timestamp), and blood-pressure re-writes are skipped via a read-back of the
days Garmin already has, so overlapping windows don't duplicate either metric.

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
Linux (x86_64 and arm64, glibc) and macOS (arm64) binaries natively, and
attaches the tarballs plus `SHA256SUMS` to the GitHub Release. See ADR-0002
and ADR-0003.

## License

Licensed under either of

- Apache License, Version 2.0 (https://www.apache.org/licenses/LICENSE-2.0)
- MIT license (https://opensource.org/licenses/MIT)

at your option.

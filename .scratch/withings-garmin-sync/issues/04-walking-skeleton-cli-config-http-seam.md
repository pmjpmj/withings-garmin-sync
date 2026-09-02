# 04: Walking skeleton: CLI, config, and the HTTP seam

**What to build:** a runnable `withings-garmin-sync` binary whose `auth` and `sync` subcommands and all flags parse correctly, that resolves the config directory, reads/writes plaintext config and token files with restrictive permissions, and honors the environment-based base-URL overrides that form the single test seam. This is the harness every later ticket plugs into.

**Blocked by:** None (can start immediately).

**Status:** done

- [x] `cargo build` produces a `withings-garmin-sync` binary with `auth` and `sync` subcommands and full flag parsing (`--dry-run` default, `--apply`, `--since`, `--until`, `--config-dir`, `--verbose`).
- [x] `--help` and `--version` work; an unknown flag/subcommand exits `2`.
- [x] Running `sync` with a missing or invalid config exits `3` with a message directing the operator to run `auth` first.
- [x] `config.toml` and `tokens.json` are read from and written to the config dir (default `~/.config/withings-garmin-sync`, overridable via `--config-dir`), and written files carry `0600` permissions.
- [x] The four base-URL overrides (`WGS_WITHINGS_API_BASE`, `WGS_GARMIN_SSO_BASE`, `WGS_GARMIN_DIAUTH_BASE`, `WGS_GARMIN_API_BASE`) are accepted and used by the HTTP client — the single test seam.
- [x] Tests drive the binary against a temp config dir and assert only stdout and exit codes (no reach into internals).

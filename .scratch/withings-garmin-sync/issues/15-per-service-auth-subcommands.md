# 15 - Split auth into per-service commands (independent re-auth)

Type: grilling
Status: resolved

## Question

The two services' credentials expire differently: Withings access tokens on
a short clock (stored `expires_at`, proactive refresh), Garmin on external
events (no stored expiry, reactive refresh on 401). Today the only repair
tool is one combined `auth` that re-does both services and persists only
when both succeed. Should auth split into per-service commands that can run
independently?

## Facts so far

- `run_auth` (src/lib.rs): prompts for Withings `client_id`/`client_secret`
  if missing → writes config → Withings browser OAuth (authorize URL, paste
  code, exchange) → Garmin username/password (+MFA loop, 3 attempts) →
  service-ticket exchange → writes both token halves to `tokens.json` only
  after both succeed ("a failure in either half of `auth` must not leave a
  partial token file behind").
- `config.rs`: single `tokens.json` with independent `withings` and `garmin`
  sections. `WithingsTokens` stores `expires_at` (epoch seconds from
  `expires_in`); `GarminTokens` stores no expiry — only access/refresh
  tokens plus the DI client id.
- `sync` refresh paths: Withings refreshes proactively against `expires_at`;
  Garmin refreshes reactively on 401, then retries once. 15 error messages
  in `src/` say "re-run `auth`" with no service distinction.
- `config.toml` is Withings-only today (`withings.client_id`/
  `client_secret` + `sync.since`); Garmin auth consumes no config.
- CLI shape precedent: ADR-0005 gave `sync weight`/`bp`/`all` subcommands
  with shared flattened `--config-dir`/`--verbose` options.
- Exit codes: `EXIT_OK`=0, `EXIT_USAGE`=2, `EXIT_CONFIG`=3, `EXIT_AUTH`=4.

## Decisions (grilling, 2026-09-05 — see ADR-0006)

1. **CLI shape**: `auth withings` / `auth garmin` subcommands (mirrors
   `sync weight|bp`); shared flattened `--config-dir`/`--verbose`.
2. **Bare `auth` stays**: bare `auth` = authenticate both (first-run path,
   spec story 3). Per-service subcommands are the repair path.
3. **Token persistence**: read-modify-write of the shared `tokens.json`;
   each command replaces only its own section. A failed `auth garmin` must
   leave `tokens.withings` byte-identical (and vice versa). No file split.
4. **Config coupling**: `auth withings` owns Withings credentials (prompts
   when missing, persists to `config.toml`); `auth garmin` needs no config
   at all.
5. **Always interactive**: per-service auth always runs its full
   interactive flow (browser OAuth / username+password+MFA). No silent
   refresh attempt in auth; refresh stays `sync`'s job.
6. **Error routing in `sync`**: Withings failures → "re-run
   `auth withings`"; Garmin failures → "re-run `auth garmin`"; missing
   tokens file → "run `auth`". Exit codes unchanged.

## Implementation checklist

- [x] `cli.rs`: `AuthArgs` gains an optional service subcommand
      (`withings`/`garmin`) + shared flattened options; bare `auth` = both.
- [x] `lib.rs`: split `run_auth` into `run_auth_withings`,
      `run_auth_garmin`, and a bare-`auth` driver that runs both;
      per-section read-modify-write of `tokens.json`.
- [x] `lib.rs`: `auth garmin` skips config validation entirely;
      `auth withings` keeps the prompt-if-empty flow.
- [x] `lib.rs`: route all "re-run `auth`" messages per service.
- [x] Tests: subcommand parsing; bare `auth` == both; `auth garmin` leaves
      the withings token section untouched and vice versa; per-service
      error-message routing; `auth garmin` runs with no config file.

## Answer

Decisions 1–6 resolved via grilling on 2026-09-05 and recorded in
ADR-0006 (per-service auth subcommands). CONTEXT.md glossary updated.
Implementation is unclaimed — next step is the `implement` or `to-tickets`
skill, not yet started.

## Comments

- 2026-09-05: Implemented. `auth withings` / `auth garmin` subcommands,
  per-section read-modify-write of `tokens.json`, `auth garmin` runs with no
  config, and sync-side error routing names the service. New black-box tests
  in `tests/auth_services.rs`; two legacy `tests/garmin_auth.rs` tests
  updated to the ADR-0006 persistence contract (a failed Garmin half no
  longer deletes the already-persisted Withings half). README and spec
  touched up with the per-service repair commands.

# ADR-0006: Per-service auth subcommands

- **Status:** Accepted
- **Date:** 2026-09-05

## Context

The single `auth` command authenticates both services in one interactive
session and persists tokens only after both halves succeed. The two
services' credentials die in different ways:

- Withings access tokens expire on a short clock (`expires_in`, ~3 hours);
  the CLI stores `expires_at` and refreshes proactively before reads.
  Refresh tokens are long-lived but die on external events such as
  client-secret rotation in the developer portal.
- Garmin DI tokens have no stored expiry at all; `sync` refreshes reactively
  on `401`. Garmin kills sessions on external events: password change, MFA
  policy, long dormancy.

The repair moment therefore differs per service, but the only repair tool
re-does both: `auth` re-runs the Withings browser flow and the Garmin
username/password/MFA flow together, and cannot persist either half unless
both succeed. Every sync-side auth failure says "re-run `auth`" without
naming a service, and a Withings-only repair (say, after rotating the client
secret) forces the operator through a Garmin login too.

## Decision

- `auth` gains per-service subcommands: `auth withings` and `auth garmin`.
  Bare `auth` remains equivalent to authenticating both services
  (back-compatible first-run path, spec story 3).
- Token persistence is a read-modify-write of the shared `tokens.json`:
  each command replaces only its own section (`tokens.withings` or
  `tokens.garmin`) and leaves the other section untouched. A failed
  `auth garmin` leaves Withings tokens byte-identical, and vice versa.
- `auth withings` owns Withings credentials: it prompts for
  `client_id`/`client_secret` when missing and persists them to
  `config.toml`. `auth garmin` needs no config at all (Garmin DI tokens
  carry their own client id).
- Auth commands are always fully interactive: `auth withings` always runs
  the browser OAuth code exchange; `auth garmin` always runs
  username/password/MFA. Silent refresh remains `sync`'s job; auth never
  tries a silent refresh first.
- Sync-side error routing names the service: Withings failures say "re-run
  `auth withings`", Garmin failures "re-run `auth garmin`", a missing or
  empty tokens file says "run `auth`". Exit codes are unchanged
  (`EXIT_AUTH` = 4).

## Consequences

- Each service can be re-authenticated independently without re-doing the
  healthy one; repairing a dead Garmin session no longer touches Withings
  credentials or tokens, and vice versa.
- `sync` still requires both halves for a combined run; repairing one
  service is exactly that service's auth command.
- Bare `auth` remains the first-run path; README and spec stories are
  touched up to document the per-service repair commands.
- `config.toml` validation is no longer a precondition of `auth garmin`;
  only `auth withings` and `sync` require valid Withings credentials.

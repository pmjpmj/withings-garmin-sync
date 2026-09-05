# ADR-0007: Auth-failure recovery policy

- **Status:** Accepted
- **Date:** 2026-09-05

## Context

When `sync` talks to a service whose access token is stale, the run should
repair itself from the stored refresh token instead of aborting: the
operator schedules `sync` unattended (cron/systemd), and an exit-4 auth
failure means re-running an interactive `auth` command by hand.

Garmin already implements the reactive half of this: any `401` on a read or
write triggers a DI token refresh (persisted to `tokens.json`) followed by a
single retry; a rejected refresh aborts with exit 4. Withings only
implements the proactive half: `sync` refreshes before reads when
`expires_at` says the token is expired. A mid-run rejection — Withings
revoking a token early, clock skew, a token killed externally — was
classified as a plain metric failure (exit 1) with no refresh attempt and no
retry, even though the stored Withings refresh token could have repaired it.

## Decision

- **Auth recovery** is the sync-side policy: when a service rejects a token
  mid-run, refresh once with the stored refresh token, retry the failed call
  once with the fresh token, and only then fail.
- Withings triggers: HTTP `401`, HTTP `403`, or a JSON body
  `status` of `401` (Withings signals token rejection in-band; the body
  check also covers a `200`-style envelope carrying `401`). Garmin's
  existing trigger (HTTP `401`) is unchanged.
- The Withings retry re-runs the whole paginated read from page 0 — never a
  partial page continuation.
- A successful mid-run refresh persists the rotated tokens immediately:
  new access token, rotated refresh token, and a fresh `expires_at` in
  `tokens.json`.
- Failure routing:
  - The refresh itself is rejected → exit 4 (`EXIT_AUTH`), "…refresh
    failed: …; re-run `auth withings`".
  - The retry with the fresh token is rejected again → exit 4, "token
    refresh succeeded but Withings still rejected the token; re-run
    `auth withings`". A second consecutive rejection means the token pair
    is dead and interactive re-auth is the only repair.
  - The retry fails for a non-auth reason (e.g. rate-limit, invalid JSON)
    → exit 1, metric failure, message unchanged.
- The proactive Withings refresh (expiry-driven, before reads) is kept:
  it prevents most rejections in the first place; the reactive path is the
  backstop.
- Code shape mirrors Garmin: `withings::read_measures` returns a typed
  `ReadFailure` (`Unauthorized` | `Failed`), and a `withings_read` helper in
  `lib.rs` owns refresh-persist-retry at the call site, exactly as
  `garmin_read`/`garmin_write` do.

## Consequences

- A cron `sync` whose Withings token died mid-run now self-heals silently;
  only a dead refresh token wakes the operator with exit 4.
- Deliberate asymmetry with Garmin: Garmin classifies a second consecutive
  `401` after a fresh token as exit 1; Withings classifies it as exit 4 with
  a "re-run `auth withings`" hint. Garmin's path is untouched by this ADR
  and may be aligned later.
- `tokens.json` is written at most twice per Withings recovery (refresh
  result, nothing on retry failure); the file format is unchanged.
- Black-box tests at the CLI seam (`tests/resilience.rs`) pin the four
  behaviors: 401 recovery, 403 recovery, body-`status`-401 recovery, and the
  two exit-4 failure routings.

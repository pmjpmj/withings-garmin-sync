# 05: Withings auth: OAuth authorization-code flow → stored tokens

**What to build:** the Withings half of the `auth` command — a one-time interactive OAuth authorization-code flow that yields persisted, refreshable Withings tokens, so later sync runs are non-interactive.

**Blocked by:** 04.

**Status:** done

- [x] `auth` prints the Withings authorize URL with the correct `response_type=code`, client id, redirect, and scope restricted to `user.metrics`.
- [x] Accepting the pasted redirect code exchanges it at the Withings token endpoint (`action=requesttoken`, `grant_type=authorization_code`) and persists access token, refresh token, and expiry to `tokens.json`.
- [x] A refresh path (`grant_type=refresh_token`) is implemented and can re-issue tokens.
- [x] A rejected code produces a clear error and a non-zero exit, without writing tokens.
- [x] Tests assert, against a fake Withings token server: the exact token request bodies, that tokens land in `tokens.json`, and failure behavior for a rejected code.

## Answer

Implemented in `src/withings.rs` + `run_auth` in `src/lib.rs`, tested black-box in `tests/withings_auth.rs` against an in-process fake server (`tests/common/mod.rs`).

- **Flow:** `auth` prompts for `client_id`/`client_secret` when config lacks them, persists `config.toml`, prints the authorize URL (`response_type=code`, `client_id`, `scope=user.metrics`, `redirect_uri=http://localhost:8765/`), accepts either the full pasted redirect URL or a bare code, exchanges it at `POST /v2/oauth2` (`action=requesttoken`, `grant_type=authorization_code`), and merges the result into `tokens.json` — preserving any existing `garmin` section.
- **Refresh:** `withings::refresh` builds the `grant_type=refresh_token` form and parses the same response shape; wired into `sync` in ticket 08.
- **Failures:** rejected code (`status 2556`), non-2xx, and transport errors exit `4` with a specific message; `tokens.json` is never written on failure.
- **Decision (deviation from the skeleton):** `tokens.json`'s `withings.expires_at` is Unix epoch seconds (`u64`) instead of a string — no chrono dependency, and ticket 08's expiry comparison is a plain integer compare. The ticket's token-shape note in the spec stays valid otherwise.
- **The authorize host** (`account.withings.com`) is hardcoded, not one of the four env-seam vars, because the CLI only *prints* the URL and never requests it (recorded in a comment in `src/withings.rs`).
- **HTTP:** `HttpClient` now owns a reqwest blocking client (cookie jar on, 30s timeout) and `post_form`; transport errors are `HttpError::Transport`, non-success responses `HttpError::Status` (both carry the body for diagnostics).

- [ ] `auth` prints the Withings authorize URL with the correct `response_type=code`, client id, redirect, and scope restricted to `user.metrics`.
- [ ] Accepting the pasted redirect code exchanges it at the Withings token endpoint (`action=requesttoken`, `grant_type=authorization_code`) and persists access token, refresh token, and expiry to `tokens.json`.
- [ ] A refresh path (`grant_type=refresh_token`) is implemented and can re-issue tokens.
- [ ] A rejected code produces a clear error and a non-zero exit, without writing tokens.
- [ ] Tests assert, against a fake Withings token server: the exact token request bodies, that tokens land in `tokens.json`, and failure behavior for a rejected code.

# 08: Resilience and observability

**What to build:** the robustness layer that keeps syncs working through token expiry, rate limiting, and the EU consent gate, and makes failures diagnosable with verbose logging.

**Blocked by:** 07.

**Status:** done

- [x] A Withings access token past expiry is refreshed before reads begin.
- [x] A Garmin `401` on a write triggers a token refresh followed by a single retry.
- [x] Withings `601` and Garmin `429` (whether surfaced as an HTTP status or inside a JSON `error.status-code`) are retried with backoff.
- [x] The Garmin `412` EU upload-consent gate surfaces a specific, actionable message telling the operator to grant upload consent in Garmin Connect settings.
- [x] `--verbose` logs each request and response.
- [x] Tests assert retry/backoff and consent-message behavior against fake servers that return those statuses.

## Answer

Implemented in `src/lib.rs` (orchestration), `src/http.rs` (verbose logging + backoff), `src/withings.rs` (601 retry), `src/garmin.rs` (`WriteFailure` split + 429 retry). Tests: `tests/resilience.rs` (8 end-to-end cases).

- **Withings expiry refresh:** before any read, if `expires_at` is missing or within 60s of now, `sync` calls the refresh grant; on success it persists the rotated access/refresh tokens and new expiry to `tokens.json`; a rejected refresh exits `4` with "re-run `auth`" and no sync happens.
- **Garmin lazy refresh:** a `401` on any write triggers one DI refresh (persisted, including a rotated refresh token), then a single retry of that write. A rejected refresh aborts the run with exit `4` ("re-run `auth`"); a still-failing retry counts as a metric failure (exit `1`). Both metrics share the refreshed token.
- **Retry with backoff:** Withings `601` on `getmeas` and on the token endpoint, and Garmin `429` on writes and DI refresh — whether HTTP status or JSON `error.status-code` (string `"429"` or number `429`) — are retried up to 3 attempts with exponential backoff (200ms base, doubled). `412` is never retried.
- **Consent gate:** `412` prints the specific "grant upload consent in your Garmin Connect account settings" message; write fails (exit `1`).
- **Verbose:** `--verbose` now logs every HTTP request (method, URL, headers, body preview) and response (status, body preview) to stderr, truncated to 300 chars. Applied to both `sync` and `auth`.
- **Decision recorded here:** a rejected Garmin refresh mid-sync aborts with exit `4` (auth-class failure per the spec's exit-code table) rather than continuing to fail every remaining write.

- [ ] A Withings access token past expiry is refreshed before reads begin.
- [ ] A Garmin `401` on a write triggers a token refresh followed by a single retry.
- [ ] Withings `601` and Garmin `429` (whether surfaced as an HTTP status or inside a JSON `error.status-code`) are retried with backoff.
- [ ] The Garmin `412` EU upload-consent gate surfaces a specific, actionable message telling the operator to grant upload consent in Garmin Connect settings.
- [ ] `--verbose` logs each request and response.
- [ ] Tests assert retry/backoff and consent-message behavior against fake servers that return those statuses.

## Comments

### Code-review fixes (post-implementation)

- **P1 fixed:** a persistent Garmin 429 delivered as HTTP 200 with a JSON `error.status-code` is no longer counted as a successful write — `write_json` re-checks the body after retries exhaust and counts a failure (regression test `persistent_json_429_is_failure_not_success`).
- Retry/backoff now also covers the `auth` command's calls: Withings 601 on the code exchange, and Garmin 429 (HTTP or JSON) on SSO login, MFA verify, and the DI service-ticket exchange.
- An incomplete `tokens.json` (e.g. `{}`, missing access tokens or client id) now exits `3` with "run `auth` first" instead of failing later with `4`.
- Refactors: one shared `http::retry` helper replaces the three ad-hoc retry loops; the verbose base-URL dump moved to `HttpClient::log_base_urls`; `json_number` renamed from `num`; removed a dead duplicated match arm in the test server.

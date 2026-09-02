# 06: Garmin auth: mobile-SSO → DI Bearer (MFA + client-id retry)

**What to build:** the Garmin half of the `auth` command — the mobile-SSO handshake that turns username/password (and an optional MFA code) into persisted, refreshable Garmin DI Bearer tokens.

**Blocked by:** 04.

**Status:** done

- [x] `auth` performs the ordered sequence: sign-in page GET (cookie jar) → login POST →, when `MFA_REQUIRED`, an interactive MFA verify POST → service-ticket exchange at the DI token endpoint → persisted access + refresh tokens.
- [x] Garmin username and password are prompted for interactively, not read from files; an MFA code is prompted for and loops on an invalid code.
- [x] The three DI client ids are tried in order and the winning client id is persisted alongside the tokens.
- [x] A refresh path (`grant_type=refresh_token`) is implemented.
- [x] Tests assert, against fake SSO and diauth servers: the ordered sequence, the native Android header set, the MFA branch, client-id retry, and token persistence.

## Answer

Implemented in `src/garmin.rs` + the Garmin half of `run_auth` in `src/lib.rs`, tested black-box in `tests/garmin_auth.rs`.

- **Flow** (matching the ticket-03 trace and the ticket-01 spike): sign-in GET (WebView UA, query `clientId=GCM_ANDROID_DARK&service=<url-encoded>`; session cookies kept by the shared reqwest jar) → login POST (same query + `locale=en-US`; exact JSON `{"username","password","rememberMe":true,"captchaToken":""}`) → `SUCCESSFUL` yields the `serviceTicketId`; `MFA_REQUIRED` reads `customerMfaInfo.mfaLastMethodUsed` and loops on `INVALID_MFA_CODE`/`MFA_FAILED`/`INVALID` (max 3 attempts) via `/mobile/api/mfa/verifyCode` (exact JSON `{"mfaMethod","mfaVerificationCode","rememberMyBrowser":true,"reconsentList":[],"mfaSetup":false}`) → ticket exchanged at `POST /di-oauth2-service/oauth/token` with Basic auth `base64(client_id:)` and form `client_id`, `service_ticket`, `grant_type`, `service_url`.
- **Client-id retry:** the three candidates are tried in order until one returns an `access_token` (non-2xx and tokenless-200 responses both advance the walk); a `429` aborts the walk with a rate-limit message; the winning id is persisted in `tokens.json`.
- **Native headers:** `User-Agent: GCM-Android-5.23`, `X-Garmin-User-Agent`, `X-Garmin-Paired-App-Version: 10861`, `X-Garmin-Client-Platform: Android`, `X-App-Ver: 10861`, `X-Lang: en`, `X-GCExperience: GC5`, `Accept: application/json`, `Cache-Control: no-cache` on both the exchange and the refresh.
- **Refresh:** `garmin::refresh` builds the `grant_type=refresh_token` form (same Basic auth + native headers); wired into sync in ticket 08.
- **Persistence:** `auth` now collects both halves and writes `tokens.json` once at the end — a failure in either half leaves no partial token file (the Withings tests were updated accordingly).
- **429 handling:** both HTTP-level 429 and a JSON `error.status-code` of `"429"` or `429` on the SSO calls surface as a clear rate-limit error (backoff retry is ticket 08's job).
- **Test infrastructure:** `tests/common/mod.rs` gained a routing fake HTTP server with per-route call counting (used for MFA loops and client-id retries). Two real bugs were fixed while stabilizing it: (1) the accept loop must survive `ECONNABORTED`/`ECONNRESET` under parallel load, and (2) on macOS, sockets accepted from a nonblocking listener inherit the nonblocking flag and must be set back to blocking mode, or reads EAGAIN and connections reset mid-request.

- [ ] `auth` performs the ordered sequence: sign-in page GET (cookie jar) → login POST →, when `MFA_REQUIRED`, an interactive MFA verify POST → service-ticket exchange at the DI token endpoint → persisted access + refresh tokens.
- [ ] Garmin username and password are prompted for interactively, not read from files; an MFA code is prompted for and loops on an invalid code.
- [ ] The three DI client ids are tried in order and the winning client id is persisted alongside the tokens.
- [ ] A refresh path (`grant_type=refresh_token`) is implemented.
- [ ] Tests assert, against fake SSO and diauth servers: the ordered sequence, the native Android header set, the MFA branch, client-id retry, and token persistence.

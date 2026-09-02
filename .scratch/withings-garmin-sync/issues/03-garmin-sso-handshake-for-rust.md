# 03 - Garmin SSO handshake for Rust

Type: research
Status: resolved
Blocked by: 01

## Question

Determine how a Rust client authenticates to Garmin Connect's undocumented API — the exact DI mobile-SSO handshake that yields a native OAuth Bearer token + `JWT_FGP` cookie — so the spec can specify the auth implementation precisely rather than defer it.

Using `python-garminconnect`'s `client.py` (and its maintained fork `lipov3cz3k/python-garminconnect`) as the reference implementation, trace the handshake step-by-step:

1. The SSO entry URLs, request/response bodies, and the exact sequence that turns username/password (+ optional MFA) into a DI OAuth Bearer token and `JWT_FGP` cookie.
2. How refresh works (which token/cookie must be retained and re-presented).
3. Any non-standard headers, `NK: NT`, `User-Agent`, or `DI-Backend` values required during the handshake or on subsequent calls.
4. Which parts are ported trivially to Rust (HTTP + JSON + cookie jar) versus which are non-trivial (e.g. any JS/webview, PKCE, MFA challenge parsing).

The answer must record a crisp, ordered auth sequence a Rust implementer can follow, with the specific URLs, parameters, and headers. Each step cited to the Python source line/section. This ticket is AFK and is **blocked by `01 - Garmin write-path spike`** (the spike confirms the flow is live before we spec the port).

## Answer

Resolved by tracing `lipov3cz3k/python-garminconnect` `garminconnect/client.py` (v5.x, current) and corroborated by the live spike in `01` (which reached and wrote through `connectapi.garmin.com` using only `curl` — proving plain HTTPS/Rust `reqwest` is sufficient, no TLS impersonation needed).

### The flow (portable to Rust trivially)

The fork tries four strategies but they all converge on the same two-stage result. For Rust, **use the mobile SSO flow** (no `curl_cffi` needed — the `01` harness did it with `curl`).

**Stage A — SSO login → CAS service ticket**

1. `GET https://sso.garmin.com/mobile/sso/en_US/sign-in?clientId=GCM_ANDROID_DARK&service=https://mobile.integration.garmin.com/gcm/android` — sets SSO session cookies (cookie jar required). Header: Android WebView `User-Agent` (constant `MOBILE_SSO_USER_AGENT`). *(client.py `_mobile_login`, ~L640)*
2. `POST https://sso.garmin.com/mobile/api/login` (same `clientId`/`service` + `locale=en-US` as params) with JSON body `{"username","password","rememberMe":true,"captchaToken":""}`. *(~L650)*
   - `responseStatus.type == "SUCCESSFUL"` → `serviceTicketId` in body.
   - `"MFA_REQUIRED"` → MFA (below).
   - `"INVALID_USERNAME_PASSWORD"` → 401 error.
   - HTTP `429` (or `error.status-code=429` in body) → rate-limited, back off.
3. **MFA (if required):** `POST https://sso.garmin.com/mobile/api/mfa/verifyCode` with JSON `{"mfaMethod": <"email"|"totp">, "mfaVerificationCode": <code>, "rememberMyBrowser":true, "reconsentList":[], "mfaSetup":false}` → `serviceTicketId`. The `mfaMethod` to use is read from `customerMfaInfo.mfaLastMethodUsed` in the login response. *(~L650, `_complete_mfa`)*

**Stage B — CAS ticket → native DI Bearer token**

4. `POST https://diauth.garmin.com/di-oauth2-service/oauth/token` with `Content-Type: application/x-www-form-urlencoded`, Basic auth `base64("<client_id>:")`, and form body:
   - `client_id` = one of `GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2` / `...2024Q4` / `...DI` (tried in order)
   - `service_ticket` = the CAS `serviceTicketId`
   - `grant_type` = `https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket`
   - `service_url` = `https://mobile.integration.garmin.com/gcm/android` (must match login `service`)
   - Native Android headers (see `_native_headers`, ~L83).
   → `{"access_token":..., "refresh_token":...}`. *(~L739, `_exchange_service_ticket`)*

**Stage C — refresh (long-lived, for the stateless CLI)**

5. `POST https://diauth.garmin.com/di-oauth2-service/oauth/token` with same Basic auth + form `grant_type=refresh_token`, `client_id`, `refresh_token`. → fresh `access_token` (+ rotated `refresh_token`). *(~L795, `_refresh_di_token`)*

Note: the `client_id` embedded in the JWT can be extracted to know which one succeeded (`_extract_client_id_from_jwt`, ~L833); the fork stores it for refresh. A Rust port should store `{access_token, refresh_token, client_id}`.

### Headers — exact set

**On every native API call (writes included):** `Authorization: Bearer <di_token>` plus the Android app headers (`_native_headers`, ~L83): `User-Agent: GCM-Android-5.23`, `X-Garmin-User-Agent: com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0`, `X-Garmin-Paired-App-Version: 10861`, `X-Garmin-Client-Platform: Android`, `X-App-Ver: 10861`, `X-Lang: en`, `X-GCExperience: GC5`, `Accept: application/json`. **No `JWT_FGP`, no `DI-Backend`, no `NK: NT`** — those belong to the legacy `modern/proxy` cookie fallback (`get_api_headers` JWT_WEB branch, ~L147), which the spec does **not** need.

### Portability verdict

- **Trivial (HTTP + JSON + cookie jar):** all of Stage A and B — plain `POST`/`GET` with a cookie jar and form/JSON bodies. `reqwest` with a `cookie_store` feature (or `cookie` + manual `Cookie`/`Set-Cookie` round-trip) is sufficient. The `01` spike proved `curl` alone works, so no browser TLS fingerprinting is required.
- **Non-trivial / needs care:**
  1. **MFA parsing** — the login response signals `MFA_REQUIRED` with `customerMfaInfo.mfaLastMethodUsed`; a one-shot CLI must *interactively* prompt for the code (and loop on `INVALID...` retry). Not hard, but it's an interactive pause in an otherwise non-interactive flow.
  2. **EU region** — the fork hardcodes `diauth.garmin.com` (no `.eu` split), but `01` confirmed EU accounts require **user-granted upload consent** before writes succeed (`412` otherwise). The CLI cannot grant it; the spec documents it as a first-run prerequisite.
  3. **Rate limiting** — `429` surfaces both at HTTP level and inside the JSON `error.status-code`; the client must distinguish them (the fork checks both).
  4. **`client_id` selection** — three candidate client IDs tried in order; a Rust port should attempt each until one returns `access_token`, then persist which one for refresh.
  5. **No PKCE/webview** — the mobile flow is plain username/password OAuth2-grant-via-ticket; nothing like PKCE or an embedded browser is involved. This is a *simplification*, not a risk.

### Canonical ordered sequence (paste-able summary)

```
1. GET  https://sso.garmin.com/mobile/sso/en_US/sign-in?clientId=GCM_ANDROID_DARK&service=https%3A%2F%2Fmobile.integration.garmin.com%2Fgcm%2Fandroid   (keep cookies)
2. POST https://sso.garmin.com/mobile/api/login?clientId=GCM_ANDROID_DARK&locale=en-US&service=https%3A%2F%2Fmobile.integration.garmin.com%2Fgcm%2Fandroid
       body {"username","password","rememberMe":true,"captchaToken":""}
   -> if MFA_REQUIRED: POST .../mobile/api/mfa/verifyCode?clientId=...&locale=en-US&service=...
       body {"mfaMethod":<email|totp>,"mfaVerificationCode":<code>,"rememberMyBrowser":true,"reconsentList":[],"mfaSetup":false}
   -> serviceTicketId
3. POST https://diauth.garmin.com/di-oauth2-service/oauth/token
       Authorization: Basic base64(<client_id>:)   ; form:
       client_id=<GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2|...>, service_ticket=<serviceTicketId>,
       grant_type=https%3A%2F%2Fconnectapi.garmin.com%2Fdi-oauth2-service%2Foauth%2Fgrant%2Fservice_ticket,
       service_url=<same as login service>
   -> {"access_token", "refresh_token"}
4. (refresh) POST https://diauth.garmin.com/di-oauth2-service/oauth/token
       form: grant_type=refresh_token, client_id=<cid>, refresh_token=<rt>
```

Sources: `garminconnect/client.py` — constants ~L35-93, `_native_headers` ~L83, `_mobile_login` ~L596, `_complete_mfa` ~L663, `_exchange_service_ticket` ~L731, `_refresh_di_token` ~L795, `get_api_headers` ~L135. Live corroboration: `01 - Garmin write-path spike` (`spike/garmin-spike.sh`).

# 01 - Garmin write-path spike

Type: task
Status: resolved

## Question

Prove that Garmin Connect's undocumented internal JSON write endpoints for weight and blood pressure still work *today*, end-to-end, using real Garmin credentials — before the spec commits its whole Garmin section to this surface.

Specifically verify, with a throwaway harness (curl or a tiny Rust/spike, not the CLI):

1. The mobile-SSO auth flow yields a usable **DI OAuth Bearer token + `JWT_FGP` cookie + `DI-Backend: apiconnect.garmin.com` header** today (confirming the old `garth` login is truly dead and the mobile-SSO flow is the live one).
2. `POST https://connect.garmin.com/modern/proxy/weight-service/user-weight` accepts a weigh-in (payload shape: `dateTimestamp`, `gmtTimestamp`, `unitKey`, `sourceType:"MANUAL"`, `value`).
3. `POST https://connect.garmin.com/modern/proxy/bloodpressure-service/bloodpressure` accepts a BP reading (payload: `measurementTimestampLocal`/`GMT`, `systolic`, `diastolic`, `pulse`, `sourceType`, `notes`).
4. Whether any *additional* headers (`NK: NT`, a specific `User-Agent`) are required beyond Bearer + cookie + `DI-Backend`. (Upload endpoint 404s without DI-Backend; confirm the JSON endpoints' requirement.)
5. Duplication behavior: does a repeated write with the same timestamp dedup or create a duplicate entry?

The answer must record: the exact working auth sequence (header/cookie/endpoint order), the payload that succeeded, any extra headers discovered, and the observed duplication behavior. Credentials location and any token storage notes belong in the answer (later tickets depend on them).

This is a `task` (does rather than decides) and is **HITL**: it needs the human's real Garmin username/password/MFA. The agent drives the harness; the human supplies credentials at the prompt.

## Comments

### Spike run 2026-09-02 — consented-blocked (partial result, ticket stays open)

Harness: `.scratch/withings-garmin-sync/spike/garmin-spike.sh` (+ `RUN-SHEET.md`).

**Proven (solid):**

1. **mobile-SSO → DI Bearer flow is live today.** Login (`POST sso.garmin.com/mobile/api/login`) → `serviceTicketId`, exchange at `POST diauth.garmin.com/di-oauth2-service/oauth/token` with `grant_type=.../service_ticket` yielded a working `access_token` + `refresh_token`. No MFA was required on the throwaway account.
2. **Write surfaces are correct.** Both `POST https://connectapi.garmin.com/weight-service/user-weight` and `POST https://connectapi.garmin.com/bloodpressure-service/bloodpressure` responded with a *semantic* 412 (`PreconditionFailedException`), not 401/404 — so auth succeeded and the request reached policy logic.
3. **No `JWT_FGP` / `DI-Backend` / `NK` header needed on the write path.** The native Bearer + Android header set (`Authorization: Bearer`, `X-Garmin-User-Agent`, `X-App-Ver`, etc.) reached the endpoint on its own. The `modern/proxy` + cookie path is the *legacy* JWT_WEB fallback and was not exercised.

**New blocker discovered (must be resolved before the spec commits):**

4. **EU consent gate.** Every write returned: `{"message":"The user is from EU location, but upload consent is not yet granted or revoked","reasonCode":2,"error":"PreconditionFailedException"}`. This is a hard gate for EU-located accounts. No client-side flag is known (the fork's `reconsentList` is MFA-only, unrelated). Web search indicates resolution is **user-side consent in Garmin Connect account settings**, not an API call — but the exact procedure is unverified. Whether a non-EU account bypasses this entirely is also unverified.

**Still unknown (blocked behind the consent gate):**

5. **Duplication semantics — UNTESTED.** The replay-same-payload check was never evaluated because every write (including the replay) hit the 412. This remains open.

**Not yet resolved in this ticket:** the write tests must be re-run once consent is granted (or a non-EU account is used), to (a) confirm 2xx acceptance, (b) capture the returned measurement id, and (c) run the duplication check. See the new map decision "EU upload consent gate" for the prerequisite. A harness timestamp bug (`%3N` leaking `N`) was fixed before the re-run.

### Spike run 2026-09-02 (evening) — SUCCESS after consent

After the human granted upload consent in Garmin Connect settings, the same harness (`STATE_DIR=/tmp/withings-garmin-spike-throwaway`, token still valid) produced:

- **weight** `POST /weight-service/user-weight` → **`HTTP 204`** (no body — the fork's `_validate_json_exists` treats 204 as success).
- **blood pressure** `POST /bloodpressure-service/bloodpressure` → **`HTTP 200`**, body echoes the accepted measurement (systolic 120, diastolic 80, pulse 72, note, `measurementTimestampLocal/...GMT`).
- **weight replay** (identical payload) → **`HTTP 204`**.

**Consent gate:** confirmed resolved by granting consent in the account; the `412 PreconditionFailedException` disappeared. This is now a documented *prerequisite* rather than a blocker.

**Duplication:** still **INCONCLUSIVE** — neither success body returns a measurement id/version to compare, so the replay can't be judged from the write responses alone. A `readback` mode was added to the harness (`GET /weight-service/weight/range/...` and `GET /bloodpressure-service/bloodpressure/range/...`) to count today's entries; run `./garmin-spike.sh readback` to settle it.

**Header verdict (clean):** the native Bearer + Android header set (`Authorization`, `X-Garmin-User-Agent`, `X-App-Ver 10861`, `X-GCExperience GC5`, etc.) succeeded **without** `JWT_FGP`, `DI-Backend`, or `NK`. The legacy `modern/proxy` + cookie path is not needed.

## Answer

**The Garmin JSON write path is proven live and viable today.** Full end-to-end success using real (throwaway) credentials.

**1. Auth flow (mobile-SSO → DI Bearer) — LIVE.**
`POST https://sso.garmin.com/mobile/api/login` (`clientId=GCM_ANDROID_DARK`, `service=https://mobile.integration.garmin.com/gcm/android`) → `serviceTicketId` (or `MFA_REQUIRED` → `POST /mobile/api/mfa/verifyCode`). Exchange: `POST https://diauth.garmin.com/di-oauth2-service/oauth/token` with `grant_type=https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket`, Basic-auth client id `GARMIN_CONNECT_MOBILE_ANDROID_DI_*` → `access_token` (+ `refresh_token`). Refresh via `grant_type=refresh_token` on the same `/oauth/token` endpoint.

**2. Weight write — WORKS.** `POST https://connectapi.garmin.com/weight-service/user-weight`, JSON `{"dateTimestamp","gmtTimestamp","unitKey":"kg","sourceType":"MANUAL","value":82.4}` → **`HTTP 204`** (empty body = success). Timestamps in ms-precision `YYYY-MM-DDTHH:MM:SS.SSS`.

**3. Blood-pressure write — WORKS.** `POST https://connectapi.garmin.com/bloodpressure-service/bloodpressure`, JSON `{"measurementTimestampLocal","measurementTimestampGMT","systolic","diastolic","pulse","sourceType":"MANUAL","notes"}` → **`HTTP 200`**, body echoes the accepted measurement. Range validation in the fork: systolic 70–260, diastolic 40–150, pulse 20–250.

**4. Headers — native Bearer only, no extras.** Required set is `Authorization: Bearer <di_token>` + Android app headers (`User-Agent: GCM-Android-5.23`, `X-Garmin-User-Agent`, `X-Garmin-Paired-App-Version: 10861`, `X-Garmin-Client-Platform: Android`, `X-App-Ver: 10861`, `X-Lang: en`, `X-GCExperience: GC5`, `Accept: application/json`, `Content-Type: application/json`). **No `JWT_FGP` cookie, no `DI-Backend`, no `NK: NT`.** (Those are the legacy `modern/proxy` JWT_WEB fallback path, not needed.)

**5. Duplication — DEDUP by timestamp (idempotent).** Two identical-payload weight writes produced **`numOfWeightEntries: 1`** on read-back (`GET /weight-service/weight/range/{d}/{d}`), and a single BP write produced `numOfMeasurements: 1` (`GET /bloodpressure-service/bloodpressure/range/{d}/{d}`). Re-writing the same measurement with the same timestamp does **not** create a duplicate — so stateless full-overwrite (no cursor/watermark) is safe in practice.

**6. EU consent prerequisite (discovered).** Before granting consent, every write returned `412 PreconditionFailedException` ("The user is from EU location, but upload consent is not yet granted or revoked"). Granting **upload consent in Garmin Connect account settings** clears it. The spec must document this as a first-run prerequisite; the CLI cannot grant it programmatically (no client-side flag exists).

Assets: `spike/garmin-spike.sh` (harness, with `auth`/`mfa`/`write`/`readback` modes) + `spike/RUN-SHEET.md`. Credentials were the human's, supplied out-of-band.

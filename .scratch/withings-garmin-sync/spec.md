# Spec: withings-garmin-sync

Status: ready-for-agent

## Problem Statement

A person who tracks **body weight** (Withings smart scale) and **blood pressure** (Withings BP monitor) lives on Linux and uses Garmin Connect as their health hub. Garmin has no public write API for these metrics, and there is no maintained tool that copies both metrics from Withings into Garmin Connect as a simple, one-shot CLI. Today the person either re-enters each reading by hand, lives with the data split across two apps, or runs fragile Python scripts. They want their Withings weight and blood-pressure history to land in Garmin Connect, preserving each measurement's original timestamp, without babysitting a daemon.

## Solution

A **one-shot, stateless Rust CLI for Linux** (`withings-garmin-sync`) that reads body weight and blood pressure from the Withings API and writes them to Garmin Connect through Garmin's undocumented internal JSON write endpoints (no FIT encoding, no body-composition metrics). After a one-time interactive `auth` step, every subsequent `sync` is non-interactive and safe to re-run: Garmin deduplicates writes by timestamp, so the CLI can do a stateless **full overwrite** of the sync window on each run with no cursor or watermark.

Credentials and settings live in plaintext under `~/.config/withings-garmin-sync/` (no Keychain). The sync **defaults to `--dry-run`** and only writes when the operator passes `--apply`. Weight and blood pressure are synced as **independent metrics**: one failing does not block the other, and the exit code reports partial success. Withings measurement timestamps are carried through to Garmin unchanged (converted to the format Garmin expects).

## User Stories

1. As a Linux user, I want a single command that syncs my Withings body weight into Garmin Connect, so that my weight history lives in one place without manual entry.
2. As a Linux user, I want a single command that syncs my Withings blood pressure into Garmin Connect, so that my BP readings appear alongside my Garmin health data.
3. As a first-time user, I want a guided one-time `auth` command that connects both Withings and Garmin, so that later syncs need no interaction.
4. As a first-time user, I want the CLI to print the Withings authorization URL and accept the redirect code I paste back, so that I can authorize without the CLI driving a browser.
5. As a first-time user, I want the CLI to prompt for my Garmin username and password rather than read them from a file, so that I never have to embed my password in config.
6. As a user with Garmin two-factor authentication, I want the CLI to pause and prompt for my MFA code, so that my protected account still works.
7. As a user, I want the CLI to store Withings and Garmin refresh tokens, so that I don't re-authenticate on every run.
8. As a returning user, I want the CLI to refresh an expired Withings access token automatically, so that gaps between runs don't force re-authorization.
9. As a returning user, I want the CLI to refresh the Garmin access token automatically when a write is rejected as unauthorized, so that syncs keep working without me.
10. As a cautious user, I want sync to default to `--dry-run`, so that nothing is written to Garmin until I explicitly approve it.
11. As a user, I want `--apply` to actually write to Garmin, so that an approved sync takes effect.
12. As a user, I want a dry-run to show exactly what would be written (metric, value, timestamp), so that I can review before applying.
13. As a user, I want each weight write to carry the original Withings measurement timestamp, so that Garmin shows the true weigh-in time, not the sync time.
14. As a user, I want each blood-pressure write to carry the original Withings measurement timestamp, so that my BP history is chronologically accurate.
15. As a user, I want re-running the sync not to duplicate entries, so that I can run it on a schedule without fear.
16. As a user, I want weight and blood-pressure sync to be independent, so that a failure in one does not block the other.
17. As a user, I want a clear exit code and summary that distinguish full success, partial success, and failure, so that I can react appropriately.
18. As a user, I want weight and blood pressure synced together in a single run, so that one command refreshes both metrics.
19. As a user, I want `--since`/`--until` flags to bound the sync window, so that I can do targeted backfills.
20. As a user, I want config and tokens stored as plaintext under `~/.config/withings-garmin-sync/`, so that I can inspect and manage them on Linux.
21. As a security-conscious user, I want config and token files written with `0600` permissions, so that other local users can't read my tokens.
22. As a user in the EU, I want the CLI to tell me clearly that Garmin requires "upload consent" to be granted in account settings, so that I understand why writes fail with `412`.
23. As a user, I want a specific, actionable error when Garmin returns the `412` consent-gate, so that I can resolve it myself rather than guess.
24. As a user, I want the CLI to respect Withings rate limiting (status `601`), so that a busy minute doesn't crash the sync.
25. As a user, I want the CLI to retry Garmin `429` responses with backoff, so that transient throttling doesn't fail the whole run.
26. As a user, I want a verbose mode that logs each API request and response, so that I can debug failures myself.
27. As a user, I want the CLI to follow Withings pagination automatically, so that I don't miss measurements beyond the first page.
28. As a user, I want systolic and diastolic values paired from the same Withings reading, so that Garmin stores valid, internally-consistent BP records.
29. As a user, I want the pulse (heart rate) carried through when Withings provides it, so that Garmin BP records are complete.
30. As a user, I want out-of-range values (per Garmin's validation) skipped with a warning, so that one bad Withings record doesn't abort the whole sync.
31. As a user, I want weight sent to Garmin in kilograms, so that the value matches Garmin's expected unit.
32. As a user, I want timestamps formatted to millisecond precision exactly as Garmin expects, so that writes are accepted.
33. As a user, I want local timestamps derived from my Linux system timezone, so that Garmin shows correct local times.
34. As an implementer, I want every external host base URL overridable via environment variables, so that the entire CLI is testable against local fake servers.
35. As a user, I want a `--config-dir` override, so that I can run the CLI with an alternate config for testing or CI.
36. As a user, I want the CLI to fail fast with a clear "run `auth` first" message when config/tokens are missing, so that I know the required setup step.
37. As a user, I want a final report of measurements written vs. skipped vs. failed, so that I can verify the outcome at a glance.
38. As a user, I want the sync to be non-interactive (after first-run auth), so that I can call it from cron, launchd, or scripts.
39. As a user, I want `--apply` to be safe to re-run, relying on Garmin's timestamp deduplication, so that overlapping windows never double-count.
40. As a user, I want the Garmin DI client id to be selected automatically (trying candidates in order and remembering the winner), so that I never have to know Garmin's internal client ids.
41. As a user, I want the Withings OAuth scope limited to `user.metrics`, so that the CLI requests only the permission it actually needs.
42. As a user, I want a native Rust binary installed via `cargo install`, so that there is no Python/Node runtime to manage.
43. As a user, I want a human-readable one-line summary on completion (success/partial/failure + counts), so that I can trust the result at a glance.

## Implementation Decisions

### Language, platform, and packaging

- Rust, compiled to a single native Linux binary. Distribution is `cargo install` only.
- The CLI is **one-shot and synchronous**: `auth` and `sync` subcommands run to completion and exit; no daemon, no scheduling, no TUI.
- Binary name is `withings-garmin-sync`.

### CLI surface

- Two subcommands:
  - `auth` — interactive, first-run only. Connects Withings (OAuth authorization code flow) and Garmin (mobile-SSO username/password, optional MFA), then persists tokens.
  - `sync` — non-interactive. Reads the window from Withings, transforms, and (with `--apply`) writes to Garmin.
- `sync` flags:
  - `--dry-run` (default): authenticate, read, transform, and print a would-write report; perform **no** writes.
  - `--apply`: perform the writes.
  - `--since <date>`, `--until <date>`: bound the sync window (ISO `YYYY-MM-DD`). Default window when omitted is the last 30 days; `--since` without `--until` means "since … until now".
  - `--config-dir <path>`: override the config directory (default `~/.config/withings-garmin-sync`).
  - `--verbose`: log each request/response.
- `auth` may also accept `--config-dir`.

### Exit codes

- `0` — success (weight and blood pressure both synced, or a dry-run completed cleanly).
- `1` — partial or total metric failure (at least one metric failed to sync; the report says which).
- `2` — usage error (unknown flag/subcommand).
- `3` — config error (missing/invalid config or tokens; instructs to run `auth`).
- `4` — auth error (token refresh rejected; re-authentication required).

Per-metric independence means exit `1` is used for any mix of success and failure across metrics; the human-readable summary and report carry the detail.

### Configuration and token storage

- One directory: `~/.config/withings-garmin-sync/` (overridable). Plaintext by decision (no Keychain).
- Two files, both written with `0600` permissions:
  - `config.toml` — static settings (Withings OAuth client id/secret, sync defaults).
  - `tokens.json` — secrets (Withings + Garmin tokens), so tokens can rotate independently of settings.

Schema (shape only, from the design of the storage, trimmed to the decision-rich parts):

```toml
# config.toml
[withings]
client_id = "…"          # from the Withings developer portal
client_secret = "…"

[sync]
since = "2026-01-01"     # optional; omit for the default 30-day window
```

```jsonc
// tokens.json
{
  "withings": { "access_token": "…", "refresh_token": "…", "expires_at": "…" },
  "garmin":   { "access_token": "…", "refresh_token": "…", "client_id": "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2" }
}
```

### The single seam: injectable HTTP base URLs

All external hosts are overridable via environment variables (defaults point at the real endpoints). This is the **one test seam**: tests point the CLI at local fake HTTP servers.

- `WGS_WITHINGS_API_BASE` — default `https://wbsapi.withings.net` (covers `/measure` and `/v2/oauth2`).
- `WGS_GARMIN_SSO_BASE` — default `https://sso.garmin.com`.
- `WGS_GARMIN_DIAUTH_BASE` — default `https://diauth.garmin.com`.
- `WGS_GARMIN_API_BASE` — default `https://connectapi.garmin.com`.

### Withings client (read-only)

- **OAuth2 token endpoint:** `POST {WGS_WITHINGS_API_BASE}/v2/oauth2`, form-encoded, `action=requesttoken`; `grant_type` selects the operation: `authorization_code` to exchange a code, `refresh_token` to refresh. There is no separate `refreshtoken` action.
- **Authorize URL** (printed for the user to open in a browser, then the redirect code is pasted back): `GET https://account.withings.com/oauth2_user/authorize2?response_type=code&…`.
- **Scope:** only `user.metrics`. Token lifetimes: access ~3h, refresh ~1y.
- **Measurements endpoint:** `POST {WGS_WITHINGS_API_BASE}/measure` with `action=getmeas` (the unversioned `/measure`, not `/v2/measure`). `action` and params are sent in the form-urlencoded POST body. Optional `meastypes` filter `1,9,10,11`; optional `startdate`/`enddate` (epoch seconds) for the window.
- **Pagination:** follow the response's `more`/`offset` fields until no more pages.
- **Measure codes and scaling:** `type` (= meastype) with `value` and `unit`; decode `real = value * 10^unit`.
  - weight = `1` (kg), systolic BP = `10` (mmHg), diastolic BP = `9` (mmHg), heart pulse = `11` (bpm).
- **Rate limit:** 120 requests/minute; a `601` status means rate-limited — back off and retry.

### Garmin client (auth + write)

**Auth — mobile-SSO → DI Bearer (ordered sequence):**

1. `GET {WGS_GARMIN_SSO_BASE}/mobile/sso/en_US/sign-in?clientId=GCM_ANDROID_DARK&service=<url-encoded https://mobile.integration.garmin.com/gcm/android>` — establishes SSO session cookies (cookie jar required).
2. `POST {WGS_GARMIN_SSO_BASE}/mobile/api/login` with JSON `{"username","password","rememberMe":true,"captchaToken":""}`.
   - `SUCCESSFUL` → a `serviceTicketId`.
   - `MFA_REQUIRED` → interactive prompt for the code, then `POST {WGS_GARMIN_SSO_BASE}/mobile/api/mfa/verifyCode` with `{"mfaMethod":<email|totp>,"mfaVerificationCode":…,"rememberMyBrowser":true,"reconsentList":[],"mfaSetup":false}`. `mfaMethod` is read from `customerMfaInfo.mfaLastMethodUsed` in the login response. Loop on invalid codes.
   - `INVALID_USERNAME_PASSWORD` → fail with a clear message.
3. `POST {WGS_GARMIN_DIAUTH_BASE}/di-oauth2-service/oauth/token`, form-encoded, Basic-auth header `base64("<client_id>:")`, with form fields `client_id`, `service_ticket=<serviceTicketId>`, `grant_type=https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket`, `service_url=https://mobile.integration.garmin.com/gcm/android`. Returns `access_token` + `refresh_token`.
   - **Client id selection:** try the candidates in order — `GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2`, `GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4`, `GARMIN_CONNECT_MOBILE_ANDROID_DI` — until one returns an `access_token`; persist the winning id.
4. **Refresh:** `POST {WGS_GARMIN_DIAUTH_BASE}/di-oauth2-service/oauth/token` with `grant_type=refresh_token`, `client_id`, `refresh_token`. Refresh lazily when a write returns `401`.

**Native API headers — sent on every write (and refresh):** `Authorization: Bearer <di_token>` plus the Android app header set:

```
User-Agent: GCM-Android-5.23
X-Garmin-User-Agent: com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0
X-Garmin-Paired-App-Version: 10861
X-Garmin-Client-Platform: Android
X-App-Ver: 10861
X-Lang: en
X-GCExperience: GC5
Accept: application/json
Content-Type: application/json   (writes only)
```

No `JWT_FGP` cookie, no `DI-Backend`, no `NK: NT` — those belong to a legacy fallback path this CLI does not use.

**Write payloads** (shapes confirmed live in the spike prototype):

```jsonc
// POST {WGS_GARMIN_API_BASE}/weight-service/user-weight  → expect HTTP 204 (empty body)
{
  "dateTimestamp": "YYYY-MM-DDTHH:MM:SS.SSS",  // local time
  "gmtTimestamp":  "YYYY-MM-DDTHH:MM:SS.SSS",  // UTC
  "unitKey": "kg",
  "sourceType": "MANUAL",
  "value": 82.4
}
```

```jsonc
// POST {WGS_GARMIN_API_BASE}/bloodpressure-service/bloodpressure  → expect HTTP 200 (echo body)
{
  "measurementTimestampLocal": "YYYY-MM-DDTHH:MM:SS.SSS",
  "measurementTimestampGMT":  "YYYY-MM-DDTHH:MM:SS.SSS",
  "systolic": 120,
  "diastolic": 80,
  "pulse": 72,           // omitted when Withings has no pulse value
  "sourceType": "MANUAL",
  "notes": ""            // omitted when empty
}
```

- Timestamps are millisecond-precision `YYYY-MM-DDTHH:MM:SS.SSS`; local variants use the Linux system timezone, GMT variants use UTC.
- Garmin range validation: systolic 70–260, diastolic 40–150, pulse 20–250. Out-of-range readings are **skipped with a warning**, not aborted.
- `sourceType` is always `"MANUAL"`.

### Transform (Withings → Garmin)

- Weight: decode the `type=1` value to kilograms (`value * 10^unit`), emit a weight payload with that kg value and the measurement's own timestamp.
- Blood pressure: group measures by their parent measure group; within a group collect systolic (`10`), diastolic (`9`), and pulse (`11`). Emit a BP payload when systolic and diastolic are both present; include pulse only when present.
- Timestamps: Withings `date` is epoch seconds (UTC). Convert to the Garmin ISO-ms formats, deriving the local component from the system timezone.

### Sync orchestration

- Read the window from Withings once, transform into two independent lists (weight, blood-pressure).
- Dry-run: print a would-write line per measurement; write nothing.
- Apply: write each list independently; a metric's failure does not abort the other. On completion, print counts of written / skipped / failed per metric and set the exit code per the table above.
- Idempotency: the CLI performs a full overwrite of the window on every `--apply` run, relying on Garmin's timestamp deduplication (verified in the spike: two identical writes → one entry). No cursor/watermark is stored.

### Error handling

- Withings `601` and Garmin `429` (which may appear as HTTP status **or** inside a JSON `error.status-code`) are distinguished and retried with backoff.
- Garmin `412` with the EU upload-consent message is surfaced as a specific, actionable error: the user must grant "upload consent" in Garmin Connect account settings. The CLI cannot grant it programmatically.
- Missing config/tokens → exit `3` with "run `auth` first".
- A rejected token refresh → exit `4` with "re-run `auth`".

## Testing Decisions

### What makes a good test

A good test observes **external behavior only**, through the single HTTP seam — never internal function signatures, module layout, or private fields. Given a fake Withings response and a fake Garmin auth/write sequence, the test asserts on: (a) the requests each fake server received (method, path, headers, body), (b) the CLI's stdout/report, (c) its exit code, and (d) the files it wrote under the config dir. If a test would break because a module was renamed or a helper was inlined, it is testing the wrong thing.

### The seam under test

The **injectable HTTP base URLs** (`WGS_WITHINGS_API_BASE`, `WGS_GARMIN_SSO_BASE`, `WGS_GARMIN_DIAUTH_BASE`, `WGS_GARMIN_API_BASE`). Tests run the compiled binary (or the library entrypoint that backs it) against in-process local HTTP test servers bound to those variables, pointed at a temp `--config-dir`. This is the only seam; there is no second, in-process seam for the sync engine.

### Modules exercised (through the seam)

CLI parsing and exit codes; config/token load and persistence (including `0600` permissions); Withings auth/token refresh and `getmeas` read + pagination; the weight/BP transform (timestamp conversion, kg decode, systolic/diastolic pairing, range skipping); Garmin mobile-SSO auth incl. MFA and client-id retry; Garmin token refresh; the two write endpoints; dry-run vs. apply; per-metric independence; retry/backoff on `601`/`429`; the EU `412` consent-gate message.

### Prior art

The codebase has no Rust tests yet (greenfield). The closest prior art is the throwaway spike harness `garmin-spike.sh` (bash + curl against the real endpoints) and its `RUN-SHEET.md`, which established the request/response shapes the tests will now assert against fake servers. Test fixtures should mirror the payloads and status codes captured there (`204` weight, `200` BP echo, `412` consent-gate, dedup-by-timestamp).

## Out of Scope

- **Body-composition metrics** (fat %, muscle mass, bone mass, hydration, BMI): the chosen JSON endpoints cannot carry them.
- **FIT-file encoding / upload** (`/upload-service/upload`): deliberately not adopted.
- **Bi-directional sync** (Garmin → Withings).
- **Other Withings metrics** (sleep, activity, heart-rate series, ECG, SpO₂).
- **Multi-account / multi-device** support.
- **GUI or menu-bar app.**
- **Packaging/distribution** beyond `cargo install`.
- **Scheduling** (cron/launchd/daemon): one-shot only; scheduling is a post-handoff concern.

## Further Notes

- **Undocumented API fragility:** Garmin Connect's JSON write endpoints and the mobile-SSO handshake are undocumented and may change without notice. The spec pins today's observed surface (verified live in `01 - Garmin write-path spike`), but implementers and users should treat it as best-effort.
- **EU upload consent:** before the first `--apply`, an EU-located Garmin account must grant "upload consent" in Garmin Connect account settings or every write returns `412`. This is a documented first-run prerequisite, not something the CLI can automate.
- **Reference implementations consulted:** `jaroslawhartman/withings-sync` (Python; Withings-side read/auth reference only — its FIT-upload route is ruled out) and `python-garminconnect` / its maintained fork `lipov3cz3k/python-garminconnect` (source of the JSON endpoint payloads and the mobile-SSO handshake traced in `03 - Garmin SSO handshake for Rust`).
- **Credentials hygiene:** tokens (especially the Garmin refresh token) are long-lived secrets. They are stored plaintext by decision, so file permissions (`0600`) are the sole protection; document this clearly to the user.

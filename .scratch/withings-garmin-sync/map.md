# Map: withings-garmin-sync

## Destination

A written **spec** (published to `.scratch/withings-garmin-sync/spec.md`) for a Rust CLI that runs on macOS and syncs **body weight and blood pressure** from Withings to Garmin Connect, one-shot and stateless. The spec targets Garmin Connect's **undocumented internal JSON write endpoints** (`/weight-service/user-weight` and `/bloodpressure-service/bloodpressure`) — no FIT encoder, no body-composition metrics — and fully specifies OAuth/token handling so an implementer can build from the spec with no decisions left unmade. Reaching the destination means the spec is written and handed off; the map does not build the CLI.

## Notes

- **Domain:** Rust, macOS, Withings API (public OAuth2 read API), Garmin Connect undocumented write API (mobile-SSO auth, no public write API — see research ticket).
- **Skills every session should consult:** `grilling` + `domain-modeling` for HITL decisions; `research` for AFK fact-finding; `prototype` only if a spike ticket is raised. `tdd`/`implement`/`to-tickets` are for *after* handoff, not this map.
- **Standing preferences (from charting):** one-shot non-interactive CLI after first-run auth; plaintext config at `~/.config/withings-garmin-sync/` (no Keychain); stateless full-overwrite (no cursor/watermark); per-metric independent success/failure; carry Withings measurement timestamps through; `--dry-run` default-on until `--apply`.
- **Reference implementation:** `jaroslawhartman/withings-sync` (Python) — but note it uses the **FIT-file upload** route, which we have ruled out; we adopt only its Withings-side reading and auth reference, plus `python-garminconnect`'s JSON endpoint payloads.

## Decisions so far

- [01 - Garmin write-path spike](issues/01-garmin-write-path-spike.md): JSON write path **proven live**. mobile-SSO→DI Bearer auth works; weight `POST /weight-service/user-weight`→`204`, BP `POST /bloodpressure-service/bloodpressure`→`200`; native Bearer+Android headers only (no `JWT_FGP`/`DI-Backend`/`NK`); **dedups by timestamp** (idempotent — stateless full-overwrite safe); EU accounts must grant "upload consent" in settings first (else `412`).
- [02 - Withings read API surface](issues/02-withings-read-api-surface.md): read via `POST https://wbsapi.withings.net/measure` (unversioned, `action=getmeas`); token via `POST https://wbsapi.withings.net/v2/oauth2` (`action=requesttoken`, `grant_type` selects exchange/refresh); scope is only `user.metrics`; meastype 1=weight, 9/10=BP, 11=pulse, decoded as `value * 10^unit`; 120 req/min, 30s code expiry.
- [03 - Garmin SSO handshake for Rust](issues/03-garmin-sso-handshake-for-rust.md): mobile-SSO→DI Bearer port is **trivial** (plain HTTPS + cookie jar, no TLS-impersonation/PKCE/webview). Ordered sequence: `GET sso.garmin.com/mobile/sso/en_US/sign-in` → `POST /mobile/api/login` (+ `/mobile/api/mfa/verifyCode` if `MFA_REQUIRED`) → `serviceTicketId` → `POST diauth.garmin.com/di-oauth2-service/oauth/token` (`grant_type=.../service_ticket`, Basic-auth client id) → `access_token`+`refresh_token`; refresh via `grant_type=refresh_token`. Native Bearer+Android headers only (no `JWT_FGP`/`DI-Backend`/`NK`). Care points: interactive MFA pause, 429-in-HTTP-and-JSON, 3 client-id retry, EU consent is user-side.

## Not yet specified

## Out of scope

- **Body composition metrics** (fat %, muscle mass, bone mass, hydration, BMI): ruled out by choosing the JSON endpoints, which cannot carry them. (Decision Q12 → b.)
- **FIT-file encoding / upload** (`/upload-service/upload`): the reference app's route; deliberately not adopted.
- **Bi-directional sync** (Garmin → Withings).
- **Other Withings metrics** (sleep, activity, heart-rate series, ECG, SpO₂).
- **Multi-account / multi-device** support.
- **GUI or menu-bar app.**
- **Packaging/distribution** beyond `cargo install`.
- **Scheduling** (cron/launchd/daemon): one-shot only; scheduling is a post-handoff concern.

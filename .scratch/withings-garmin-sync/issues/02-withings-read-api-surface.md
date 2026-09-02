# 02 - Withings read API surface

Type: research
Status: resolved

## Question

Pin the exact Withings read-API surface the Rust CLI will use, so the spec's Withings section can be written without guessing at endpoints or versioning.

Resolve, citing the current Withings docs (`developer.withings.com/api-reference/`, the `/v2/measure` spec, and the OAuth web flow):

1. The correct measurement endpooint and version: `POST https://wbsapi.withings.net/v2/measure` vs the reference repo's unversioned `POST https://wbsapi.withings.net/measure?action=getmeas`. Which is current and which `action` (`getmeas`) applies.
2. The token endpoint: `https://wbsapi.withings.net/v2/oauth2` vs `https://account.withings.com/oauth2/token` — which is current, and the `requesttoken`/`refreshtoken` actions.
3. The OAuth2 scopes required for reading weight + BP (confirm `user.metrics`, whether `user.info` is also needed, and any others).
4. The authoritative `meastype` codes and their value scaling: weight=1 (kg), diastolic BP=9 (mmHg), systolic BP=10 (mmHg), heart pulse=11 (bpm) — verify against current docs, and confirm the `value * 10^unit` decoding rule.
5. Rate limits (confirm 120 req/min) and the 30-second authorization-code expiry.

The answer must name the exact endpoints, scopes, measure codes, and scaling rule the spec will hard-code, each with a source. This ticket is AFK and unblocks the spec's Withings-client section.

## Answer

Resolved against the Withings OpenAPI spec v2.0 (`https://developer.withings.com/openapi.yaml`) and developer-guide pages. **Correction to the question's premise:** the weight/BP read endpoint is the **unversioned** `/measure`, not `/v2/measure`.

1. **Measurement endpoint:** `POST https://wbsapi.withings.net/measure` with `action=getmeas`. `/v2/measure` is activity/workout data only (`measurev2-getactivity` etc.) and does **not** serve `getmeas`. Optional `meastypes` filter: `1,9,10,11` (and `4` for height if BMI is later needed).
2. **Token endpoint:** `POST https://wbsapi.withings.net/v2/oauth2` with `action=requesttoken`; `grant_type` selects the operation (`authorization_code` for exchange, `refresh_token` for refresh — there is **no** `refreshtoken` action). The old `account.withings.com/oauth2/token` is retired. Authorize URL: `GET https://account.withings.com/oauth2_user/authorize2?response_type=code&...`.
3. **Scopes:** only **`user.metrics`** is required for weight/BP. (`user.info` and `user.activity` are not needed.)
4. **meastype codes & scaling (CONFIRMED):** weight=1 (kg), height=4 (m), diastolic BP=9 (mmHg), systolic BP=10 (mmHg), heart pulse=11 (bpm). Decode via `real = value * 10^unit` (e.g. `value=82400, unit=-3` → 82.4 kg). Response shape: `body.measuregrps[].measures[]` with `type` (= meastype), `value`, `unit`.
5. **Rate limit & expiry:** 120 req/min (rate-limit status code `601`); authorization code valid **30 seconds**. Token lifetimes: access 3h, refresh 1y.

One implementation note for the spec: `action` (and params) should be sent in the form-urlencoded POST body (matching official samples), though Withings historically accepts query params too.

Sources: `developer.withings.com/openapi.yaml` (primary), `developer.withings.com/llms.md`, `developer.withings.com/developer-guide/v3/.../oauth-authorization-url/` and `oauth-web-flow`, notifications overview.

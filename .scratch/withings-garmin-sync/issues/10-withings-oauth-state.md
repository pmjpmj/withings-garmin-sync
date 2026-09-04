# 10 - Withings OAuth authorize URL missing required `state`

Type: task
Status: resolved

## Question

`auth` fails at the Withings authorization step: the authorize URL built by
`withings::authorize_url` carries no `state` query parameter, and Withings
requires it (their OpenAPI marks `state` as `required: true` on
`GET /oauth2_user/authorize2`).

## Decisions (grilling round 1)

- Q1: state value = random per `auth` run, generated in memory, never persisted. (a)
- Q2: when the operator pastes the redirect URL back, verify the echoed `state`
  against this run's generated value; a mismatch is a hard error (`EXIT_AUTH`,
  re-run `auth`). A bare code (no state to compare) remains accepted. (a)
- Q3: BP payload left untouched pending the 400 diagnosis (ticket 11). (c)
- Q4: no spike verification; fix and let real runs prove it. (c)
- Q6: no ADR; this file records the rationale. (b)

## Answer

- `withings::generate_state()`: 16 random bytes from `getrandom` 0.4 (already
  present in the dependency tree) hex-encoded to 32 characters, fresh for each
  `auth` invocation, never persisted.
- `authorize_url(client_id, state)` now appends `&state=<url-encoded state>`.
  The token exchange is unchanged: `action=requesttoken` takes no `state`
  field per the Withings API.
- `extract_code(pasted, state)` validates the echoed state when the pasted
  string carries one (either query order); a mismatch fails with `EXIT_AUTH`.
  Bare codes pass through as before.
- Tests: unit tests for URL construction, state shape (32 hex), both query
  orders, mismatch rejection, stateless-URL and bare-code acceptance; black-box
  tests asserting the printed URL carries `state=` and that a mismatched
  pasted state exits 4 without writing tokens or calling the token endpoint.

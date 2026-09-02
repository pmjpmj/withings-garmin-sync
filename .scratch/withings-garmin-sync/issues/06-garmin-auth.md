# 06: Garmin auth: mobile-SSO → DI Bearer (MFA + client-id retry)

**What to build:** the Garmin half of the `auth` command — the mobile-SSO handshake that turns username/password (and an optional MFA code) into persisted, refreshable Garmin DI Bearer tokens.

**Blocked by:** 04.

**Status:** ready-for-agent

- [ ] `auth` performs the ordered sequence: sign-in page GET (cookie jar) → login POST →, when `MFA_REQUIRED`, an interactive MFA verify POST → service-ticket exchange at the DI token endpoint → persisted access + refresh tokens.
- [ ] Garmin username and password are prompted for interactively, not read from files; an MFA code is prompted for and loops on an invalid code.
- [ ] The three DI client ids are tried in order and the winning client id is persisted alongside the tokens.
- [ ] A refresh path (`grant_type=refresh_token`) is implemented.
- [ ] Tests assert, against fake SSO and diauth servers: the ordered sequence, the native Android header set, the MFA branch, client-id retry, and token persistence.

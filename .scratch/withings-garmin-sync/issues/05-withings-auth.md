# 05: Withings auth: OAuth authorization-code flow → stored tokens

**What to build:** the Withings half of the `auth` command — a one-time interactive OAuth authorization-code flow that yields persisted, refreshable Withings tokens, so later sync runs are non-interactive.

**Blocked by:** 04.

**Status:** ready-for-agent

- [ ] `auth` prints the Withings authorize URL with the correct `response_type=code`, client id, redirect, and scope restricted to `user.metrics`.
- [ ] Accepting the pasted redirect code exchanges it at the Withings token endpoint (`action=requesttoken`, `grant_type=authorization_code`) and persists access token, refresh token, and expiry to `tokens.json`.
- [ ] A refresh path (`grant_type=refresh_token`) is implemented and can re-issue tokens.
- [ ] A rejected code produces a clear error and a non-zero exit, without writing tokens.
- [ ] Tests assert, against a fake Withings token server: the exact token request bodies, that tokens land in `tokens.json`, and failure behavior for a rejected code.

# 08: Resilience and observability

**What to build:** the robustness layer that keeps syncs working through token expiry, rate limiting, and the EU consent gate, and makes failures diagnosable with verbose logging.

**Blocked by:** 07.

**Status:** ready-for-agent

- [ ] A Withings access token past expiry is refreshed before reads begin.
- [ ] A Garmin `401` on a write triggers a token refresh followed by a single retry.
- [ ] Withings `601` and Garmin `429` (whether surfaced as an HTTP status or inside a JSON `error.status-code`) are retried with backoff.
- [ ] The Garmin `412` EU upload-consent gate surfaces a specific, actionable message telling the operator to grant upload consent in Garmin Connect settings.
- [ ] `--verbose` logs each request and response.
- [ ] Tests assert retry/backoff and consent-message behavior against fake servers that return those statuses.

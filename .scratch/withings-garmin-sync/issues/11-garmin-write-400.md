# 11 - Garmin returns HTTP 400 on weight and blood-pressure writes

Type: task
Status: resolved

## Question

`sync --apply` against a real account: Garmin rejects both write endpoints —
`POST /weight-service/user-weight` and
`POST /bloodpressure-service/bloodpressure` — with HTTP 400.

## Facts so far

- Payload shapes match ticket 01's live-proven shapes (weight → 204, BP → 200
  on a throwaway account, 2026-09-02) and current python-garminconnect master.
- Divergences from the proven shape: BP `notes` is omitted (the lib always
  sends it, even empty) and `pulse` is omitted when the Withings group has none.
- 400 is a semantic rejection; auth-level failures (401), the EU consent gate
  (412), and rate-limiting (429) are handled separately.

## Blocked on

The HTTP 400 response bodies from a `sync --apply --verbose` run (F1), plus
whether the failing BP readings come from Withings groups without pulse.

## Decisions so far

- Q3: leave the BP payload as-is until the 400 cause is identified. (c)
- Q4: no spike verification; fix and let real runs prove it. (c)
- Q6: no ADR; this file records the rationale. (b)

## Comments

### F1 — verbose run against the real account (2026-09-04)

Both writes returned HTTP 400 with **empty** bodies:

```json
{"dateTimestamp":"2026-08-31T10:19:09.000","gmtTimestamp":"2026-08-31T09:19:09.000","sourceType":"MANUAL","unitKey":"kg","value":68.143}
```

```json
{"diastolic":83,"measurementTimestampGMT":"2026-08-29T07:36:42.000","measurementTimestampLocal":"2026-08-29T08:36:42.000","pulse":75,"sourceType":"MANUAL","systolic":119}
```

Weight: 2 failed / 0 written. BP: 1 failed / 0 written. The BP reading has a
pulse, so the missing-pulse theory is dead.

Deltas vs the spike-proven payloads (ticket 01): weight `value` 3 dp (proven:
1 dp); timestamps `.000` ms (proven: non-zero ms); dates backdated 4–6 days
(proven: now); BP `notes` absent (proven: always sent).

### Grilling round 2 decisions

- Q1: keep millisecond timestamps. (b) — no timestamp change.
- Q2: round weight to one decimal (0.1 kg) in `transform`. (a) — implemented
  (`round1`); 68.143 → 68.1.
- Q3: one-shot ship; re-run `sync --apply --verbose` on the same window. (a)
- Q4: still omit BP `notes`. (a) — the BP request is unchanged, so the re-run
  is expected to still 400 on BP unless the cause was transient.

### Re-run results

**(2026-09-02, later) Weight still 400 with `value: 68.1`** — the precision
(1 dp) theory is **dead**; the re-run proves it. The remaining weight deltas
vs the spike's proven 204 are exactly two: timestamps carry `.000` ms (spike:
non-zero ms) and the date is backdated 2 days (spike: now). The harness was
re-run (throwaway, now + non-zero ms → success), confirming the spike shape is
still accepted today.

**(2026-09-02, night) A live now-timestamped Withings measurement also 400s**
(`dateTimestamp: 2026-09-02T23:58:36.000`, `value: 68.5`, real account). The
backdated-date theory is **dead** — a now-write failed too. Two suspects
remain: `.000` ms (vs the spike's non-zero ms) or an account-level difference
(throwaway vs real). Discriminator: Probe 2 (throwaway + now + `.000` ms).

### Fix shipped (skip: Q7-b)

Per the "skip" call, milliseconds are stripped without running the throwaway
probe. `timefmt::local_ms`/`gmt_ms` became `local_iso`/`gmt_iso`, formatting
`YYYY-MM-DDTHH:MM:SS` with no fractional seconds; the renames keep the names
honest. Test fixtures updated to second precision. This revises round-2 Q1
(b → a) on the strength of tonight's live evidence.

### Resolution (2026-09-03)

Live probes on the real account settled the rules:

- Real account + non-zero ms → **204**. Real account + `.000` ms → **204**
  (once writes worked at all).
- No fractional seconds at all → **500** — Garmin's write endpoints
  **require** milliseconds on the timestamps.
- The original 400s: every payload variant (3 dp, 1 dp, `.000` ms, no ms;
  past and now dates) 400'd on the real account while the throwaway accepted
  writes — an account-level gate, consistent with the EU upload-consent
  prerequisite (ticket 01).

**Final code state:** `timefmt` keeps millisecond timestamps
(`local_ms`/`gmt_ms`, `%.3f` — the strip-ms change was reverted); weight is
rounded to 1 dp (`round1`). Payload shapes are back to the ticket-01-proven
forms, and the real account now accepts them.

**(2026-09-03) Precision rounding reverted.** The operator verified live that
Garmin accepts multi-decimal weight values, so `round1` was reverted to the
original `round3` (3 dp — float-artifact guard only). Q2 (a → b).

**(2026-09-03) Header set verified exact.** New black-box test
(`apply_weight_write_sends_exact_header_set_without_extras`) asserts the
complete raw header set of the weight write: the 8 native Android headers +
authorization + content-type + HTTP-level host/content-length, nothing else.
The CLI's request is byte-equivalent to the spike's proven curl — so any
remaining 400 is a payload-date or account effect, not a transport difference.

### Root cause (2026-09-03)

The 400 was the **HTTP protocol version**: Garmin's write endpoints return
400 (empty body) to HTTP/1.1 requests from this client; the operator's curl
succeeds over **HTTP/2** (204). The CLI's reqwest build had no `http2`
feature, so every write went out as HTTP/1.1. Fixed by enabling
`reqwest`'s `http2` feature — ALPN now negotiates h2 like the proven curl
path. Fallback if this had failed: switch `rustls` → `native-tls`.

Also: verbose mode now prints response headers (added while hunting this),
and the exact-header-set test pins the write path's wire headers.

### h2 alone was not enough

With `http2` enabled the CLI negotiates HTTP/2.0 (verbose-verified) but the
weight write still returned 400 through the same Cloudflare edge
(`server: cloudflare`, `cf-ray`, `cf-cache-status: DYNAMIC`). A curl probe
with the exact 3-dp backdated payload returned 204 — so URL, token, headers,
body, and protocol version are all exonerated. The only remaining difference
is the **TLS client**: rustls' ClientHello vs macOS curl's LibreSSL. Switched
reqwest `rustls` → `native-tls` (SecureTransport) per the pre-authorized
Q12 order.

### h1 is the accepted protocol

`curl --http1.1` with the exact payload → 204, so HTTP/1.1 is fine at the
edge — the block is hyper's h2 fingerprint, not h1 itself. Dropped the
`http2` feature (kept `native-tls`): the CLI now speaks h1 over a
curl-family TLS stack, sidestepping h2 fingerprinting entirely.

New integration test `apply_writes_send_exact_headers_and_bodies_for_both_endpoints`
pins the complete wire request for both write endpoints: exact header name
set (no extras), exact header values, host, content-length == body bytes,
and the exact JSON body — the shapes the spike's curl proved live.

### The actual root cause: duplicate Content-Type

The raw-wire test (`apply_weight_write_raw_wire_bytes_match_proven_request`)
captures the literal bytes on a real socket and immediately exposed what
every parsed-view test had hidden: **`content-type: application/json` was
sent twice**. `write_json` pushed it explicitly while `post_json`'s `.json()`
added it again. The earlier exact-set test silently `dedup()`ed names, so the
duplicate never surfaced. Strict edge handling rejects the malformed
request with an empty-body 400 — present in every failing variant (any
payload, h1 or h2, rustls or SecureTransport), absent from curl's single-
header request. Fixed by removing the explicit header; the parsed tests no
longer dedup (duplicates now fail them).

### Re-run results (duplicate Content-Type fixed)

**(2026-09-04) SUCCESS.** `sync --apply --verbose` on the real account:
weight → `HTTP 204 (HTTP/1.1)` through Cloudflare with origin headers
(`cache-control`, `pragma`, `connection: keep-alive`) — the request reached
Garmin's origin. `weight: 2 written, 0 skipped, 0 failed`; 3-dp value
`68.568` accepted live. `summary: all metrics synced`. The BP endpoint
shares the same `write_json` path and is covered by the same tests; the
Aug-29 BP reading was outside this run's window (0 written).

**(2026-09-04, later) BP confirmed live as well** — a run covering the
Aug-29 reading wrote it successfully. Both endpoints proven working on the
real account post-fix; the `notes` standby (Q4-a) was not needed.

## Answer

**Root cause:** `write_json` sent `Content-Type: application/json` twice —
once pushed explicitly, once added by `post_json`'s `.json()`. Garmin's
edge rejected the malformed request with an empty-body 400 in every variant
(payload, protocol, TLS stack), which is why every earlier theory looked
plausible and failed. The duplicate was invisible to parsed-view tests
(they deduped header names) and only surfaced in the raw-wire test.

**Fix:** removed the explicit header; `post_json`'s `.json()` supplies the
single Content-Type. `tests/sync.rs` now has (1) parsed exact-header-set
tests for both endpoints with dedup removed so duplicates fail them, and
(2) `apply_weight_write_raw_wire_bytes_match_proven_request`, which captures
the literal socket bytes and checks the request line, the full header list,
every header value, host, content-length, and the body verbatim.

**Configuration kept:** HTTP/1.1 (h1 verified acceptable; h2 fingerprints
of hyper were rejected at the edge) + `native-tls`. The `http2` feature is
removed. Verbose mode now logs response headers and the HTTP version.

**Exonerated along the way (all live-verified):** weight precision (3 dp
accepted), `.000` millisecond timestamps (required — no-fraction got a
500), backdated dates, upload consent, header set/values.

### Re-run results (http2 enabled)

(pending)

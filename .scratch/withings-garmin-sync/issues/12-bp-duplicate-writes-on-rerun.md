# 12 - Re-running sync uploads duplicate blood-pressure entries

Type: task
Status: resolved

## Question

The operator ran `sync --apply` twice over the same window and Garmin Connect
now shows two identical blood-pressure readings. Is there any check on
existing data, and what should prevent duplicates?

## Facts so far

- The CLI is stateless: `run_sync` reads the Withings window once, transforms,
  and writes every measurement unconditionally. There is **no read-back** of
  existing Garmin data, no cursor/watermark/checkpoint of what was already
  uploaded (a grep for `checkpoint|watermark|last_sync` in `src/` finds
  nothing).
- Only write endpoints exist in `src/garmin.rs`
  (`POST /weight-service/user-weight`,
  `POST /bloodpressure-service/bloodpressure`); no read endpoints are
  implemented.
- Payload timestamps derive from Withings `group.date` (epoch seconds), so
  re-running the same window produces byte-identical payloads on every run.
- The spec (`spec.md`: user stories 15/39, the Idempotency bullet under
  "Sync orchestration") relies on "Garmin deduplicates writes by timestamp",
  and README.md repeats it ("Safe to re-run: Garmin deduplicates by
  timestamp").
- Ticket 01's spike actually verified dedup only for **weight**: two
  identical weight writes → `numOfWeightEntries: 1`. For BP it performed a
  *single* write plus a read-back showing `numOfMeasurements: 1` — the BP
  replay was never run. The intermediate note (2026-09-02 evening) flagged
  duplication as **INCONCLUSIVE**, but the final answer generalized the
  weight result to both metrics.

## Open questions

1. ~~**Confirm the failure mode.**~~ **RESOLVED (2026-09-04):** the operator
   replayed byte-identical BP payloads against the live endpoint and still
   got duplicates. Garmin's BP endpoint does **not** dedup by timestamp
   (weight does; BP does not). Per-endpoint behavior is now an observed fact,
   not a hypothesis.
2. **Choose a fix strategy.** Candidates:
   - ~~read-back-and-skip (exact timestamp)~~ **not implementable** — the BP
     range read-back exposes per-day `measurementSummaries[]` with
     `numOfMeasurements` counts but `measurements: []` (no per-measurement
     timestamps to match against Withings `group.date`).
   - **day-granularity read-back-and-skip (CHOSEN)**: read back the window,
     collect the set of dates that already have `numOfMeasurements > 0`, and
     skip every Withings BP reading on those dates. Keeps the CLI stateless.
   - a stored cursor/watermark of the last-written measurement (breaks the
     "no cursor/watermark" design decision; precise per-measurement, but
     only forward-safe — a window that extends *before* the cursor would
     re-write older readings),
   - per-endpoint dedup trust: keep writing everything, document that only
     weight dedups (leaves user story 15 broken for BP).
3. **Scope.** Does weight need the same treatment, or is its dedup
   verified enough to leave as-is? **RESOLVED (2026-09-04):** weight is left
   as-is — its timestamp dedup is verified (ticket 01), so only BP gains a
   read-back path.

## Comments

### Initial report (2026-09-04)

Operator ran sync twice (no `--since`/`--until`; 30-day default window) and
Garmin Connect shows two identical BP readings. Both runs reported
"all metrics synced" with no skipped/failed counts, so the CLI gave no
indication of the duplicate.

### Timestamp-format investigation (2026-09-04)

Hypothesis tested: the duplicates happen because the BP write sends an
incorrect timestamp (`.0` fraction) while weight sends `.000`.

Finding: rejected. The Rust code already sends `.000` for **both** metrics.
Weight and BP both build their timestamps from the same
`timefmt::local_ms()` / `timefmt::gmt_ms()` (src/lib.rs:444-445, 466-467),
and both format with `%.3f` (src/timefmt.rs:17,29), which always emits three
fractional digits. A unit test pins this (`gmt_ms(1767342600) ==
"2026-01-02T08:30:00.000"`).

The `.0` in the operator's verbose log is the Garmin **response** echo
(`<- HTTP 200 ... body:`), not the outgoing request (`-> POST ... body:`).
The logged object carries `version`, `multiMeasurement`, `category`,
`categoryName` — fields our `bp_payload` never emits — so it is Garmin's
normalized stored-record shape. This means Garmin re-normalizes timestamps
to `.0` on its own side regardless of what we send, so timestamp *format*
was never the dedup discriminator.

Conclusion: no code change for timestamp format (it is already `.000`).
The remaining live hypothesis is open question 1 — Garmin's BP endpoint
does not dedup identical-timestamp writes at all.

The operator read back a populated BP day (throwaway account, one write
today). The range response is per-*day* summaries only:

```json
{"from":"2026-09-04","until":"2026-09-04",
 "measurementSummaries":[{"startDate":"2026-09-04","endDate":"2026-09-04",
   "highSystolic":120,"highDiastolic":80,"lowSystolic":120,"lowDiastolic":80,
   "numOfMeasurements":1,"category":"STAGE_1_HIGH","categoryName":"NORMAL",
   "measurements":[]}],
 "categoryStats":{...}}
```

Key observation: `numOfMeasurements: 1` but `measurements: []` — the range
endpoint returns **no per-measurement timestamps** (contrast weight, whose
`previousDateWeight`/`nextDateWeight` carry `samplePk`/`date`/`timestampGMT`
in epoch ms). So exact-timestamp read-back-and-skip is **not implementable**
with the known BP read surface.

### Failure mode confirmed (2026-09-04)

The operator replayed byte-identical BP payloads (same timestamps, same
values) against the live endpoint and still got duplicates. Conclusion:
**Garmin's BP endpoint does not dedup by timestamp** (weight dedups; BP does
not). This overturns the spec's blanket claim (`spec.md` idempotency bullet,
README "Safe to re-run") and the generalized conclusion in ticket 01 — both
must be corrected to per-endpoint behavior. Open question 1 is resolved; the
remaining work is choosing a fix strategy (open question 2) and its scope
(open question 3).

## Answer

Implemented day-granularity read-back-and-skip for blood pressure only
(weight is untouched — its timestamp dedup is verified).

- `garmin::read_bp_dates` (src/garmin.rs): `GET
  /bloodpressure-service/bloodpressure/range/{start}/{end}` with the native
  header set minus `Cache-Control`; shares the 401-refresh / 412 / 429 retry
  handling via the existing `WriteFailure` enum. `parse_bp_dates` extracts the
  `startDate`/`endDate` of every `measurementSummaries[]` entry whose
  `numOfMeasurements > 0`.
- `garmin_read` (src/lib.rs): a read twin of `garmin_write` — refreshes the DI
  token on 401 and retries once, otherwise surfaces the error.
- `run_sync` apply path: before writing BP, read back the window
  (`timefmt::local_date(since)`..`local_date(until)`) and skip any Withings BP
  reading whose `local_date` is already in the returned set; each skip logs a
  `warning:` line and counts as skipped. A failed read-back fails the BP
  metric closed (skips all BP writes that run) rather than risk duplicates.
- `timefmt::local_date` added to match Garmin's day bucketing (same local
  timezone the write path already uses for `measurementTimestampLocal`).

Docs corrected: README "Safe to re-run" and spec.md idempotency bullet + user
story 39 now describe per-endpoint behavior (weight dedups; BP read-back).

Tests: `apply_rerun_skips_bp_days_already_on_garmin_but_rewrites_weight`
(black-box: first run writes BP, second run's read-back sees the day and
skips it while weight still re-writes); `parse_bp_dates_*` unit tests;
the fake server gained a `get_prefix` route for the date-suffixed read-back
path; and the previously-dead `apply_pulse_omitted_when_withings_has_none`
test was given its missing `#[test]`. Full suite, clippy, and fmt are clean.

Known limitation (accepted): day granularity is coarse — a genuinely-new BP
reading that lands on a day Garmin already has will also be skipped. This is
the finest granularity the read-back exposes; revisit only if same-day
multi-readings become a real requirement.

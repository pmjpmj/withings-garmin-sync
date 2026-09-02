# 07: Sync both metrics end-to-end (dry-run + apply)

**What to build:** the `sync` command reading body weight and blood pressure from Withings and writing them to Garmin Connect in one pass — dry-run by default, writes only under `--apply` — with original Withings timestamps carried through and per-metric independence.

**Blocked by:** 05, 06.

**Status:** done

- [x] `sync` reads the window from Withings (`getmeas` with meastypes `1,9,10,11`), follows pagination, and decodes values via `value * 10^unit`.
- [x] Weight is decoded to kilograms; blood pressure is grouped per reading into systolic/diastolic, with pulse included when present.
- [x] Withings timestamps are carried through and converted to Garmin's millisecond-precision local and UTC formats.
- [x] `--dry-run` (default) prints a would-write report per measurement and performs no writes.
- [x] `--apply` writes weight to Garmin's weight endpoint and blood pressure to the blood-pressure endpoint using the native header set, with the two metrics treated independently (one failing does not block the other).
- [x] Out-of-range readings (systolic 70–260, diastolic 40–150, pulse 20–250) are skipped with a warning rather than aborting the run.
- [x] The final report counts written/skipped/failed per metric and sets exit `0` (both metrics ok) or `1` (any metric failed).
- [x] Tests assert end-to-end against fake Withings + Garmin servers: request traffic, the report, exit codes, and that a re-run does not duplicate entries.

## Answer

Implemented across `src/withings.rs` (read), `src/transform.rs` (decode/pair/range-check), `src/timefmt.rs` (chrono epoch→ISO-ms local/GMT + `YYYY-MM-DD` parsing), `src/garmin.rs` (payload builders + `write_json`), and `run_sync` in `src/lib.rs`. Tests: `tests/sync.rs` (11 end-to-end cases) + unit tests in the modules.

- **Read:** `POST /measure` form `action=getmeas`, `meastypes=1,9,10,11`, `startdate`/`enddate` (epoch seconds), `offset`; `Authorization: Bearer <withings token>` (per the current OpenAPI spec). Pagination follows `more`/`offset` with a 50-page safety cap and a guard against a non-advancing offset.
- **Window:** `--since`/`--until` flags win, then `config.toml` `sync.since` as the default start, else last 30 days. `--until` alone means "beginning..until". Invalid dates exit `2`; `since > until` exits `2`.
- **Transform:** weight meastype 1 → kg (`value * 10^unit`, rounded to 3 decimals); BP groups meastypes 10/9/11 by measure group, emits only when systolic AND diastolic are both present, includes pulse only when present; any out-of-range component (systolic 70–260, diastolic 40–150, pulse 20–250) skips the whole reading with a stderr warning naming the value and range. **Decision recorded here:** an out-of-range pulse skips the whole record rather than dropping just the pulse — Garmin validates the payload as a unit and would reject it anyway.
- **Writes:** `POST /weight-service/user-weight` → expect `204`; `POST /bloodpressure-service/bloodpressure` → expect `200`; both with the native Android headers + `Authorization: Bearer <di token>` + JSON content type. Whole-number BP values serialize as JSON integers (spike shape); `pulse`/`notes` omitted when absent/empty. `412` surfaces the EU upload-consent message (fully tested in ticket 08).
- **Report:** dry-run prints one would-write line per measurement (`weight: 82.4 kg at <local>`, `blood-pressure: 120/80 (pulse 72) at <local>`); apply prints per-metric `written/skipped/failed` counts and a one-line summary. Exit `0` when both metrics have zero failures, `1` otherwise (per-metric independence: a failing weight endpoint does not stop BP writes).
- **Idempotency:** re-runs send the identical payload set (full overwrite, no cursor); the test asserts byte-identical second-run writes.
- **Dependency:** added `chrono` for local/UTC ISO formatting (system timezone, tested with `TZ=UTC` through the binary seam).

- [ ] `sync` reads the window from Withings (`getmeas` with meastypes `1,9,10,11`), follows pagination, and decodes values via `value * 10^unit`.
- [ ] Weight is decoded to kilograms; blood pressure is grouped per reading into systolic/diastolic, with pulse included when present.
- [ ] Withings timestamps are carried through and converted to Garmin's millisecond-precision local and UTC formats.
- [ ] `--dry-run` (default) prints a would-write report per measurement and performs no writes.
- [ ] `--apply` writes weight to Garmin's weight endpoint and blood pressure to the blood-pressure endpoint using the native header set, with the two metrics treated independently (one failing does not block the other).
- [ ] Out-of-range readings (systolic 70–260, diastolic 40–150, pulse 20–250) are skipped with a warning rather than aborting the run.
- [ ] The final report counts written/skipped/failed per metric and sets exit `0` (both metrics ok) or `1` (any metric failed).
- [ ] Tests assert end-to-end against fake Withings + Garmin servers: request traffic, the report, exit codes, and that a re-run does not duplicate entries.

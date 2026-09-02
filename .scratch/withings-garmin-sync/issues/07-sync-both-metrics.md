# 07: Sync both metrics end-to-end (dry-run + apply)

**What to build:** the `sync` command reading body weight and blood pressure from Withings and writing them to Garmin Connect in one pass — dry-run by default, writes only under `--apply` — with original Withings timestamps carried through and per-metric independence.

**Blocked by:** 05, 06.

**Status:** ready-for-agent

- [ ] `sync` reads the window from Withings (`getmeas` with meastypes `1,9,10,11`), follows pagination, and decodes values via `value * 10^unit`.
- [ ] Weight is decoded to kilograms; blood pressure is grouped per reading into systolic/diastolic, with pulse included when present.
- [ ] Withings timestamps are carried through and converted to Garmin's millisecond-precision local and UTC formats.
- [ ] `--dry-run` (default) prints a would-write report per measurement and performs no writes.
- [ ] `--apply` writes weight to Garmin's weight endpoint and blood pressure to the blood-pressure endpoint using the native header set, with the two metrics treated independently (one failing does not block the other).
- [ ] Out-of-range readings (systolic 70–260, diastolic 40–150, pulse 20–250) are skipped with a warning rather than aborting the run.
- [ ] The final report counts written/skipped/failed per metric and sets exit `0` (both metrics ok) or `1` (any metric failed).
- [ ] Tests assert end-to-end against fake Withings + Garmin servers: request traffic, the report, exit codes, and that a re-run does not duplicate entries.

# 03: Flags, backfill, and boundary semantics

**What to build:** The window flags keep their operator-facing role: `--since`/`--until` bypass the floors entirely for the run's reads (so the operator can force a backfill), and a successful flag-driven apply still advances the floors to the newest written timestamps, so scheduled runs continue from there. Hand-lowering a floor in config is a documented backfill lever: the next apply re-sends the older measurements — for BP this duplicates entries on Garmin, exactly as the docs warn, and that re-send is pinned as intended behavior. The strictly-newer boundary (`startdate = floor + 1`) is pinned: the measurement at exactly the stored floor is never re-written.

**Blocked by:** 02 - BP sync on a machine-updated floor; read-back removed

**Status:** resolved

- [x] `--since` bypasses the floors for the run's reads; `--until` alone keeps its "from the beginning" semantics
- [x] A successful `--since`-driven apply advances both floors to the newest written measurement timestamps
- [x] Hand-lowering a floor in config makes the next apply re-send the older measurement (for BP this duplicate is pinned as the documented, intended behavior)
- [x] The measurement whose timestamp exactly equals the stored floor is not re-read or re-written (`floor + 1` boundary)
- [x] Invalid windows (e.g. `--since` after `--until`) still exit with the usage error code, unchanged
- [x] Black-box tests pin all of the above through the fake-server harness

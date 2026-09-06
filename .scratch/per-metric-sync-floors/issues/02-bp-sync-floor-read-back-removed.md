# 02: BP sync on a machine-updated floor; read-back removed

**What to build:** `sync bp --apply` advances `sync.bp.since` exactly like weight: the floor is set to the newest written Withings BP timestamp after a successful apply, and never moves on dry runs, empty runs, or failed writes. The day-granular Garmin read-back is deleted — no run ever calls Garmin's blood-pressure range endpoint — so a second same-day BP reading uploads normally and a rerun against unchanged data writes nothing. `sync all` advances both floors independently: one metric failing never blocks the other metric's floor or its data.

**Blocked by:** 01 - Weight sync on a machine-updated floor

**Status:** resolved

- [x] `sync bp --apply` writes BP readings and rewrites config so `sync.bp.since` equals the newest written Withings BP timestamp
- [x] Re-running `sync bp --apply` against unchanged Withings data writes nothing and leaves the floor unchanged
- [x] Two BP readings on the same calendar day both upload (one day, two writes)
- [x] No run sends any request to Garmin's BP range endpoint (pinned by the fake server, whose route is removed)
- [x] `sync all` advances the two floors independently: a failed weight write does not stop the BP floor from advancing, and vice versa
- [x] The old rerun test that pinned day-skip behavior is removed or replaced by the floor-based equivalents
- [x] Full test suite, clippy, and fmt are clean

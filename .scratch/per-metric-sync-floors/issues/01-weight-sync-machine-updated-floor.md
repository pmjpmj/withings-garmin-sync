# 01: Weight sync on a machine-updated floor

**What to build:** `sync weight --apply` reads only measurements strictly newer than the stored weight floor, writes them, and advances the floor to the newest written Withings timestamp — so running the sync twice writes nothing the second time. The config file gains per-metric floors (`sync.weight.since` and `sync.bp.since`, epoch seconds at Withings precision); the legacy shared `sync.since` key is dropped from the schema and ignored if present in an existing file. A metric without a floor bootstraps from the rolling last 24 hours, exactly as today. `sync all` keeps working: weight is filtered by its floor while BP (which has no floor yet in this ticket) uses the 24-hour bootstrap, all in a single Withings read. Dry runs, empty runs, and failed weight writes never touch the config, and `auth` preserves the floors when it rewrites config.

The floor keys have this shape:

```toml
[sync.weight]
since = 1767342600   # epoch seconds, machine-updated

[sync.bp]
since = 1767342600
```

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] A config with `sync.weight.since` / `sync.bp.since` loads, and `auth` (bare, `withings`, `garmin`) preserves both values when rewriting config
- [x] A config carrying the legacy shared `sync.since` still loads; the value is ignored
- [x] `sync weight --apply` with no floor reads from the rolling last 24 hours, writes the measurements, and rewrites config so `sync.weight.since` equals the newest written Withings timestamp
- [x] Re-running `sync weight --apply` against unchanged Withings data reads strictly newer than the floor (`startdate = floor + 1`), writes nothing, and leaves the floor unchanged
- [x] A dry run never modifies the config file; an apply that writes nothing and an apply whose weight writes fail also leave the floor untouched
- [x] `sync all` keeps working with one Withings read: weight filtered strictly-newer than its floor, BP on the 24-hour bootstrap
- [x] Exit codes and per-metric reports keep their existing semantics
- [x] Black-box tests pin all of the above through the fake-server harness; the existing suite stays green

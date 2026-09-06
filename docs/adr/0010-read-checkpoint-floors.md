# ADR-0010: Read-checkpoint floors

- **Status:** Accepted
- **Date:** 2026-09-06

## Context

ADR-0008 defined floor advancement as a write-echo: "a metric's floor moves
to the newest written timestamp only when every one of that metric's writes
succeeded. Dry runs, empty runs, and failed metrics never write the config."
A flag-driven apply also advanced floors.

Two observations:

- **Re-read efficiency.** When no new measurements arrive for days, an
  empty run leaves the floor put and the window (`floor + 1` → now) grows;
  every scheduled run re-fetches it. The floor only means "newest written",
  not "verified through here".
- **Operator intent.** A clean run that read `[floor + 1, until]` and found
  nothing has verified the window empty; a checkpoint meaning ("everything
  up to here has been handled") is more useful than a write-echo.

The checkpoint is sound under two recorded operator assumptions: Withings is
always up to date (a measurement reaches the cloud at its device timestamp;
no late arrival), and the sync machine's clock is aligned with the devices'
clocks (NTP on both sides). Under those assumptions the query timestamp is a
safe cut-through-everything boundary.

## Decision

- On a clean **flag-free apply** (per metric: zero failed writes, whether
  data was written or skipped), each included metric's floor advances to the
  **Withings query timestamp** — the local clock captured at the moment the
  Withings request fires (the `enddate` actually sent, before pagination),
  not the `now` computed earlier in the run and not the post-read time — with
  a **monotonic guard**: never lower than the current floor. This **amends
  ADR-0008**'s "floor moves to the newest written timestamp" and "empty runs
  never write the config".
- **Flag-driven applies never advance floors.** `--since`/`--until` runs
  still bypass floors for reads (the backfill path) but leave the floors
  untouched. This **amends ADR-0008**'s "a successful flag-driven apply
  still advances the floors".
- Dry runs never write the config (unchanged).
- A floorless metric gains a floor on its first clean flag-free apply —
  previously a floor appeared only after a first successful write.
- Failed writes still block that metric's advancement (per-metric
  all-or-nothing, unchanged); skips are not failures. A config-write
  failure warns and leaves the floors unadvanced, without changing the run's
  reported outcome (unchanged).
- The apply report notes the advancement per metric, e.g.
  `weight: 2 written, 0 skipped, 0 failed — floor advanced to
  2026-01-02T08:30:00Z`.

## Consequences

- The floor's meaning changes: from "newest written" to "everything with a
  Withings timestamp at or before the query timestamp has been handled —
  written or verified absent". The CONTEXT.md glossary is updated to match.
- The late-arrival and clock-alignment assumptions are load-bearing. If
  either breaks (e.g. phone-relayed BP that syncs hours late, a sync machine
  with dead NTP running ahead of the devices), a reading taken shortly
  before a clean run can be silently dropped: the floor passes its device
  timestamp before it reaches the cloud, and strictly-newer reads never see
  it. Clock skew on the sync machine is the first suspect if readings go
  missing right after runs.
- Flag-driven applies never touching floors means a backfill of data
  **newer** than the floor is re-sent by the next flag-free run. Weight is
  safe (Garmin dedups by timestamp); BP duplicates (Garmin's BP endpoint
  does not dedup). The remedy is to hand-adjust the floor in `config.toml`
  after such a backfill, or to accept the duplicates. Hand-lowering a floor
  remains the backfill lever; old-data backfills (below the floor) are
  unaffected.
- ADR-0009 (human-readable floor representation) is unchanged: the stored
  form, tolerant parsing, and migration-on-write behavior all stand.
- Tests pin the new behavior black-box: an empty-clean apply advances the
  floor to the query timestamp (rewriting
  `tests/floors.rs::empty_apply_never_writes_config`, which pinned the old
  behavior); a clean apply with data also lands on the query timestamp; a
  flag-driven apply leaves floors untouched; the monotonic guard never
  lowers a floor; a floorless metric gains a floor on its first clean
  flag-free apply; failed metrics never advance.

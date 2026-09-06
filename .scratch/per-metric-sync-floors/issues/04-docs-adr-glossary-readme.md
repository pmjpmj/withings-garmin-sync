# 04: Docs — ADR-0008, glossary, README

**What to build:** The feature is recorded and explained everywhere the project keeps its domain knowledge, so an operator or future agent understands the new window semantics without re-deriving them. ADR-0008 records the per-metric machine-updated sync floors: it amends ADR-0001's window-precedence resolution (the `sync.since` level becomes per-metric and machine-updated) and ADR-0005's dedup mechanism and cadence recommendation (day-granular read-back replaced; BP moves from once-daily-evening to 2-hourly), and supersedes ticket 12's chosen read-back strategy. The runtime-stateless clause of ADR-0005 stands, with the config now carrying machine-maintained floors. CONTEXT.md's glossary and README are updated to match.

**Blocked by:** 03 - Flags, backfill, and boundary semantics

**Status:** resolved

- [x] ADR-0008 written with status Accepted, amending ADR-0001 and ADR-0005 and superseding ticket 12's strategy
- [x] CONTEXT.md glossary: sync window resolution names the per-metric floors; the cadence entry drops the day-granular-dedup rationale and shows 2-hourly for both metrics; the apply entry no longer mentions the BP dedup-by-day read-back; the decisions list gains ADR-0008
- [x] README: config table and example show `sync.weight.since` / `sync.bp.since`; the scheduling section shows 2-hourly BP and drops the same-day-drop limitation; a warning states that Garmin does not dedup BP writes, so forcing an older window (`--since` or hand-lowering a floor) duplicates BP entries; a migration note covers the removed shared `sync.since`
- [x] Docs use the glossary vocabulary and contradict no remaining ADR

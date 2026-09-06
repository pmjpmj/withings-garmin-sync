# 02: Operator docs match read-checkpoint floors

**What to build:** the operator-facing documentation of the ADR-0010
behavior, after 01 ships. The README's floors section describes the
checkpoint meaning (a floor marks the Withings query timestamp of the
metric's last clean flag-free apply; everything at or before it is handled),
the backfill section states that `--since`/`--until` runs never advance
floors, and the BP-duplication warning explains that a backfill of data
newer than the floor is re-sent by the next scheduled run — duplicating BP
(no Garmin dedup) — with the remedy being a hand-adjusted floor in
`config.toml`. The per-metric-sync-floors spec's story 19 ("floors still
advance after a `--since` backfill") gets a `## Comments` note recording
that ADR-0010 supersedes it.

**Blocked by:** 01 (the docs describe the shipped behavior).

**Status:** ready-for-agent

- [ ] README floors section explains the checkpoint semantics in the
  project's glossary vocabulary (sync floor, apply, rolling window)
- [ ] README backfill section no longer claims flag-driven applies advance
  floors; it says they leave floors untouched
- [ ] README BP-duplication warning covers the recent-data backfill case and
  names the hand-adjust-floor remedy
- [ ] spec story 19 carries a `## Comments` supersession note referencing
  ADR-0010, without rewriting the historical story text
- [ ] No `README`/spec claim contradicts ADR-0010

## Comments

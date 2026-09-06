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

**Status:** resolved

- [x] README floors section explains the checkpoint semantics in the
  project's glossary vocabulary (sync floor, apply, rolling window)
- [x] README backfill section no longer claims flag-driven applies advance
  floors; it says they leave floors untouched
- [x] README BP-duplication warning covers the recent-data backfill case and
  names the hand-adjust-floor remedy
- [x] spec story 19 carries a `## Comments` supersession note referencing
  ADR-0010, without rewriting the historical story text
- [x] No `README`/spec claim contradicts ADR-0010

## Answer

Operator docs updated for ADR-0010.

- `README.md` Configuration section: the example comment now reads
  "everything at or before this is handled", the prose defines a floor as
  the Withings query timestamp of the metric's last clean flag-free apply
  (written or verified absent), notes that applies with `--since`/`--until`
  never touch floors, and names the rolling-window bootstrap in glossary
  vocabulary. The migration note now says "clean flag-free apply".
- `README.md` backfill caveat: the `--since` bullet no longer claims a
  successful backfill advances floors — it says flag-driven applies leave
  them untouched, and splits the older-than-floor case (no knock-on effect)
  from the newer-than-floor case (next scheduled run re-sends it, duplicating
  BP; remedy = hand-adjusted floor in `config.toml`). The intro now says
  forced windows "can" duplicate BP.
- `README.md` Scheduling section: "never duplicate entries or re-send data"
  is now qualified by "as long as the floors are machine-maintained", with
  the flags/hand-edit exceptions pointed at the backfill caveat.
- `.scratch/per-metric-sync-floors/spec.md`: appended a `## Comments`
  section recording that ADR-0010 supersedes story 19 (flag-driven applies
  never advance floors; clean applies land on the query timestamp), and
  also noting the story-10, Implementation-Decisions, and Testing-Decisions
  wording ADR-0010 supersedes. Historical story text is untouched.
- Verified via grep that no README/spec claim contradicts ADR-0010.

No code changed; docs-only, so no typecheck/tests were needed.

## Comments

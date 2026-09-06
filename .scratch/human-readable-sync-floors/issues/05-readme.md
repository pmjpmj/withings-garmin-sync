# 05: README — ISO floors

**What to build:** README reflects the new floor representation. The config
example shows quoted ISO datetimes; the intro's "epoch-second floor" wording
is replaced; the backfill section's hand-lowering example edits a readable
datetime or date instead of doing epoch arithmetic; a migration note covers
legacy integer floors (they keep loading and are rewritten on the next
successful apply). ADR-0009 and the CONTEXT.md glossary were updated during
design — this ticket touches README only.

**Blocked by:** 04 - Black-box tests

**Status:** resolved

- [ ] Config example: `sync.weight.since` / `sync.bp.since` as `"2026-01-02T08:30:00Z"` with a comment
- [ ] Intro: "each metric remembers its progress as an ISO-8601 floor in `config.toml`"
- [ ] Backfills section: hand-lowering example uses an ISO datetime or date, not epoch arithmetic
- [ ] Migration note: integer floors keep loading and are rewritten on the next successful apply; the removed shared `sync.since` note stays
- [ ] README uses the glossary vocabulary and contradicts no ADR

## Answer

README updated: ISO config example, ISO intro wording, datetime/date
backfill example, and a migration note covering legacy integer floors
alongside the kept removed-shared-`sync.since` note. ADR-0009 and the
CONTEXT.md glossary were updated during design.

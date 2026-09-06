# 04: Black-box tests — ISO floors and migration

**What to build:** Update the black-box harness pins in `tests/floors.rs`
from the epoch integer format to the new string format, and add the new
boundary cases. Every existing behavior pin stays: strictly-newer reads
(`startdate = floor + 1`), dry-run byte-identical config, empty and failed
runs never writing config, independent advancement, `auth` preservation,
0600 permissions, flag bypass, and the same-second exclusion.

**Blocked by:** 01, 02, 03

**Status:** resolved

- [ ] Test helpers rewritten: `floored_config` writes canonical ISO strings; `floor()` reads the string form (or a new `floor_iso` helper alongside)
- [ ] All existing tests re-run green with ISO floors (startdate assertions unchanged — internal math is still epoch)
- [ ] New: legacy integer floors load and are rewritten canonically by the next apply
- [ ] New: hand-written date (`since = "2026-01-01"`) loads as midnight UTC and behaves as a backfill
- [ ] New: offset and fractional hand-edits load, normalize, and are written back canonically
- [ ] New: malformed floor (junk, naive datetime, space separator) exits 3 with a config error
- [ ] New: apply report labels show ISO floors (ticket 03)
- [ ] Round-trip stability: a dry run over an ISO config leaves the file byte-identical

## Answer

`tests/floors.rs` rewritten: `floored_config`/`floor_iso` emit canonical ISO,
`floor()` reads string-or-integer. All 20 existing pins green unchanged; 6
new tests cover legacy-integer migration, hand-written date/offset/fraction
normalization, exit-3 malformed forms, ISO report labels, and byte-identical
dry runs. Full suite green (151 tests).

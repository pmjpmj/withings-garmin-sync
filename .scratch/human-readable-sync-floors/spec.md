# Spec: human-readable-sync-floors

Status: ready-for-agent

## Problem Statement

Since ADR-0008, each metric's sync floor lives in `config.toml` as an
epoch-second integer (`sync.weight.since` / `sync.bp.since`). The machine
writes that format exactly, but the operator edits floors by hand:
hand-lowering a floor is the documented config-level backfill lever. An
epoch integer like `1767342600` is unreadable — the operator cannot tell
when a floor points without converting, and hand-edits require epoch
arithmetic. The `--since`/`--until` flags already use a human-readable form
(`YYYY-MM-DD`), so the config is the only place sync state is stored
opaquely.

## Solution

The floors are stored as RFC 3339 UTC datetimes at second precision —
`since = "2026-01-02T08:30:00Z"` — and the machine always writes that
canonical form. Internally the floor remains an epoch-second `i64`; the ISO
conversion happens only at the serde/config boundary, so window math
(`floor + 1`, `MetricBound`, floor advancement) is untouched. Hand-edits are
tolerated and normalized on the next write: RFC 3339 datetimes with offsets
are converted to UTC, fractional seconds truncate toward the floor (never
rounded up), plain `YYYY-MM-DD` dates mean midnight UTC, and legacy integer
epoch floors still load and are rewritten canonically on the next successful
apply (or `auth` rewrite). Malformed values fail config load with exit 3
naming the accepted formats. The flag-free report labels print floors in the
canonical ISO UTC form. `--since`/`--until` and `tokens.json` are out of
scope. ADR-0009 records the decision (amends ADR-0008).

## User Stories

1. As an operator, I want the floors in `config.toml` stored as human-readable ISO-8601 datetimes like `2026-01-02T08:30:00Z`, so I can read and reason about them without converting epochs.
2. As an operator, I want the machine to always write the canonical RFC 3339 UTC form with trailing `Z` and whole seconds, so the file is stable and self-consistent.
3. As an operator with an existing config carrying integer epoch floors, I want it to keep loading with identical behavior and to be rewritten in ISO on the next successful apply (or `auth`), so the upgrade is seamless.
4. As an operator hand-lowering a floor, I want to write an RFC 3339 datetime or a plain `YYYY-MM-DD` date (midnight UTC, like the flags), so I have a readable backfill lever.
5. As an operator, I want hand-edited timezone offsets converted to UTC and fractional seconds truncated toward the floor, so tolerant edits never skip a genuine measurement.
6. As an operator, I want a malformed floor value to fail config load with exit 3 and a message naming the accepted formats, so I notice a typo immediately.
7. As an operator, I want the flag-free report labels to print floors in ISO UTC, so the output is as readable as the config.
8. As an operator, I want the strictly-newer semantics (`floor + 1`), the one-second exclusivity rule, dry-run no-write behavior, and per-metric all-or-nothing advancement to remain exactly as they are, so this is purely a representation change.
9. As an operator, I want `--since`/`--until` flags and `tokens.json` untouched by this change, so nothing else shifts under me.
10. As an implementer, I want the change pinned by unit tests for the parser/formatter and through the existing black-box harness, so the boundary is proven in both directions.

## Implementation Decisions

- **Canonical form.** `sync.weight.since` / `sync.bp.since` serialize as
  `"YYYY-MM-DDTHH:MM:SSZ"` (RFC 3339, UTC, whole seconds, trailing `Z`).
  `chrono` is already a dependency; no new crates.
- **Internal type.** `MetricFloor.since` stays `Option<i64>` epoch seconds.
  A serde module on the field handles (de)serialization, so `Config`,
  `load_config`, `write_config`, `MetricBound::resolve`, and floor
  advancement change only by loading/emitting strings.
- **Tolerant parsing.** On load, accept: RFC 3339 datetimes (offsets
  converted to UTC), fractional seconds (truncated toward the floor,
  never rounded up), plain `YYYY-MM-DD` (midnight UTC, same as the flags),
  and legacy bare integers (epoch seconds, unchanged meaning). Anything
  else — including naive datetimes without a timezone and space separators
  — is a config error (exit 3) naming the accepted formats.
- **Migration on write.** Integer floors are rewritten canonically by the
  next `write_config` (successful apply, or `auth`). Dry runs and empty
  runs never write config, so an old-format file persists until then.
- **Report labels.** `floor_label` and the combined `window_label` paths
  print the canonical ISO UTC form; flag-derived labels are untouched.
- **Out of scope.** `--since`/`--until` stay date-granular; `tokens.json`
  `withings.expires_at` stays epoch; no new range validation on floors.
- **Docs.** ADR-0009 (Accepted) and the CONTEXT.md glossary are updated as
  part of design; README is ticket 05.

## Testing Decisions

- Unit tests in `src/timefmt.rs` pin the formatter and every accepted/
  rejected parse form.
- Black-box tests in `tests/floors.rs` are rewritten from the integer
  format to ISO and gain migration, hand-edit, normalization, and malformed
  (exit 3) cases. All existing behavior pins stay: `floor + 1` startdate,
  dry-run byte-identical config, empty/failed runs never writing config,
  independent advancement, `auth` preservation, 0600 permissions.

## Out of Scope

- Widening `--since`/`--until` to datetimes (separate decision).
- Changing `tokens.json` formats.
- Any change to window semantics, dedup behavior, or exit codes.

## Further Notes

- This spec contradicts ADR-0008's "epoch-second timestamp" wording by
  design; ADR-0009 amends ADR-0008 and is the authority on the new format.
- The fractional truncation direction matters: rounding up could raise a
  hand-edited floor past a genuine same-second measurement; truncation
  toward the floor cannot.

# ADR-0009: Human-readable sync floors

- **Status:** Accepted
- **Date:** 2026-09-06

## Context

ADR-0008 made each metric's sync floor an epoch-second integer in
`config.toml` (`sync.weight.since` / `sync.bp.since`). The machine reads and
writes that format exactly, but the operator edits floors by hand: hand-
lowering a floor is the documented config-level backfill lever for re-sending
a range. An epoch integer like `1767342600` is unreadable — the operator
cannot tell when a floor points without converting, and hand-edits require
epoch arithmetic. The `--since`/`--until` flags already use a human-readable
form (`YYYY-MM-DD`), so the config is the only place sync state is stored
opaquely.

## Decision

- `sync.weight.since` and `sync.bp.since` are stored as RFC 3339 UTC
  datetimes at second precision: `since = "2026-01-02T08:30:00Z"`. The
  machine always writes this canonical form. The stored value is the same
  UTC instant as before — only its representation differs — so the
  strictly-newer (`floor + 1`) semantics are unchanged. This **amends
  ADR-0008**, which specified an epoch-second integer.
- Internally the floor stays an epoch-second `i64`; the ISO conversion
  happens only at the serde/config boundary. Window math (`floor + 1`,
  `MetricBound`, floor advancement) is untouched.
- Tolerant parsing on load, in addition to the canonical form:
  - RFC 3339 datetimes with timezone offsets (`+02:00`, `-05:30`) are
    accepted and converted to the UTC instant.
  - Fractional seconds are accepted and truncated toward the floor (to the
    integer second, never rounded up — rounding up could raise the floor
    past a genuine same-second measurement).
  - A plain `YYYY-MM-DD` date is accepted and interpreted as midnight UTC,
    matching the flag semantics (a date-granular backfill lever).
  - Legacy integer epoch floors still load with their exact meaning.
  - Anything else is a config error (exit 3) naming the accepted formats.
- Migration is on-write: a config carrying integer floors keeps loading,
  and the next successful apply (or `auth` rewrite) writes them canonically.
  Dry runs and empty runs never write the config, so an old-format file can
  persist until the next apply.
- The flag-free report label (`floor_label`) prints the canonical ISO form
  in UTC, matching the config and this ADR.
- `--since`/`--until` stay date-granular `YYYY-MM-DD`; extending them is an
  independent decision, not part of this change. `tokens.json`
  `withings.expires_at` stays epoch (machine-only state, a different file).

## Consequences

- The config becomes self-describing: an operator can read a floor, lower it
  by editing a readable datetime or date, and reason about the next run's
  window without epoch conversion.
- No semantic change to sync behavior: reads remain strictly newer than the
  floor, the one-second exclusivity rule still applies, dry runs still never
  write config, and floor advancement stays per-metric and all-or-nothing.
- Hand-edits are normalized on the next write: offsets become UTC, fractions
  truncate, dates gain their canonical midnight time. The machine never
  writes a fraction, an offset, or a date-only value.
- Old installs are unaffected until their next successful apply rewrites the
  floors; a malformed hand-edit fails config load with exit 3.
- Tests pin the boundary black-box: canonical writes, integer migration,
  date/fraction/offset acceptance, and exit 3 on malformed input.

# ADR-0003: aarch64 Linux release asset for Raspberry Pi 4

- **Status:** Accepted
- **Date:** 2026-09-04

## Context

The release pipeline (ADR-0002) ships two targets: x86_64 Linux (glibc) and
arm64 macOS. A Raspberry Pi 4 running 64-bit Raspberry Pi OS (the default OS
since 2022) has no release asset and must build from source on the device —
exactly the toolchain requirement the pipeline was built to remove. ADR-0002
explicitly left aarch64 Linux open: "No musl or aarch64 Linux targets yet."

## Decision

The release matrix gains a third entry, built natively on a GitHub-hosted
Arm64 runner (`ubuntu-24.04-arm`, GA since August 2025 and free for public
repositories):

- **Target:** `aarch64-unknown-linux-gnu`; **asset:** `linux-arm64`.
- **Native build, no cross-compilation.** The runner itself is arm64, so
  ADR-0002's stance holds: no cross toolchains, and `native-tls` links
  against the runner's own OpenSSL. The binary stays dynamically linked
  against system OpenSSL 3 (`libssl.so.3`), which Raspberry Pi OS
  (bookworm+) ships, same as the x86_64 asset.
- **Tests run on arm64 too.** The existing `cargo test --locked` gate runs on
  the arm runner like every other platform, so the pipeline doubles as an
  aarch64 test pass.
- **64-bit only.** Legacy 32-bit Raspberry Pi OS (`armv7`) is unsupported
  until someone asks — mirroring ADR-0002's Intel-Mac stance of adding a
  matrix entry when there is demand.
- **Tag/manifest contract unchanged.** The arm runner runs the same tag
  verification, and packaging follows the existing per-target tarball naming
  (`withings-garmin-sync-<tag>-linux-arm64.tar.gz`, folded into the release
  job's `SHA256SUMS`).

This amends ADR-0002, which said "No musl or aarch64 Linux targets yet."
Nothing else in ADR-0002 changes.

## Consequences

- Versioned releases now carry three assets: `linux-x86_64`, `linux-arm64`,
  `macos-arm64`.
- Installing on a Raspberry Pi 4: download the `linux-arm64` tarball, verify
  against `SHA256SUMS`, extract — no toolchain on the device.
- If an arm64 user reports a missing `libssl.so.3`, the same escape hatch as
  ADR-0002 applies: rustls or musl with vendored OpenSSL.
- If the repo ever goes private, arm64 standard runners stop being free; the
  matrix entry would need a paid plan or removal.

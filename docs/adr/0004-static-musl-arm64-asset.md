# ADR-0004: Static musl arm64 release asset (Bullseye-compatible)

- **Status:** Accepted
- **Date:** 2026-09-04

## Context

ADR-0003 added the arm64 release asset built natively on the
`ubuntu-24.04-arm` runner (glibc 2.39) and documented Raspberry Pi OS
**bookworm+** as the support floor. A Raspberry Pi 4 on **Bullseye**
(Debian 11, glibc 2.31) cannot run the v0.1.3 binary:

```
/lib/aarch64-linux-gnu/libc.so.6: version `GLIBC_2.33' not found
```

The `GLIBC_2.33` symbol version (the stat-family, introduced by glibc's
64-bit time_t transition) is emitted by Rust std whenever it is built
against glibc ≥ 2.33, so the binary's minimum glibc tracks the build host.
This is a general failure mode of dynamically-linked glibc binaries: the
minimum glibc is whatever the build machine had, and Bullseye's 2.31 is
below it.

Since commit d7183b6 the binary needs no system crypto: `reqwest` uses
`rustls` with `aws-lc-rs` (which officially supports
`aarch64-unknown-linux-musl`), and `rustls-platform-verifier` reads system
roots from `/etc/ssl/certs` in pure Rust. The only shared-library dependency
left is glibc itself — which is exactly what breaks on Bullseye.

## Decision

The arm64 asset becomes a **fully static musl build**.

- **Target:** `aarch64-unknown-linux-musl`; **asset:** `linux-arm64`
  (name unchanged). No glibc floor at all: the binary runs on Bullseye,
  Bookworm, and anything newer, with zero shared-library dependencies.
- **Still a native build.** The arm64 runner stays; `musl-tools` supplies
  the musl cross-libc toolchain. ADR-0002/0003's "no cross-compilation"
  stance was motivated by `native-tls` fragility, which no longer applies
  since the rustls switch — and this is arch-native, libc-cross only.
- **No vendored OpenSSL.** ADR-0002 anticipated "musl with vendored
  OpenSSL" as the escape hatch if libssl became a problem; with rustls +
  aws-lc-rs the musl build needs no C crypto library at all.
- **The test gate exercises the shipped target.** CI now runs
  `cargo test --locked --target "${{ matrix.target }}"`, so the arm64 tests
  run as static musl binaries, not host-glibc ones.

## Consequences

- Bullseye, Bookworm, and future Raspberry Pi OS releases all run the same
  arm64 asset; there is no glibc or `libssl.so` requirement left to
  document.
- The binary is larger (static musl) — irrelevant at this scale.
- musl behavioral differences are minor for a one-shot HTTPS CLI: the
  resolver reads `/etc/resolv.conf` directly (dhcpcd maintains it on Pi OS),
  and chrono's local time works via `TZ`/`/etc/localtime`.
- The x86_64 asset keeps glibc and has the same latent failure on distros
  with glibc < 2.33; no one has reported it. The same musl switch applies
  there if someone does.
- macOS asset unchanged.

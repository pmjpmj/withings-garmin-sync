# 13 - arm64 release binary fails on Raspberry Pi OS Bullseye (GLIBC_2.33 not found)

Type: task
Status: resolved

## Question

The operator ran the `linux-arm64` release binary on a Raspberry Pi 4 running
64-bit Raspberry Pi OS **Bullseye** and got:

```
/lib/aarch64-linux-gnu/libc.so.6: version `GLIBC_2.33' not found
```

Why does the released arm64 binary not run, and what should the fix be?

## Facts so far

- The arm64 asset is built natively on the `ubuntu-24.04-arm` GitHub runner
  (ADR-0003). Ubuntu 24.04 ships **glibc 2.39**, so every symbol Rust std and
  the dependency tree reference carries a glibc version tag of at most 2.39.
- The `GLIBC_2.33` tag in the error is the symbol version glibc introduced in
  its 64-bit time_t transition for the `stat`/`lstat`/`fstat`/`fstatat`
  family — Rust std built against glibc ≥ 2.33 emits these references.
- Raspberry Pi OS **Bullseye** = Debian 11 = **glibc 2.31**. The dynamic
  loader cannot satisfy `GLIBC_2.33` and refuses to start the binary.
  **Bookworm** (glibc 2.36) works, which is why ADR-0003 documented
  "bookworm+" and the failure only appears on Bullseye.
- The binary's only shared-library requirements are glibc; there is no
  `libssl` dependency any more (rustls since d7183b6, `reqwest` with the
  `rustls` feature; `aws-lc-rs` provides the crypto).
- The x86_64 asset has the same latent issue: built on `ubuntu-latest`
  (glibc 2.39), it would also fail on any x86_64 distro with glibc < 2.33.
  No one has reported it.

## Open questions

1. **Fix strategy.** Candidates:
   - **static musl (CHOSEN)**: build the arm64 asset as
     `aarch64-unknown-linux-musl`. Fully static, no glibc floor at all — runs
     on Bullseye, Bookworm, and anything newer. `aws-lc-rs` officially
     supports `aarch64-unknown-linux-musl` (pre-generated bindings, only a C
     compiler needed), and `rustls-platform-verifier` loads system roots from
     `/etc/ssl/certs` in pure Rust, so nothing blocks it.
   - zigbuild with a pinned glibc (`aarch64-unknown-linux-gnu.2.31`): keeps
     glibc dynamic linking but adds zig + cargo-zigbuild to CI and relies on
     `aws-lc-sys`'s cmake build working under `zig cc` — more moving parts
     for no benefit here.
   - build inside a `debian:bullseye` container: preserves the ADR-0003
     "native glibc" story, but container jobs on arm64 hosted runners are an
     uncertain support surface.
2. **Scope.** Only arm64 is reported; only arm64 changes. x86_64 stays glibc
   with the latent issue documented in ADR-0004 as a known follow-up.

## Comments

### Initial report (2026-09-04)

Operator on a Pi 4 (arm64, Bullseye) ran the v0.1.3 `linux-arm64` tarball
and got the `GLIBC_2.33 not found` loader error before any program output.
The same tarball runs on Bookworm. This matches the build-host glibc (2.39,
ubuntu-24.04-arm) exceeding the target distro's glibc (2.31, Bullseye).

## Answer

Implemented: the arm64 release asset is now a static musl build.

- `.github/workflows/release.yml`: the arm64 matrix entry targets
  `aarch64-unknown-linux-musl` (asset name `linux-arm64` unchanged); a new
  step installs `musl-tools` on the arm64 runner; the test gate now runs
  `cargo test --locked --target "${{ matrix.target }}"`, so the shipped
  target is what gets tested on every platform.
- `docs/adr/0004-static-musl-arm64-asset.md`: new ADR recording the decision
  and its consequences (no glibc floor, static binary, musl resolver/locale
  caveats, x86_64 left as-is with its latent issue documented).
- README updated: the arm64 bullet now says static musl, runs on Bullseye+;
  the stale "dynamically linked against system OpenSSL 3" claims (false since
  the rustls switch) are corrected on both Linux bullets.
- `CONTEXT.md` decisions list gains the ADR-0004 entry.
- `Cargo.toml` version bumped to 0.1.4 (tag/manifest contract requires the
  bump on `main` before tagging v0.1.4).

Immediate workarounds for the operator, in order of preference: build from
source on the Pi (`cargo install --path .` / `cargo build --release`), or
upgrade the Pi to Bookworm; v0.1.4 ships the durable fix.

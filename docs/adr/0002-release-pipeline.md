# ADR-0002: Release pipeline for Linux and macOS

- **Status:** Accepted
- **Date:** 2026-09-04

## Context

The CLI is distributed by source (`cargo install --path .`), which requires
every user to own a Rust toolchain. There was no CI at all. The plan was to
publish versioned binaries for Linux and macOS on GitHub.

## Decision

A release pipeline is triggered by pushing a version tag (`vX.Y.Z`) to GitHub
(`.github/workflows/release.yml`):

- **Native builds, one runner per OS.** `ubuntu-latest` builds
  `x86_64-unknown-linux-gnu`; `macos-latest` builds `aarch64-apple-darwin`.
  No cross-compilation: native-tls (OpenSSL on Linux, Security.framework on
  macOS) makes cross builds fragile for no benefit.
- **Linux ships glibc, dynamically linked against system OpenSSL 3** (the
  runner needs `libssl-dev` + `pkg-config`; every current distro ships
  `libssl.so.3`). No musl or aarch64 Linux targets yet.
- **macOS ships arm64 only, unsigned.** Gatekeeper quarantine after a browser
  download is documented (`xattr -d com.apple.quarantine`); real signing
  needs a paid Apple Developer account and buys nothing for one user.
- **Tag/manifest contract:** the tag must match the `Cargo.toml` version. The
  workflow hard-fails on mismatch; it never edits the manifest. Version bumps
  happen on `main` as a normal commit *before* tagging, so tags stay
  immutable and source installs match releases.
- **Release assets, not workflow artifacts.** Artifacts expire after 90 days
  and aren't versioned. Build jobs hand tarballs to the release job via
  workflow artifacts (legitimate CI-to-CI handoff), and the release job
  attaches them to the GitHub Release created by `action-gh-release`, with
  release notes auto-generated from commits since the previous tag.
- **Tests gate the release.** Each build job runs `cargo test --locked`
  before building. No clippy/fmt on the release path.
- **Packaging:** one `withings-garmin-sync-<tag>-<target>.tar.gz` per target
  plus a single `SHA256SUMS` computed by the release job.

## Consequences

- Releasing is: bump `version` in `Cargo.toml`, commit, `git tag vX.Y.Z`,
  `git push --tags`. Anything else fails the run.
- GitHub Releases become the canonical way to install without a toolchain.
- If a Linux user reports a missing `libssl.so.3`, revisit: either rustls
  (self-contained binaries) or musl with vendored OpenSSL.
- Intel Macs are unsupported until someone asks; adding
  `x86_64-apple-darwin` is a one-line matrix entry (cross-compiles fine from
  the arm64 runner).

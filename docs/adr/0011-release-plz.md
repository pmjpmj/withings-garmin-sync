# ADR-0011: Prepare releases with release-plz

- **Status:** Accepted
- **Date:** 2026-11-28

## Context

ADR-0002 requires a maintainer to bump `Cargo.toml` and push a matching tag
manually. The project distributes binaries and source installs, but does not
publish its package to a Cargo registry.

## Decision

Use the release-plz GitHub Action on pushes to `main` to maintain a reviewed
release PR. release-plz derives the version and checked-in `CHANGELOG.md` from
Conventional Commits, using Cargo's SemVer behavior for pre-1.0 versions
(features and fixes bump patch; breaking changes bump minor). On merge,
release-plz creates a `vX.Y.Z` tag in git-only mode. It does not publish a
Cargo package or GitHub Release. The tag triggers the existing
`.github/workflows/release.yml`, which tests and builds the binaries and
creates the GitHub Release with generated notes and the release assets.

The workflow uses the `RELEASE_PLZ_TOKEN` repository secret with contents and
pull-request write access. A token other than the default `GITHUB_TOKEN` is
needed so the tag push starts the binary release workflow and the release PR
starts normal pull-request checks.

## Consequences

- Merging the release PR is the approval step; maintainers no longer bump
  versions or create tags manually.
- Commits intended to affect a release must follow Conventional Commits.
- Repository setup must provide `RELEASE_PLZ_TOKEN`; without it, release PRs,
  their checks, or the tag-triggered binary workflow will not run as intended.
- The binary release workflow remains responsible for the GitHub Release and
  all downloadable assets.

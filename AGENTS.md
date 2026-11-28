## Code quality

- After making code changes, run `cargo fmt` and `cargo clippy --all-targets` and fix any issues before finishing.
- Commit messages must follow the [Conventional Commits](https://www.conventionalcommits.org/) spec (e.g. `feat: ...`, `fix: ...`, `chore: ...`).

## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, and `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: `GLOSSARY.md` at the repo root + `docs/adr/`. See `docs/agents/domain.md`.

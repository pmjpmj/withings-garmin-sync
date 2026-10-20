## Code quality

- After making code changes, run `cargo fmt` and `cargo clippy --all-targets` and fix any issues before finishing.
- Commit messages must follow the [Conventional Commits](https://www.conventionalcommits.org/) spec (e.g. `feat: ...`, `fix: ...`, `chore: ...`).

## Agent skills

### Issue tracker

Issues are tracked as local markdown files under `.scratch/<feature>/` in this repo. See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context layout: `CONTEXT.md` at the repo root + `docs/adr/`. See `docs/agents/domain.md`.

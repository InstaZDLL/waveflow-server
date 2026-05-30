# Contributing to waveflow-web

Thank you for your interest. Before submitting a pull request, please read these notes — they cover the few things that aren't obvious from the codebase.

## Developer Certificate of Origin (DCO)

Every commit must carry a `Signed-off-by:` trailer asserting that the contributor has the right to submit the work under the project's license. A `Signed-off-by:` line means you agree to the [Developer Certificate of Origin](https://developercertificate.org/).

Sign your commits with:

```bash
git commit -s -m "your message"
```

The CI rejects pull requests whose commits aren't signed off. There's no CLA — the DCO is enough.

## Commit messages

Conventional commits are enforced locally via husky + commitlint. The `commit-msg` hook validates every message against `.commitlintrc.cjs`:

- Header ≤ 100 characters
- Scope is kebab-case (e.g. `feat(auth-routes): ...` not `feat(authRoutes): ...`)
- Subject is lowercase (`subject-case: ['lower-case']`)
- The standard conventional types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `style`, `perf`, `ci`, `build`, `revert`

A typical message:

```
feat(home): wire the recently-played carousel
```

`bun install` runs `prepare: husky` automatically, which installs the hook.

## Pull requests

- Run `bun run typecheck`, `bun run lint`, and `bun run build` before opening a PR. CI runs the same triple and fails on red.
- PR labels (`scope:*`, `type:*`, `size:*`) auto-apply via `.github/workflows/label-pr.yml` — no manual labelling required.
- The PR template (`.github/pull_request_template.md`) carries the test plan checklist; fill it in honestly.

## License

waveflow-web is AGPL-3.0-only. By submitting a contribution you agree it will ship under that license. The DCO sign-off attests to this; there's no separate CLA.

## Code style

- Prettier handles whitespace + line-length. `bun run format:check` in CI; `bun run format` to fix locally.
- ESLint covers semantic rules. Configuration is in `eslint.config.js`.
- TypeScript: `strict: true` is non-negotiable. Tests included.

## Filing issues

Use the issue templates in `.github/ISSUE_TEMPLATE/`. Quick rules:

- Bugs: include browser + version, reproduction steps, expected vs actual.
- Features: explain the user-facing problem before proposing a solution.

Questions and ideas are welcome on [Discussions](https://github.com/InstaZDLL/waveflow-web/discussions). For security issues, use the GitHub private vulnerability reporting flow — please don't open public issues for them.

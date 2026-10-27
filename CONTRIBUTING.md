# Contributing to waveflow-server

Thanks for considering a contribution. This file covers the legal + workflow expectations specific to this repo. The main repo's [CONTRIBUTING.md](https://github.com/InstaZDLL/WaveFlow/blob/main/CONTRIBUTING.md) (style guides, commit conventions, etc.) applies on top of what's below.

## Developer Certificate of Origin

All commits to this repository must carry a `Signed-off-by:` trailer. This is the [Developer Certificate of Origin (DCO)](https://developercertificate.org/) — a lightweight, per-commit attestation that you have the right to submit the code under the project's license. Same model the Linux kernel, Docker, Kubernetes and Funkwhale use.

In practice:

```bash
git commit -s -m "fix(api): clamp paginated page size to 200"
```

`-s` appends a line like `Signed-off-by: Your Name <you@example.com>` to the commit message body, sourced from your local `user.name` / `user.email`. The git config must match the identity GitHub recognises as yours — anonymous or generated emails won't satisfy the check.

If you forget the trailer, amend the last commit with `git commit --amend -s --no-edit` (or rebase + sign-off for a series). CI rejects unsigned PRs.

### Full text

By making a contribution to this project, you certify that:

> a. The contribution was created in whole or in part by you and you have the right to submit it under the open source license indicated in the file; or
>
> b. The contribution is based upon previous work that, to the best of your knowledge, is covered under an appropriate open source license and you have the right under that license to submit that work with modifications, whether created in whole or in part by you, under the same open source license (unless you are permitted to submit under a different license), as indicated in the file; or
>
> c. The contribution was provided directly to you by some other person who certified (a), (b) or (c) and you have not modified it.
>
> d. You understand and agree that this project and the contribution are public and that a record of the contribution (including all personal information you submit with it, including the sign-off) is maintained indefinitely and may be redistributed consistent with this project or the open source license(s) involved.

Verbatim from <https://developercertificate.org/>.

### Why DCO and not a CLA

A DCO keeps contributing friction-free (one CLI flag), while still giving the project a defensible chain of provenance. A full CLA (Contributor License Agreement) would let the maintainer relicense contributed code later (e.g. for a proprietary enterprise offering); right now WaveFlow's plan is a hosted SaaS on the existing AGPL-3.0 terms, which doesn't require relicensing rights, so a CLA would be friction without benefit. If that changes, this section will be updated and a CLA tool (e.g. [cla-assistant.io](https://cla-assistant.io/)) added before the policy switches.

## License

By submitting a pull request, you agree that your contribution is licensed under [AGPL-3.0-only](LICENSE) for inclusion in this repository.

## Where code and tests go

The server's `src/` is filed by domain, with only `main.rs` and `lib.rs` at its root: `app/`, `cli/` and `config/`; the two API surfaces `api/` (`/api/v2`) and `subsonic/` over the domain services in `services/`; then `auth/`, `library/` (the music on disk), `storage/` (SQLite), `remote/` (anything that talks to another host), `http/` (HTTP outside the two surfaces), `media/` and `quality/`. Put a new module under the parent whose domain it serves. [CLAUDE.md](CLAUDE.md) describes each one.

Tests follow two rules, and CI checks both with `scripts/check-rust-layout.sh`:

- **Unit tests sit beside their module, never inline.** A module with tests is a directory: `foo/mod.rs` declares `#[cfg(test)] mod tests;` and the tests live in `foo/tests.rs`. Integration tests live under `tests/`, one `[[test]]` target per subject in `Cargo.toml`; a large target is a directory whose `main.rs` declares one file per subject.
- **Files stay small.** A source file under `src/` stays within 800 lines; a `tests.rs`, or any file under `tests/`, within 1,000. Past that, split the file along its seams into a directory.

The web client under `webapp/src/` is filed the same way, by domain: `app/` holds the shell and the routes, one file per area under `app/routes/`; a screen goes under `pages/<area>/`, beside the others its route file mounts; a piece shared between screens goes under `components/`, in the folder of what it serves (`layout/`, `catalog/`, `menus/`, `track/`, `artist/`, `artwork/`, `playlists/`); then `api/`, `player/`, `state/`, `lib/` and `styles/`. A unit test sits beside the file it tests, as `foo.test.ts` or `foo.test.tsx`. The Playwright suite under `webapp/e2e/` is one spec per subject, over the shared mock server in `e2e/fixtures/`. In `webapp/`, `bun run lint`, `bun run typecheck` (the client, the tool configs and the end-to-end tests, each under its own tsconfig), `bun run test` and `bun run build` cover what CI checks before the browser; `bun run test:e2e` runs the Playwright suite, which CI runs on every pull request too. CI also checks the client's shape with `scripts/check-webapp-layout.sh`: a `.ts`/`.tsx` file under `src/` stays within 600 lines, a stylesheet within 800, a unit test or any file under `e2e/` within 1,000.

CI also builds the docs with `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`, so an intra-doc link that resolves nowhere fails the build. Write a link to an item in another module as a full path, ``[`Item`](crate::module::Item)``, rather than importing the item for the link's sake.

## Commit conventions

Same as the main repo: [Conventional Commits](https://www.conventionalcommits.org/) with kebab-case scopes, lowercase subject. Examples that fit this repo:

- `feat(api): add /api/v1/playlists endpoint`
- `fix(sync): tighten the resurrected-device guard`
- `refactor(db): factor pool wiring out of main`
- `docs(rfc): point links at the merged RFC-001`

A `commitlint` GitHub Action is on the roadmap once the project has enough churn to warrant it.

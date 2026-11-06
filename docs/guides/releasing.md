# Releasing, the R2 mirror and the GitLab standby

_Written 2026-11-06, with the mirror and the standby pipeline. For the
maintainer: where a release goes, what keeps a second copy of it, and what to
do the day the GitHub repository or its account stops answering._

## Where a release goes

A tag `vX.Y.Z` (or `vX.Y.Z-suffix`) starts
[`release.yml`](../../.github/workflows/release.yml):

1. `verify` holds the tag to `Cargo.toml`, and a stable tag to its notes in
   [`docs/releases/`](../releases/).
2. The image is built per architecture and pushed by digest to GHCR and Docker
   Hub; the standalone archives are built, signed and smoke-tested
   ([`standalone.yml`](../../.github/workflows/standalone.yml),
   [`ffmpeg.yml`](../../.github/workflows/ffmpeg.yml)).
3. Once both exist, the floating tags (`latest` or `beta`) move and the GitHub
   release is published, its body written by
   [`release-notes.sh`](../../scripts/release/release-notes.sh).
4. `mirror` copies the release onto the second home described below.

## The second home

Everything a release is, kept where the GitHub account's fate does not reach:

| What | Where | Kept current by |
| --- | --- | --- |
| The code and every signed tag | [`gitlab.com/waveflow-groupe/waveflow-server`](https://gitlab.com/waveflow-groupe/waveflow-server), private | [`mirror-gitlab.yml`](../../.github/workflows/mirror-gitlab.yml), on every push to `main` and every tag |
| waveflow-core, the one git dependency | `gitlab.com/waveflow-groupe/WaveFlow-projet` (displayed as WaveFlow), the desktop app's backup | the desktop repository |
| The archives, the FFmpeg sources, `SHA256SUMS` | `https://download.waveflow.app/server/releases/<tag>/`, the R2 bucket `waveflow-releases` | `release.yml`'s `mirror` job |
| The release pages | the GitLab project's releases, linking to R2 | `mirror`, when `GITLAB_API_TOKEN` is set (see below) |
| The image | Docker Hub, `instazdll/waveflow-server`: a separate account | the release itself |

The bucket is shared with the desktop app, which owns `releases/<tag>/` and the
channel manifests at its root. The server keeps to `server/`: both repositories
tag `v2.x`, and a shared prefix would let one overwrite the other.
[`publish-r2.sh`](../../scripts/release/publish-r2.sh) uploads, then reads
every file back through the public domain and checks it against `SHA256SUMS`,
so a mirror that answers wrongly fails the job instead of passing silently.

Every release from 2.0.0-beta.0 to 2.1.2 was copied on 2026-11-06: the GitLab
releases exist for all eight, and R2 holds the archives of the four that have
some (2.1.0-rc.1 onwards; earlier releases shipped the image only).

### Secrets and variables on GitHub

| Name | Kind | Without it |
| --- | --- | --- |
| `GITLAB_DEPLOY_KEY` | secret | `mirror-gitlab.yml` says so and pushes nothing. The private half of a deploy key with write access to the GitLab project, allowed to push the protected `main` (force included) and to create the protected `v*` tags. |
| `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` | secrets | All four absent: `mirror` says so and skips. Some of them only: it fails, a half-configured mirror being a mistake. |
| `R2_ACCOUNT_ID`, `R2_BUCKET` | secrets | as above |
| `GITLAB_API_TOKEN` | secret, optional | The GitLab release page is skipped; the files are still mirrored. |
| `R2_PUBLIC_URL`, `GITLAB_PROJECT`, `GITLAB_REPOSITORY` | variables, optional | `https://download.waveflow.app`, `waveflow-groupe/waveflow-server`, its SSH address |

The R2 token is created once, in the Cloudflare dashboard: R2 → Manage API
tokens → Create, **Object Read & Write**, applied to the `waveflow-releases`
bucket alone. Its Access Key ID and Secret Access Key are the first two
secrets; the account id (R2 overview) and `waveflow-releases` the other two.

`GITLAB_API_TOKEN` stays unset on the Free tier: a project access token, the
only token scoped to the one project, is a paid feature on GitLab.com, and a
personal token with the `api` scope would open every project of the account.
Nothing is lost without it. The release pages are rebuilt from what the mirror
already holds — the tags, `docs/releases/` and the R2 files — with the
commands under [Switching to GitLab](#switching-to-gitlab).

## The standby pipeline

[`.gitlab-ci.yml`](../../.gitlab-ci.yml) is the twin of the GitHub workflows:
the checks of `ci-rust.yml`, `ci-web.yml` and `ci-image.yml` on a merge request
and on `main`, and on a tag what `release.yml`, `standalone.yml` and
`ffmpeg.yml` do, publishing to GitLab's registry, Docker Hub, R2 and a GitLab
release. The release logic it shares with GitHub lives in `scripts/release/`
and `packaging/`, so the two cannot drift apart on what a release is.

It does not run today. The backup has CI/CD turned off in its settings, and the
mirror pushes with `ci.skip`, so the minutes of the Free tier stay untouched.
`glab ci lint` is the check it gets until then; the first real run is the day
of the switch, which is why the first thing that day is a pipeline on `main`.

Three things differ from GitHub on purpose:

- **waveflow-core is fetched from GitLab.** `Cargo.lock` names
  `github.com/InstaZDLL/WaveFlow`, which goes with the account. Each job that
  compiles redirects that URL to `CORE_MIRROR_PATH` with git's `insteadOf` and
  makes cargo fetch through git (`CARGO_NET_GIT_FETCH_WITH_CLI`); the image
  build gets the same redirect as the BuildKit secret `gitconfig`, which the
  `Dockerfile` reads when it is there and ignores when it is not. The lock file
  and its revision do not change. While the desktop project is private, the
  job token reads it because that project lists this one in its job token
  allowlist (set on 2026-11-06).
- **R2 is the only download host**, so a missing R2 variable fails the release
  instead of skipping, and the GitLab release links to R2.
- **The archives' `SOURCE.txt` links GitLab.** `pack.sh` and `build.sh` read
  `WAVEFLOW_SOURCE_TREE`, `WAVEFLOW_SOURCE_BLOB` and `WAVEFLOW_CORE_TREE`, which
  the pipeline points at the GitLab projects: the AGPL source offer must not
  send a reader to a host that is gone.

## Switching to GitLab

### Before, while GitHub still answers

- **Import the issues and pull requests** if they matter: GitLab's importer
  (New project → Import project → GitHub) reads GitHub's API with your token,
  so it cannot run after the account is gone. The mirror carries code and tags,
  not issues.
- **Set the four R2 secrets**, so every release from now on is already on R2.
- Keep the desktop app's backup current too: the server's pipeline compiles the
  waveflow-core revision `Cargo.lock` pins, and that revision must be on GitLab.

### The day of the switch

1. **Make both projects public** — `waveflow-server` and the desktop's — since
   the archives' source offer and the core dependency point at them.
2. **Turn CI/CD on** (Settings → General → Visibility → CI/CD), on the server's
   project.
3. **Set the variables** (Settings → CI/CD → Variables), **protected**, masked
   where GitLab accepts it. The `v*` tags are already protected, so a tag
   pipeline receives them:

   | Variable | Required for |
   | --- | --- |
   | `DOCKERHUB_USERNAME`, `DOCKERHUB_TOKEN` | the image |
   | `SIGNTOOL_PFX_BASE64`, `SIGNTOOL_PFX_PASSWORD` | the Windows archive |
   | `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_ACCOUNT_ID`, `R2_BUCKET` | the release: R2 is the only host |
   | `MACOS_X86_64_RUNNER_TAG`, `MACOS_ARM64_RUNNER_TAG`, `LINUX_ARM64_RUNNER_TAG` | only to point at runners of your own |
   | `CORE_MIRROR_PATH`, `WAVEFLOW_CORE_TREE` | only if the desktop project's path changes |

4. **Runners.** Linux amd64 and arm64 and Windows are hosted on every tier.
   macOS is not on the Free tier: apply to the GitLab for Open Source program
   (a public AGPL project qualifies; it brings the macOS runners and a lower
   cost factor) or register `gitlab-runner` with the shell executor on a Mac.
   The hosted Macs are Apple silicon only; the Intel archive needs an Intel Mac
   of your own, tagged `MACOS_X86_64_RUNNER_TAG`. Compute minutes are the
   other limit: 400 a month on Free do not cover one release.
5. **Run a pipeline on `main`** (Build → Pipelines → Run pipeline) and read it
   to the end before tagging anything.
6. **Recreate any release page the backup lacks**, with a personal access token
   (`api` scope, short expiry, revoked afterwards):

   ```bash
   export GITLAB_PROJECT=waveflow-groupe/waveflow-server GITLAB_API_TOKEN=<token>
   tag=v2.1.3
   mkdir -p release-assets && cd release-assets
   curl -fsSO "https://download.waveflow.app/server/releases/$tag/SHA256SUMS"
   awk '{print $2}' SHA256SUMS | xargs -I{} curl -fsSO "https://download.waveflow.app/server/releases/$tag/{}"
   cd .. && git checkout "$tag"
   sh scripts/release/release-notes.sh "$tag" "https://download.waveflow.app/server/releases/$tag" \
     registry.gitlab.com/waveflow-groupe/waveflow-server instazdll/waveflow-server > notes.md
   python3 scripts/release/gitlab-release.py "$tag" notes.md release-assets \
     "https://download.waveflow.app/server/releases/$tag"
   ```

7. **Release by tagging**: bump `Cargo.toml`, write `docs/releases/<tag>.md`,
   push a signed tag. The pipeline does the rest.

### What does not come across

| On GitHub | On GitLab |
| --- | --- |
| `ghcr.io/instazdll/waveflow-server` | Gone with the account. Docker Hub keeps every version, and new ones go to GitLab's registry too. Point the README's and `operations.md`'s pull commands at Docker Hub. |
| The README's release, issue and discussion links; the landing site's download button | Point them at GitLab and `download.waveflow.app`. |
| The lrclib User-Agent names the GitHub repository (`src/remote/lrclib/mod.rs`) | Harmless; change it in the next release. |
| The DCO check, CodeQL, the PR labeller, the cache cleanup, Dependabot, CodeRabbit | Not ported. GitLab's own SAST and dependency scanning, and Renovate, are the nearest replacements; the DCO sign-off can be enforced by a push rule (paid) or a job. |
| `workflow_dispatch` retries of a tag | Run a pipeline by hand on the tag's ref. |

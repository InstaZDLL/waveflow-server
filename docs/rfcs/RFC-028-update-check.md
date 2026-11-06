# RFC-028 — Telling an administrator a new version is out

- **Status**: Proposed
- **Implemented by**: this RFC's pull request. The *Status* field above never
  flips in this project; this line is the one that says what is running.
- **Date**: 2026-11-06
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-019](RFC-019-exposure-hardening.md) (what leaves the
  server, and the promise that a default server sends nothing on its own),
  [RFC-023](RFC-023-hardening.md) (every answer read under a ceiling)

## Problem

Nothing tells the person who runs a WaveFlow Server that a newer version
exists. 2.1.1 and 2.1.2 were security releases: an instance that is never
looked after keeps the hole, and its administrator has no way to know short of
watching a repository.

The server can learn it by asking. Asking is a request to another host, and
[What leaves the server](../guides/privacy.md) promises that **a server left at
its defaults sends nothing on its own**. That promise is kept.

## Decision 1 — where the answer comes from: a manifest on R2

Each release publishes two small files beside its archives on the R2 bucket
behind `download.waveflow.app` (`scripts/release/publish-r2.sh`, run by both
`release.yml` and `.gitlab-ci.yml`):

- `server/stable.json`, moved by a stable release;
- `server/beta.json`, moved by every release, a stable one included, so that a
  server on a pre-release hears of the stable version that follows it.

```json
{
  "version": "2.1.2",
  "tag": "v2.1.2",
  "published_at": "2026-11-06T06:53:33Z",
  "page": "https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.1.2",
  "notes": "https://download.waveflow.app/server/releases/v2.1.2/RELEASE-NOTES.md"
}
```

Not GitHub's API: the address is compiled into every server out there, and it
has to outlive the account (see [Releasing](../guides/releasing.md)). A
manifest **never moves backwards**: the script compares the version it would
write with the one published and leaves a newer one in place, so a patch to an
older line, or a retried old tag, cannot tell every server to go back.

## Decision 2 — the server asks only once an administrator said yes

`WAVEFLOW_UPDATE_CHECK` takes three values:

| Value | What happens |
| --- | --- |
| `ask` (default) | Nothing is sent until an administrator answers the question the web client puts to them. The answer is stored in the database (`instance_update_check`) and can be changed in Settings › Administration. |
| `on` | The check runs; no question is asked. |
| `off` | The check never runs, and no question is asked: an air-gapped or policy-bound instance decides once, in its environment. |

Once on, the server reads the manifest at boot and every 24 hours, and at once
when an administrator turns it on. A server on a pre-release reads `beta.json`,
any other `stable.json`. The request goes through the outbound client every
other request uses — no redirect, no proxy, `WaveFlowServer/<version>` as its
User-Agent and nothing else about the instance — and the answer is read under
the 2 MiB API ceiling. A failure is logged as a verdict and kept as one, never
with the URL or the body; it changes nothing but the next answer to the route.

`WAVEFLOW_UPDATE_URL` (default `https://download.waveflow.app/server`) names
where the manifests are, for a fork that publishes its own.

## Decision 3 — administrators only

`GET /api/v2/server/update` and `PUT /api/v2/server/update` answer an
administrator of the instance, on a credential that carries administrative
authority, and nobody else: anyone else is refused with the `403` every other
administrative route answers (`Access::Admin`). Nothing is hidden by it — the
version a server runs is already public on `/health`. Only an administrator can
update, and a listener told "a new version is out" can do nothing with it.

The `GET` reports the policy, the stored decision, the running version and its
channel, the last manifest read and when, whether it names a newer version, and
whether the last attempt failed. The `PUT` records an administrator's yes or no
and answers `409` when the environment has decided (`on` or `off`).

## Decision 4 — the web client asks, then tells

For an administrator, the shell reads the route once per session:

- while the policy is `ask` and nobody has answered, a dialog asks whether the
  server may check `download.waveflow.app` once a day, saying what is sent;
- when a newer version is known, a dialog says so, links its release page,
  says how a container or an archive is updated, and can be dismissed for that
  version — the dismissal is kept in the browser, per version, so the next
  release asks again.

Settings › Administration holds the switch, and says when the check last ran.

## What does not change

No route other than the two above, no request at boot unless the check is on,
nothing in the Subsonic façade, nothing for an account that is not an
administrator.

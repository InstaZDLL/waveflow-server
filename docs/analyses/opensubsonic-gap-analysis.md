# OpenSubsonic / API v2 audit — fifth pass

Scope: `main` at `1eecc10`, that is PRs #121 and #122 (21 commits) on top of the
32 of the earlier passes. This document replaces the fourth-pass report.

> Survey of 2026-08-19, read-only against `main` at `1eecc10`; the addenda in
> §3 follow it to 2026-08-24.
>
> **Status as of 2026-10-29, `main` at `4ccf395`:** nothing in §3 or §5 is
> open. The four points of §3 were closed by #123 and #126 and on 2026-08-24,
> `getLicense`'s date excepted, which stays by decision; the four-clients
> question was settled by the campaigns of 2026-08-23 and 2026-09-16, and
> `v2.0.0-beta.1` was tagged on 2026-09-16. A third campaign ran on 2026-10-27
> and 2026-10-28 (Juliet 8, Feishin 1.17.0, Symfonium 15.1.0, DSub 5.5.3); its
> fixes are #367 to #377, one of them a deliberate change to the frozen
> contract for DSub alone (#374, `WAVEFLOW_SUBSONIC_LEGACY_CLIENTS`).

Static verification. `cargo clippy` and `cargo test` cannot run here —
`index.crates.io` is blocked by the network egress policy and no registry is
cached. `CI (Rust)` passed on `4110f22`, the merge of #121; it was still running
on `1eecc10` as this was written.

---

## 1. Both priorities are closed

### 1.1 The OAuth escalation

`Access::Unrestricted` names the category that was missing: issuing a reference.
`granted_by` returns `false` for any non-empty list, and only the early return
on an empty one grants it — so a session, an OAuth grant or a token with no
scopes may issue, and a restricted token may not.

That is the right shape, and for the right reason: this is not a role but a
restriction. An ordinary account still pairs its own devices from its own
session, which is what the flow is for; what is refused is doing it in the name
of a deliberately narrowed reference.

A single route carries it (`src/http.rs:1116`) — I checked that no other route
issues a reference under a lesser authority: `create_api_token` and
`set_subsonic_credential` are `Admin`, `auth/login` demands the password,
`auth/refresh` demands the refresh token.

The durable fix — carrying the scopes through the grant, onto
`oauth_authorization` and then `session` — is still owed, and both the enum and
the RFC say so. That is the right way to leave a debt: named where it will be
found again.

### 1.2 The compatibility matrix has been replayed

Four clients replayed on 2026-08-19 against the current contract — Symfonium,
Feishin, DSub, Juliet — each on a real device or desktop, and each **read from
the server's own state rather than from the client's display**. That distinction
is what makes the campaign credible.

Substreamer is explicitly **out of the replayed set**: the build no longer
launches on the current environment, its row stays as historical evidence
against the old contract and does not count toward the next tag. Juliet takes
its place and moves the iOS verification from an emulator to a physical iPhone
along the way.

That honesty runs through the whole document: methods a client did not call are
recorded as unexercised rather than inferred as passing, the FLAC that DSub
never finishes is recorded as unresolved with the evidence that the server is
not at fault, and two claims from the internal brief were corrected **by** the
runs rather than confirmed by them.

---

## 2. What the campaign brought back

Four server defects no automated test would have produced. That is the argument
for real client validation, made concretely rather than asserted.

| Defect | Found by | What it took to see it |
|---|---|---|
| A cold transcode refused `Range: bytes=0-` | Feishin | a browser, which opens every resource that way |
| AVFoundation's `bytes=0-1` probe was still classified as a seek after the first fix | Juliet | an iOS player, which probes before it reads |
| `createPlaylist` with `playlistId` appended instead of replacing | Feishin | a client that edits by sending back what remains |
| An album credited to two artists hung under a third entity named after the joined string | DSub | a client that browses by artist index |

The first two showed themselves in the most misleading way there is: the first
play of a track failed and the second succeeded, off the cache the failed
request had just built. A test that plays twice passes.

### The quality of the fixes — reviewed

**Byte ranges.** `starts_at_the_first_byte` and `parse_range` trim both bounds
the same way and both reject multipart: a spelling accepted cold cannot be
refused once the cache is there. `bytes=0-1,4-5` is indeed refused — the second
segment does not parse as an integer. Answering 200 with the whole stream to an
opening range is permitted by HTTP, and it is the only option available: before
the transcode exists there is no total length to put in a `Content-Range`.

**Album identity.** Four commits, which reads as honest iteration rather than
groping: while fixing the attachment to the artist, they found that
canonicalising the joined credit erased the boundaries between credits, so that
`A; B C` and `A; B; C` answered the same key and merged. The final shape
canonicalises credit by credit and joins on a separator the canonical form
cannot produce.

The repair pass is worth pointing out: it catches up libraries indexed by an
earlier version **without a rebuild**, since the files have not changed and
nothing would ever be re-indexed. And it handles the key collision by moving the
tracks and then deleting the obsolete row, rather than with an `UPDATE` that
would violate the uniqueness and fail the whole scan. Its `LIKE '%;%'` filter
matches exactly the separator `split_values` uses — I checked; it is no narrower
than the splitter.

**The fix I keep** is the one no client reported. Adding `clear_tracks` to the
intent payload unconditionally would have changed the fingerprint of **every**
playlist update this version had already recorded, turning a client's replay
across the upgrade into a conflict. The field is added only when it is set. That
is the kind of defect that shows itself only in production, at one user's, once.

---

## 3. What remains

> **Addendum of 2026-08-20.** This report describes `main` at `1eecc10`. PR #123
> has since closed **all four** lines of this section: scopes travel through the
> OAuth grant (`scopes_json` on `oauth_authorization` and on `session`, two
> migrations), `sortName` exists on album and artist and is re-derived at the end
> of a scan, `getMusicDirectory` answers for tracks with no album, and
> `/api/v2/search`'s 400 is annotated. `Access::Unrestricted` went with them: the
> carrying of scopes replaces the blanket restriction described in §1.1, which
> only stood in while the migration was missing. `getLicense` keeps its
> hard-coded date, deliberately.
>
> The framing question at the end of the section — **four clients replayed
> against five** — is still open: it is a decision, not a fix.

> **Addendum of 2026-08-23.** Point 2 below is void for its heaviest half. PR
> #126 delivered `roles[]`, `contributors[]` and `displayComposer` with the
> columns they needed — thirteen roles, an instrument sub-role, and an album that
> hangs off each of its credited artists. `sortName` had already arrived with PR
> #123. What is left of that point is the album output fields
> (`originalReleaseDate`, `releaseDate`, `releaseTypes[]`, `recordLabels[]`,
> `discTitles[]`), still absent and still honestly declared so.
>
> The framing question of §5.1 is closed: the four clients were replayed on 23
> August 2026 against the aligned model, and
> [`subsonic-compatibility.md`](../compatibility/subsonic-compatibility.md) carries the result.

> **Addendum of 2026-08-24.** Point 2 below is closed. The five album output
> fields the previous day's addendum left open are delivered:
> `originalReleaseDate`, `releaseDate`, `releaseTypes[]` and `recordLabels[]`
> hang off the album and are filled like `year`, and `discTitles[]` is derived
> from the available tracks as the genres are. The three arrays are emitted empty
> rather than absent, under both element names an album carries — `album` and,
> inside a directory, `child`. Both dates are omitted when no tag names them, as
> the reference does: an `ItemDate` with no year is not a date, and the arrays
> already carry the group's presence signal.
>
> **The thread has moved.** The four clients ask to be replayed before a stable
> tag, which §4 of the 23 August handover already requires.

Nothing structural. The list fits in four lines, and two of them are named debts
rather than defects.

1. ~~**The durable OAuth fix** — carrying the scopes through the grant.~~
   Delivered by PR #123: `scopes_json` on `oauth_authorization` and on
   `session` (`migrations-v2/20260819000000_oauth_authorization_scopes.sql`,
   `20260819010000_session_scopes.sql`).
2. ~~**`AlbumID3` and `ArtistID3` fields** needing columns.~~ Delivered:
   `sortName` by PR #123, `moods[]`, `explicitStatus`, `roles[]`,
   `contributors[]` and `displayComposer` by PR #126, then
   `originalReleaseDate`, `releaseDate`, `releaseTypes[]`, `recordLabels[]` and
   `discTitles[]` on 24 August 2026.
3. ~~**`song.parent` falls back to `library_id`** with no album: a
   `getMusicDirectory` on that identifier will not return the track.~~
   Delivered by PR #123; `getIndexes` also lists album-less tracks as root
   children since #372 (2026-10-28).
4. **Two surface inaccuracies**: `getLicense` expires at a hard-coded
   2099-12-31, and ~~`search_catalog` does not annotate its 400 for a missing
   `q`~~ (annotated by PR #123, `src/api/catalog.rs`). The date stays, by
   decision.

A framing point rather than a defect: the next tag will rest on **four**
replayed clients, not five. The document says so itself and does not try to make
Substreamer count. If five is the intended threshold, a fifth current client is
missing; if four is enough, that is worth writing down as a decision.

> **State on 2026-10-29.** Settled as four: the four clients were replayed on
> 2026-08-23 and 2026-09-16, `v2.0.0-beta.1` was tagged on 2026-09-16, and the
> same four ran again on 2026-10-27 and 2026-10-28.

---

## 4. Verdict

There is no capability gap left between the two surfaces, no known authorization
defect, and no validation debt.

| Axis | Winner | Gap |
|---|---|---|
| Authentication, sessions, revocation, scopes | v2 | wide |
| Incremental sync, idempotence | v2 | wide |
| Typed contract, operation | v2 | wide |
| Discovery, genres, favourites, bookmarks, random | tie | closed |
| Client ecosystem | Subsonic | structural |

The first four passes of this audit described gaps. This one describes a finish.
What remains in §3 does not hold a tag back: optional fields correctly declared
absent, one named architectural debt, and two cosmetic details.

> **State on 2026-10-29.** The debt and one of the two details were paid by
> #123; the verdict held for `v2.0.0-beta.1`. "No validation debt" was true of
> the contract surveyed: the 2026-10-27/28 campaign still found defects no
> automated test reached, fixed by #367 to #377.

---

## 5. Proposed priorities

> **Stale as of 2026-08-23, kept as written.** Points 1 and 2 are done: the four
> clients have been replayed, and `sortName` is delivered. See the addendum in
> §3. This list describes what remained at the time of the audit, not what
> remains today. **On 2026-10-29 points 3 and 4 are done as well** (#123), the
> hard-coded `getLicense` date excepted, kept by decision.

1. Settle the four-clients-against-five question explicitly, since it is the
   only thing between the current state and a tag.
2. `sortName` on `album` and `artist`.
3. The durable OAuth fix, before other routes have to reason about scopes.
4. The three cosmetic points of §3.

---

OpenSubsonic sources consulted:
[tokenInfo](https://opensubsonic.netlify.app/docs/endpoints/tokeninfo/),
[apiKeyAuthentication](https://opensubsonic.netlify.app/docs/extensions/apikeyauth/),
[Child](https://opensubsonic.netlify.app/docs/responses/child/),
[AlbumID3WithSongs](https://opensubsonic.netlify.app/docs/responses/albumid3withsongs/).

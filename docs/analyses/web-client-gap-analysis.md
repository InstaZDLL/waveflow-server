# The embedded web client ↔ Navidrome: the gap, surveyed, and a plan

> Survey of 2026-09-06, taken read-only against `main` at `84d0f8b`.
> The figures come from the code: the paths declared by `#[utoipa::path]` under
> `src/api/` and `src/media.rs` on one side, the calls in
> `webapp/src/api/` on the other. Navidrome is described from its own
> documentation ([navidrome.org/docs/overview](https://www.navidrome.org/docs/overview/)),
> not from memory.
>
> No code was modified during this survey.

## What was found

**The server declares 63 paths under `/api/v2`. The web client calls 26.**

That is the fact which governs this whole document. What the interface is
missing is mostly not to be built: it is to be wired up. Of the fifteen families
of features absent from the screen, **eleven are already served** and are
waiting for a `fetch`. Only four ask for server work, and three of those four
are features Navidrome has and WaveFlow never intended to have.

The second observation is about form and follows from the first. The interface
looks generic less because of its colours than because of its **emptiness**: a
grid, a sidebar, a player, and little else on screen. Navidrome does not look
richer because it is better drawn — it looks richer because it puts sorting,
filtering, hover-to-play and queue management into the same square inches.

> **Corrected on 2026-09-06, while writing batch A.** This survey first wrote
> "no page header, no sorting, no filtering, no hover". Two of the four were
> false, and they were already false when the sentence was written:
> `PageHeader` has existed since `e7c8b64` and is used on eight pages, and
> `.grid a:hover .cover` already lifted the cover. What was missing was more
> precise than "the header" and "the hover": the **controls** in the header, and
> the **actions** in the hover. An inventory taken read-only sees what is absent
> from the screenshot, not what is absent from the code.

A third thing deserves saying up front, because it changes the cost of
everything else: **there is no UI library at all.** No Tailwind, no Radix, no
shadcn, no Lucide. Three runtime dependencies — React, React DOM, TanStack
Router — and 1,327 lines of hand-written CSS, 666 lines of theme tokens,
home-made SVG icons. The "template" look comes from the vocabulary used, not
from an imposed framework. So there is nothing to tear out, and every rule can
be changed without fighting anybody.

## 1. The functional gap with Navidrome

### What both can do

Multiple accounts with per-user state, multiple libraries with per-account
rights, on-the-fly transcoding, public-link shares, automatic library watching
(`WAVEFLOW_SCAN_INTERVAL_SECS`), a themeable and responsive web interface,
Subsonic/OpenSubsonic compatibility validated against five real clients.

On that foundation the gap is not functional. It is in the interface.

### What Navidrome does and WaveFlow does not do at all

These five points ask for the server, not the screen. **Two are still not
begun.** Three are closed, server and screen both: external scrobbling on
2026-09-14 — see point 15 — the `.m3u` import on 2026-10-12 and internet radio
the same day.

| | State in WaveFlow |
| --- | --- |
| **Smart playlists** (dynamic, iTunes style) | nothing |
| **Automatic `.m3u` import** | **done on 2026-10-12** (#216), import and export, server and screen — see `docs/plans/ui-and-missing-features-plan.md` |
| **External scrobbling** — Last.fm, ListenBrainz, Maloja | done, RFC-010; server and screen, see point 15 |
| **Internet radio** | **done on 2026-10-12** (#217), one list per account, the four Subsonic methods, `/api/v2/radio-stations` and a page |
| **Jukebox mode** | `jukeboxRole` is declared `false` (`src/subsonic/nodes.rs:319`) |

Two gaps of degree come on top. Navidrome's **transcoding** is configured per
user and per player; WaveFlow has only concurrency ceilings,
`WAVEFLOW_TRANSCODE_GLOBAL_LIMIT` and `..._PER_USER_LIMIT`, with no per-account
format or bitrate profile. And **translation**: 34 languages against two.

### What Navidrome does, the server already knows how to do, and the screen ignores

This is the bulk of the gap, and it costs client work only.

| Feature | Route already served |
| --- | --- |
| Five-star ratings | `GET /ratings`, `PUT /ratings/{type}/{id}` |
| Resume playback | `GET /bookmarks`, `PUT`/`DELETE /bookmarks/{track}` |
| Lyrics | `GET /tracks/{id}/lyrics` |
| Browsing by genre | `GET /genres`, `GET /songs?genre=` |
| Listening history | `GET /history` |
| Random playback | `GET /songs/random` (filterable by genre and years) |
| Now playing | `GET /now-playing` |
| Live scan progress | `GET /scans/{id}`, `GET /scans/{id}/events` (SSE) |
| A library's tracks | `GET /libraries/{id}/tracks` |
| Member management | `PUT`/`DELETE /libraries/{id}/members/{user}` |
| API tokens | `GET`/`POST /admin/users/{u}/tokens`, `DELETE .../{id}` |
| Transcoding state | `GET /transcode/status` |

### What WaveFlow does and Navidrome does not

The gap is not one-way, and these four have no screen either.

- **Receiving a file** — [RFC-008](../rfcs/RFC-008-receiving-a-file.md), fragmented
  negotiation, quota, commit. `POST /libraries/{id}/uploads` and the session
  routes.
- **The per-track canvas** — [RFC-009](../rfcs/RFC-009-track-canvas.md), six
  routes.
- **The library event stream** —
  [RFC-007](../rfcs/RFC-007-library-event-stream.md), with the originating device,
  retention and acknowledgement.
- **A tag correction that survives a rescan** — `PATCH /tracks/{id}`, backed by
  `track_override`.

A fifth, invisible but structural: album and artist identifiers are **derived**
from the tags (`src/pid.rs`, UUID v8), hence stable from one installation to the
next.

### What is not a goal

To be written once so as not to come back to it. The **Subsonic facade is
frozen** for `v2.0-beta`; nothing in this plan may change its observable
behaviour. The **desktop client** stays the privileged consumer of the recent
surfaces: the web client does not have to catch up with the desktop route for
route.

## 2. The wiring: what is really missing

### Eleven connections, zero lines of server

Each is a screen or a control against a route that already answers. Nothing to
design, nothing to migrate, nothing for the domain to review.

1. **Stars** on the track, the album and the artist. The route accepts all
   three types; the screen shows none.
2. **Lyrics** in a "now playing" view. The route returns a `LyricsList`, hence
   both the synchronised and the flat form.
3. **Resume**: put the playhead back where it was. The bookmark exists per
   track.
4. **Genres** as a navigation entry, with `GET /songs?genre=` for what follows.
5. **History** and **recent plays**, both in `GET /history`.
6. **Random** as a front-line action — it is what a music server does when you
   do not know what to listen to, and the route already filters by genre and by
   years.
7. **Now playing**, in the administration: who is listening to what, on which
   device.
8. **Live scan progress.** The administration screen starts a scan and then says
   nothing more. The SSE stream exists — but **not for `EventSource`**: the
   route goes through `authenticated()`, which reads the `Authorization` header,
   and `EventSource` sends none. That is the wall stream tickets work around for
   `<audio src>`; here the way through is `fetch`, whose response body is read
   as it arrives, which puts the framing on the client.
9. **A library selector.** The client lists the libraries in the administration
   but never frames the navigation on one, while
   `GET /libraries/{id}/tracks` and the `library_id` parameters of `/genres`,
   `/songs` and `/songs/random` are made for it. It is the one gap where
   Navidrome is frankly ahead on a feature WaveFlow has.
10. **A library's members**: add, change role, remove.
    **Corrected on 2026-09-07: this point is not only a connection.** There are
    `PUT` and `DELETE` on `/libraries/{id}/members/{user}`, but **no route lists
    a library's members**. A screen could therefore not show who has access,
    only grant and revoke blind. A `GET /api/v2/libraries/{id}/members` is
    needed before drawing it.
11. **API tokens**, so that an account can create one without the CLI.

### Three connections for the recent surfaces

Heavier, because the interaction has to be designed as much as called.

12. **Tag correction** — `PATCH /tracks/{id}`. A form per track, and the
    question of what is shown when a correction diverges from the file.
13. **Upload** — negotiation, fragments, commit. This is the biggest: it needs a
    queue, a resume, progress, and the `accepts_uploads` flag respected in the
    screen (a closed library must not offer the button).
14. **Canvas** — place, replace, remove, and read it in the player through
    `canvas-ticket` then `canvas-stream/{ticket}`. The ticket discriminator
    already exists, so `<video src>` will work with no header.

### What really asks for the server

15. **External scrobbling.** It is the one of Navidrome's five gaps whose
    absence is noticed in daily use. It needs an outgoing HTTP client,
    per-account credentials and a retry queue — hence a design decision, hence
    an RFC before any code.

    > **The RFC has existed since 2026-09-13**:
    > [RFC-010](../rfcs/RFC-010-external-scrobbling.md). It settles the attachment
    > point, the durable queue that freezes what was listened to, what is not
    > queued, the two tiers of credentials, the atomicity boundary — a
    > transactional queue is atomic with ours, never with a third party — and
    > the outgoing surface, this server's first. Revised the same day after an
    > external review.
    >
    > **And it has been implemented since 2026-09-14.** The three recipients
    > answer, the queue purges, and a destination exists in several instances
    > named by the operator. What a web client then had to do held in four
    > gestures:
    >
    > - read `GET /api/v2/scrobble-destinations` and offer the names, with the
    >   reason when one of them is unavailable;
    > - place and withdraw an authorization through
    >   `PUT`/`DELETE /api/v2/scrobble-links/{provider}/{destination}`;
    > - open the Last.fm journey through
    >   `POST /api/v2/scrobble-links/lastfm/authorize/{destination}` and send
    >   the browser to the address returned — the return concludes on its own
    >   and redirects;
    > - show the state of the queue, and above all **ask the person** what to do
    >   with the uncertain listens: decision 13 refuses to settle it in their
    >   place, so an interface that does not show them leaves that choice
    >   unreachable.
    >
    > The "External scrobbling" section of the
    > [API guide](../guides/api-v2-guide.md#external-scrobbling) describes all four.
    >
    > **The four are done, on 2026-09-14** by
    > [#201](https://github.com/InstaZDLL/waveflow-server/pull/201), in
    > `webapp/src/lib/scrobbling.ts`
    > and `scrobbling-page.tsx`. The screen is at `/settings/scrobbling`, and
    > that path is not a choice: `lastfm_callback` redirects there once the link
    > is concluded, so no other spelling works — and as long as it did not
    > exist, a Last.fm journey carried to its end landed on the "not found"
    > page.
    >
    > Two things the list above did not say, and which came out while writing
    > the screen. A link to an instance the operator has removed from their
    > configuration must stay displayed: the server breaks it without erasing
    > it, because the queue behind it is somebody's, and a list built on the
    > offered instances alone would make it disappear along with the explanation
    > and the means of removing it. And an uncertain listen carries no title —
    > decision 12 keeps the envelope out of the API — so the screen names it by
    > cross-referencing `played_at` with `GET /api/v2/history`, which is what
    > that field is documented for. Two listens at the same instant name
    > nothing: guessing would ask the question about the wrong one.

The other four — smart playlists, `.m3u` import, radio, jukebox — are decided
before being estimated. None is a prerequisite of `v2.0-beta`.

> **The `.m3u` import was decided and done on 2026-10-12** (#216): both
> dialects, the encoding guessed from the bytes and not from the extension,
> resolution by file name then by agreement on the parent folders, the
> `#EXTINF` credit as a last resort, and a report of what could not be placed.
> Plus the export, for the round trip.
>
> **And internet radio was decided and done the same day** (#217): a
> `radio_station` row per account, the three Subsonic writes beside the getter
> that had always answered an empty container, `/api/v2/radio-stations`, and a
> page that manages the list and plays a station from its own element — the
> queue player assumes a track with a duration, an id and something to
> scrobble, and a stream has none of the three. **The decision the issue asked
> for first**: the server stores an address and never fetches it, so the
> feature reaches no host from inside the network it runs in.
>
> **Two remain**: smart playlists, jukebox.

### A sixteenth, which posing the fifteenth revealed

16. **On a phone, eight screens have no link.** Measured on 2026-09-14 while
    posing the fifteenth: `.sidebar` goes to `display: none` below 820 px
    (`styles.css`), and `Navigation({ mobile: true })` keeps only the entries
    marked `primary`. The other eight — artists, genres, random, history,
    shares, upload, administration, and the scrobbling that has just been
    added — stay reachable **by typing their address**, and by nothing else. The
    test suite does not see it: the mobile cases navigate by `page.goto`, never
    by clicking the bar.

    This is not a defect of #201, which only revealed an eighth case: the other
    seven have been there since batch A. And it is not an argument for marking
    the scrobbling screen `primary` either — a bar of six slots where one plays,
    searches and queues is not the place for a settings screen, and pushing it
    in would make the other seven even more invisible by contrast. What is
    missing is an **overflow**: a gesture that opens the rest. Its shape is to be
    decided — a menu, a "more" screen, a drawer — and that is an interface
    decision, so it waits for its own rather than slipping into a pull request
    about something else.

    > **Settled and done on 2026-09-15**, by
    > [#202](https://github.com/InstaZDLL/waveflow-server/pull/202): a seventh
    > slot, "More", opens a sheet listing the other eight. A native `<dialog>`,
    > because the focus trap, the Escape key and the inertness of the page
    > behind come from the element and not from code to be rewritten every time
    > the markup moves.
    >
    > **And the bar was already overflowing.** Its grid said `repeat(5, 1fr)`
    > from the first drawing, which had five `primary` entries; a sixth arrived
    > later without touching it, so the bar had been going to two rows since.
    > The suite did not see it: it measures horizontal overflow, and a second
    > row overflows downwards. The number of columns is now the browser's, so it
    > can no longer go stale.

## 3. The redesign plan

### The diagnosis, and why it is not a matter of taste

The complaint "it looks like shadcn" is right about the look and wrong about the
cause: there is no shadcn. The vocabulary used — a very dark neutral background,
a single saturated accent, rounded cards, a full-width button — is the one
everybody produces, and it can be changed in CSS.

But the main reason the interface looks generic is **that it is empty**. A page
carrying only a grid looks like every page carrying only a grid. The redesign is
therefore first a matter of **density**, which has the advantage of being
checkable instead of debatable: either a page has a header and controls, or it
does not.

### The moves, from the most profitable to the least

**Controls in the page header.** The header exists and already carries the title
and the item count; what it lacks is sorting and filtering. There was only one
order, with no way to change it, while `GET /api/v2/albums` has accepted `sort`
all along — `AlbumOrder`'s vocabulary, shared with Subsonic's `type`. It is the
move that closes the most of the gap with Navidrome, and it depends on no new
route.

One reservation about that vocabulary, found while wiring it: `random` is
`ORDER BY RANDOM()` under a `LIMIT`, so every page is an independent draw and a
paged list would repeat and omit albums. It has no place in a sort menu; batch
B's random is `GET /songs/random`, which is something else. And four of the
orders **filter** as much as they order: `frequent`, `recent` and `starred`
answer only for albums that have a play count, a last listen or a star, and
`byYear` only for those carrying a year. The count displayed must therefore be
read off the response.

**Affordances on the covers.** The hover exists — it lifts the cover — but it
offers nothing: one has to enter an album to start anything. It lacks "play" and
"queue". Two constraints attach to that: the buttons cannot live inside the
anchor, where they would be neither valid nor keyboard-usable, and the player
had no `enqueue`.

**A player that deserves its bar.** It carries previous, pause, next, a scrubber
and two durations. It lacks volume, shuffle, repeat, access to the queue, and the
way back to the current album from the cover.

**Titles on two lines.** `line-clamp: 2`. Today truncation strikes permanently —
"50thSg発売記念〜モー…", "Armageddon - The 1st Al…",
"Ballad of the Mer (Orches…". One line of CSS for half the problem.

**The two native `<select>`s** for theme and language, in an application styled
entirely by hand. It is the most visible unfinished edge of the interface.

**Typography, decided once.** The display serif is the one move that is not
generic.

> **Corrected on 2026-09-06.** "It is an orphan: it is used on the sign-in
> screen and nowhere else" is false, and it is this document's fifth claim not
> to survive a reading of the code. The serif carries `h1, h2, h3` — hence
> **every page title** —, the sidebar's mark, the letter that stands in for a
> missing cover and the code on the 404 page. It was already a system; what made
> it look like an accident is that it was declared **four times, in two
> spellings** — three of the four omitted "Palatino Linotype", so the fallback
> was not the same depending on which screen was being looked at.
>
> The operator's decision: make it a system. In practice all that was left was
> to name it, `--font-display` and `--font-body` at the top of `styles.css`, one
> stack each. **This question is closed.**

**The card inside the card**, on sign-in: two nested rounded rectangles whose
backgrounds differ by three per cent. It is the strongest "template" signal on
the screen.

**Composition across the width.** The sign-in screen was drawn around 1280 px
and then centred; beyond that it floats in a void that does no work.

~~**The language of the sign-in screen.**~~ **Removed on 2026-09-06: the defect
does not exist.** `I18nProvider` already reads `navigator.language` when nothing
is stored (`i18n.tsx:376`), and it wraps `RouterProvider`, so the sign-in screen
benefits like the rest. The English observed during the survey came from the
session's browser or from a `waveflow.locale` already written — not from a
selector living "after the session". Nothing to correct here; the real question
about languages is further down, and it is about how many.

**The emptiness of the sidebar**, between the navigation anchored at the top and
the settings anchored at the bottom.

### What must not be done

Do not introduce a UI library to solve a vocabulary problem. Three runtime
dependencies and 1,327 lines of CSS are an asset: the client's automated gate —
Biome, TypeScript, Vitest, Playwright and the WCAG A/AA rules through
`@axe-core` — holds because the surface is small.

## The proposed sequence

Four batches, ordered so that each can ship alone.

**Batch A — density.** Sort and filter controls in the header, `line-clamp`,
hover actions, styled selects, the emptiness of the sidebar. Zero new routes. It
is the batch that changes perception most for the least risk.

> **State on 2026-09-06.** The whole batch is written except the last point.
> Sorting goes through the server, filtering stays in the browser — `collect`
> already holds the whole list, and a round trip per keystroke would buy
> nothing. The `enqueue` the hover demanded was added to the player along the
> way. **The emptiness of the sidebar was not addressed**: everything that would
> fill it is either content to invent or batch B (genres, recent plays). It
> joins the open questions below rather than being settled in passing. The
> language on first load has left the list: see the struck-out move, it was
> already done.

**Batch B — wiring up the comforts.** Stars, lyrics, resume, genres, history,
random, library selector. Eleven routes that exist, a screen each or nearly. It
is what closes the Navidrome gap.

> **State on 2026-09-06.** Six of the seven are wired: five-star ratings on the
> track table, genres (the list and a page per genre), recent plays, random,
> lyrics and saved positions in a "now playing" view. **The library selector
> was deliberately left aside** — it was the first of the open questions below.
> **Settled on 2026-09-07: one active library at a time.** It therefore remains
> to be written, and it joins batch C rather than reopening batch B.
>
> Three things learned while wiring. `GET /songs?genre=` takes the genre in its
> display form and canonicalises it before comparing, so "Hip-Hop" and "hip hop"
> are one genre on the server. `GET /history` answers **listens** and not
> tracks — `track_id`, `submission`, `played_at` — so a screen that wants titles
> resolves them itself. And the favourite, until then rendered with a star,
> became a heart: five rating stars beside a favourite star would have made one
> control asking two questions.

**Batch C — the player and operation.** Volume, shuffle, repeat, an accessible
queue, the way back to the album. Then on the administration side: live scan
progress, now playing, members, tokens. **Plus the library selector**, left over
from batch B, now decided and therefore writable: one active library, hidden if
there is only one, remembered from one visit to the next.

**Batch D — the recent surfaces.** Tag correction, then upload, then canvas. In
that order: the upload is a machine, and the canvas assumes batch C's player
exists.

> **Tag correction delivered on 2026-09-11.** The editor lives at
> `/tracks/{id}/edit`, reached from every track row by an owner or a manager of
> the library. It shows the provenance of the seven scalar fields, offers to
> restore the two lists with no preview, and sends only what the form changed:
> computing the body is a pure, tested function, because that is where data loss
> hides. Retyping the file's value removes the correction instead of pinning it
> against a future retag.

> **Upload delivered on 2026-09-11.** The `/upload` page appears only in a
> library that accepts files, for an owner or a manager — which required
> exposing `accepts_uploads` on `GET /api/v2/libraries`, until then read by the
> upload service alone. Every file is identified in the browser by a BLAKE3
> computed in a Web Worker with `@noble/hashes`: the server recomputes that hash
> at the commit and refuses any other. A fragment whose acknowledgement is lost
> is resumed from the state the server reads back, never from a guess by the
> client. The canvas remains.

> **Canvas delivered on 2026-09-11, and batch D is closed.** An owner or a
> manager places, replaces and removes a track's loop from its editor, where the
> operator has opened the library to canvases — which required exposing
> `accepts_canvas` on `GET /api/v2/libraries`, a door distinct from the files'.
> The Now Playing page plays it over the cover as the desktop does: muted,
> looping, hidden from assistive technology, absent when the user has asked for
> less motion, and the cover stays visible if the browser cannot read the video.
> The client learns that a track carries a canvas by asking for its ticket: no
> field on the track says so, and the ticket's 404 already answers it.

> **Corrected on 2026-09-07.** This line said "the correction is a form". It is
> not: `GET /tracks/{id}` exposes no provenance and `PATCH /tracks/{id}` erases
> the corrections it is not sent back, which a client cannot do since it cannot
> read them. Batch D's first item therefore asks for **the server before the
> screen**: the read route `/tracks/{id}/overrides` and the fixing of the
> existing `PATCH` into a partial patch, both described above. It is this
> document's sixth claim corrected by implementing it, and the only one whose
> stake was a silent data loss.

External scrobbling fits in no batch. It needs an RFC.

> **It got one, and it is closed — 2026-09-14.**
> [RFC-010](../rfcs/RFC-010-external-scrobbling.md), written on 2026-09-13 and
> implemented the next day on the server, then on the screen the same day. So it
> never joined a batch: it followed its own path, from the decision to the
> button.

## The contract for tag corrections

> Decided on 2026-09-07, before batch D. This is not a screen rule: the three
> notions below will turn up in the API, in search, in sorting, in albums, in
> artists and perhaps in synchronization.

### Three values, and only one leaves the catalogue

```
source     = what the file contains              (`track` columns)
override   = the WaveFlow correction, or absent  (a `track_override` row)
effective  = override ?? source
```

> **Corrected on 2026-09-10, while writing the read route.** That diagram holds
> for **seven** of the nine fields, not for nine. Artists and genres are not
> columns: a correction on either is **materialised** into `track_participant`,
> `track_genre` and the `*_display` strings — `apply_track_override_lists` does
> `DELETE FROM track_participant` and then reinserts — because those rows feed
> every projection and the search index. **Once the correction is placed, the
> file's value is therefore no longer in the database**, only in the file. The
> seven scalar fields survive because the projection merges them with
> `COALESCE` instead of overwriting them.
>
> The consequence for the editor: it shows the provenance of the seven, and for
> the other two offers a restore **with no preview**. Restoring re-reads the
> file, which `set_track_metadata` already does; what is not cheap is displaying
> it in advance.

The server already works this way and says so: the migration
`20260826010000_track_override.sql` writes "*every column is nullable and NULL
means "no correction here, use what the file said"*", and `song_select!` does
`COALESCE(ovr.title, t.title)`. **The whole catalogue consumes `effective`.**
The ordinary screen therefore shows `Around the World` and nothing else — no
double line, no mention of provenance in a grid.

The vocabulary matters: this is not a *modified tag*, since the file is not
touched. It is an **override**, a **WaveFlow value**, a **correction**.

### What is missing to build the editor

Two defects, found while preparing batch D, and the second is the serious one.

**Provenance is exposed nowhere.** `GET /api/v2/tracks/{id}` returns a
`SongItem`, hence the effective value, and nothing says whether a title comes
from the file or from a correction. An editor cannot show what it offers to
restore.

**`PATCH /api/v2/tracks/{id}` replaces the whole set.** An omitted field is not
"left as it was": it is *erased*, and an empty body deletes the row. Since the
client cannot read the existing corrections, correcting the title alone silently
erases the others. That is not an ergonomic annoyance, it is a data loss with no
message.

### The shape chosen

A new read route, and the existing write route corrected.

```
GET   /api/v2/tracks/{id}            → SongItem, the effective value, unchanged
GET   /api/v2/tracks/{id}/overrides  → { source: {…}, overrides: {…} }   ✅ written
PATCH /api/v2/tracks/{id}            → a three-state partial patch        ⏳ blocked
```

The read route is written: it has no consumer to spare and the audit
prerequisite applied only to the **change of semantics**. The `PATCH` is still
waiting on the desktop client's audit.

`overrides` must be the **real row**, not a second `COALESCE` projection: that
is precisely the difference the editor displays. `effective` does not have to
appear there, the client already holds the `SongItem`.

The patch has three states, and that is the whole point:

| The body carries | Effect |
| --- | --- |
| nothing for that field | the correction does not move |
| a value | creates or replaces the correction |
| `null` | removes the correction; `effective` becomes the file's value again |

"Restore the file's value" is therefore `{"title": null}`, and not a separate
verb.

**Why fix the route rather than add a second one.** A
`PATCH /tracks/{id}/overrides` beside a `PATCH /tracks/{id}` would leave this
repository with **two write APIs for the same thing, for ever**, for the sole
reason that the first had bad semantics — decided before the beta was even out,
and fixable as long as it is not frozen. Reading is a new need and deserves its
route; writing is not.

**Why not the other forms of provenance.** `?with_source=true` would give two
response shapes to one route for a need that exists only in the editor.
`source_*` fields on `SongItem` would weigh down the catalogue's central type
for information that is rarely read — and `SongItem` feeds the Subsonic facade's
`song_node()`, frozen for `v2.0-beta`.

### Two prerequisites before writing a line

**Audit the desktop client.** It lives in
[`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow), outside this
repository, and it is the only possible consumer of the current semantics: the
web client has only two `PATCH`es, on playlists and on accounts, and nothing in
`docs/` declares `/api/v2` frozen — the freeze covers the Subsonic facade and
RFC-003's synchronization contracts, which this route is not part of. If it
depends on whole-set replacement, it is migrated at the same time or the contract
is versioned. Changing the semantics without having looked would be trading one
silent data loss for another.

**`Option<T>` cannot say the three states.** `TrackMetadataPatch` declares
`pub title: Option<String>` in bare serde, and an **absent** field and a
**`null`** field both deserialise to `None` there: the tri-state is not
expressible in that shape. `Option<Option<T>>` with a `deserialize_with` that
tells the two apart is needed. Failing that, one writes the contract, tests that
"absent touches nothing", and `null` behaves identically with no test noticing.

## A reminder of scope

Nothing in this plan touches the Subsonic facade, frozen for `v2.0-beta`, nor
does it come before this version's one remaining lock: replaying Symfonium,
Feishin and DSub against the current model, as Juliet was on 2026-08-31. The web
client is not on that critical path.

## What is still open

- ~~**The library selector.**~~ **Settled on 2026-09-07: one active library at a
  time.** The contract, in one sentence: *the web interface always works inside
  one library; changing it changes the catalogue's scope, it does not merge
  catalogues.* No "all libraries" option in v2.

  It was not the filter's screen that was costly, it was the semantics behind
  it: two libraries carrying the same record immediately pose four questions —
  an album shown twice, or one album with two provenances, or two editions, and
  which supplies the cover, the year, the tags. It is the identity problem
  `src/pid.rs` solves for files, posed again at the presentation layer. The
  desktop client may unify its sources; the server's interface is the view of
  **one** server catalogue, and does not have to reproduce the client's
  presentation model.

  Two details to respect: hide the selector when only one library is reachable,
  and remember the last one chosen.

- ~~**What is shown of a tag correction.**~~ **Settled on 2026-09-07: the
  effective value everywhere, provenance in the editor alone.** See the contract
  below — the question turned out to be an API evolution and not an interface
  rule.
- ~~**The fate of the serif.**~~ **Settled on 2026-09-06: a system.** And the
  question rested on a false premise — see the corrected move above. All that
  was left was to name the two stacks instead of repeating them.
- ~~**The emptiness of the sidebar.**~~ **Closed by batch B**, without having
  been addressed for itself: the four navigation entries added — genres, now
  playing, random, recent plays — occupy the space batch A had left empty. That
  was the right order: the emptiness wanted content, and the content ended up
  existing.
- **CodeQL on `tests/**`.** **Settled on 2026-09-07: the analysis stays.** The
  workflow already carries an inline `config:`, but CodeQL's configuration does
  not cross path and rule: one can ignore `tests/**` for *all* rules, or filter
  a rule over *all* paths — the intersection "this rule, in the tests only" is
  not expressible. Cutting `tests/**` would lose the signal on test helpers,
  fixtures and reusable setup code, which is far wider than the false positive
  aimed at. The known occurrences are therefore dismissed by hand. If this ever
  became thirty alerts a week, the question would reopen.
- ~~**Languages.**~~ **Settled on 2026-09-07: nothing for now.** Two languages ×
  171 keys do not justify a rework. The rule is laid down in advance: **adding a
  third language triggers converting the dictionaries into modules loaded on
  demand**, the active language plus English as a fallback. Optimising today for
  thirty-four languages that do not exist would be premature; carrying on with a
  static `Record` the day they exist would be a mistake.

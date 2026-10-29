# The interface and the missing features: a survey and a plan

> Survey of 2026-09-15, taken read-only against `main` at `746e0b1`.
> Two sources, distinct and marked as such: a session in front of the real
> interface — a local instance served by the release binary, an existing
> database, a private window with no extensions — and a reading of the code to
> find the causes. Every figure comes from the repository; what was not verified
> says so.
>
> No code was modified during this survey.
>
> **Status as of 2026-10-29, `main` at `4ccf395`:** every issue in §5 is closed
> — #206 and #207 by #220 on 2026-09-15, #208 to #213 on 2026-10-11 (#245,
> #243, #244, #246, #247, #250), #214 to #217 and #222 on 2026-10-12 (#251,
> #252, #253, #258, #254). The artist page's biography and similar artists
> came with RFC-011 (#270 to #276), smart playlists and podcasts with RFC-025
> (#333). Still open: per-user transcoding profiles, animated artwork, and in
> the Subsonic façade jukebox, chat, video, upload and the legacy `search`.

## 1. Two real defects, found in the console

These are the only observations from this session that describe faulty
behaviour. The rest of the document describes work not done, which is not the
same thing.

> **State on 2026-10-29.** Both fixed by #220 (`ccb3ae5`, 2026-09-15), which
> closed #206 and #207.

### The canvas ticket is asked for again on every mount

`POST /api/v2/tracks/{id}/canvas-ticket` answers `404` for a track with no
canvas, and the client **asks again endlessly** — once per mount of the panel,
visible as a burst in the console.

The client handles the case correctly, though: `canvasUrl`
(`webapp/src/api/canvas.ts`) catches the `404` and returns `null`. No exception is
raised; what the console shows is the request itself, which the browser logs
whatever the JavaScript does with it.

The cause is in `canvasQuery` (`webapp/src/api/queries.ts:231`): `staleTime: 0` and
`gcTime: 0`. Those two zeros were placed **on purpose** by PR #204 — a canvas
ticket expires, so it must never be served from a cache. But they do not
distinguish two answers of opposite nature:

- **a ticket**, which goes stale and must not be kept;
- **"this track has no canvas"**, which does not go stale within a session.

**Expected fix**: remember the negative answer for the session, and only that
one. Never keep a live ticket. It is exactly the family of the looping
favicon — a stable answer asked for indefinitely.

### A dead session polls the server for ever

`GET /api/v2/now-playing` answers `401`, and starts again.

`nowPlayingQuery` (`webapp/src/api/queries.ts:125`) carries
`refetchInterval: 30_000`. And `call` (`webapp/src/api/client.ts`) retries **once**
after a successful `refresh()`. So a `401` that reaches the console means the
refresh itself failed: the session is definitively dead.

**Consequence**: a `401` every thirty seconds, indefinitely, without the user
being taken back to the sign-in screen. The application stays on display, empty,
polling a server that keeps saying no.

**Expected fix**: a `401` that survives the refresh must stop the interval and
hand back to the sign-in screen, not loop.

### What, in the same console, is not a defect

- **`GET /favicon.ico 404`, once.** That is the intended behaviour, and it
  confirms PR #205 under real conditions: no more loop, no more typo served as
  an image.
- **`Cannot read properties of undefined (reading 'startTime')` in
  `reportAllChanges`.** That is the **web-vitals** API, which is neither in
  `package.json` nor in `bun.lock` — the client's five dependencies are `react`,
  `react-dom`, `@tanstack/react-query`, `@tanstack/react-router` and
  `@noble/hashes`. The faulty code is injected (`VM227`, anonymous source), so it
  comes from the browser's tooling, not from the client.
- **`ERR_CONNECTION_REFUSED`.** The binary was being rebuilt.

## 2. The interface: what the session asks for

### The server's mark is not WaveFlow's

The two identities have diverged, and not over a detail. The reference logo is
the neighbouring repository's, `assets/logo.svg` from
[InstaZDLL/WaveFlow](https://github.com/InstaZDLL/WaveFlow):

|  | Desktop (the reference) | Server (current) |
|---|---|---|
| Bars | **five** | four |
| Profile | **symmetric** (140/80/40/80/140) | asymmetric, rising then falling |
| Colour | **diagonal gradient** `#34D399` → `#10B981` → `#059669` | flat `#34d399` |
| Container | **none**, the bars are the logo | bars knocked out of a solid rounded square |

**And the server's mark is copied in three places, none of which is an SVG**:

- `webapp/src/styles/styles.css:308` — `.brand-mark`, drawn in CSS;
- `webapp/src/main.tsx:120` — the four `<i>`s that CSS shapes;
- `webapp/public/favicon.svg` — the only vector version, written by hand from
  the previous two.

**Expected fix**: adopt the desktop's identity, and above all **stop redrawing
it**. One SVG, referenced by the sidebar and by the tab alike, removes the very
possibility of a new divergence.

**Two reservations to handle, not to ignore**:

1. **Do not copy the file as it is for the favicon.** It is rendered at 16 px:
   five thin bars with no container, on a gradient, lose their contrast against a
   light tab bar. The server's rounded square had that reason — it makes the mark
   findable in a row of tabs. To be arbitrated: keep a container **for the
   favicon alone**, carrying over the five-bar profile and the gradient.
2. **The favicon is deliberately not themeable** (see the comment in
   `webapp/public/favicon.svg`). An icon that changes with the player's theme is
   harder to find again, not easier. The gradient does not reopen that choice;
   the container, if it is kept, stays solid.

### Gather the settings, and show the registered clients there

The sidebar stacks thirteen navigation entries, then the theme, then the
language, then the account. Theme and language are not destinations: they are
settings left in a corridor.

Expected: a **Settings** page gathering the theme, the language, the scrobbling
(today at `/settings/scrobbling`, already under that prefix) — and showing **the
registered clients**, WaveFlow's as well as Subsonic's. The material already
exists and is visible nowhere: the API tokens are buried in a disclosure on the
Administration page, and the dedicated Subsonic password only appears beside
them. These are devices authorized to read the library; they deserve a view that
says which ones, since when, and a button to revoke them.

### The first load

Asked for: in place of the skeleton, a careful loading screen carrying the mark,
one to two seconds, then a direct load once cached.

**A reservation, and it is important**: do not impose a one-to-two-second floor.
A floor makes the application slower for everybody, permanently, to hide a load
that takes 79 ms. What is needed is a brand screen **shown during** the load and
removed as soon as it is over, with a minimum of around 300 ms to avoid a
flicker. The difference between the two is not cosmetic: one dresses a wait, the
other manufactures one.

### Fake slowness: no

Proposed in the session, out loud, with the hesitation that goes with it.

**Recommendation: do not do it.** The impression that "it loads too fast to be
true" does not come from the speed, it comes from **the absence of a drawn
transition**: the content substitutes one picture for another, with nothing
linking the two states. The answer is a transition — a short fade, a slide — not
a `setTimeout`. Added slowness is a regression that can no longer be removed
without it being noticed.

### Live search

The Search page requires a click on a button. Expected: the search starts as one
types.

To be done with a debounce (around 250 ms) and cancellation of the previous
request, or every letter is a round trip and the answers arrive out of order. The
button can stay for keyboard submission.

### The artist pages

Today: a name and albums. Expected: the desktop client's level — biography,
similar artists.

**This is not client work, and it is the most underestimated point of the
document.** The session first concluded that "the server already knows how to
answer" because the methods are routed. They are, and they answer nothing:

| Method | What it actually returns |
|---|---|
| `getTopSongs` | an empty container (`src/subsonic/mod.rs:401`) |
| `getSimilarSongs` | an empty container (`:402`) |
| `getSimilarSongs2` | an empty container (`:403`) |
| `getArtistInfo`, `getArtistInfo2` | `artist_info` looks the artist up **for its refusal alone** — the 404 semantics — then returns an empty container (`src/subsonic/browse.rs:79`) |
| `getAlbumInfo`, `getAlbumInfo2` | the release identifier if it exists, nothing else |

And the reason is written in the code, as a policy and not as an omission:
**"WaveFlow queries no remote source, so notes and biography images stay
absent."** There is no biography and no similar artist **anywhere** — not in the
database, not in the services, not in a migration. At Navidrome that data comes
from Last.fm and MusicBrainz.

**The consequence for the estimate**: an artist page at the desktop's level
requires a capability that does not exist — **an external metadata source**, with
its cache, its rate limiting, its degradation when unreachable, and a privacy
question to settle (a self-hosted server querying a third party about its
owner's library, by default or on consent). That is a matter for an **RFC**, not
for an interface ticket.

**And the answer is probably already designed, elsewhere.** The
[InstaZDLL/waveflow-plugins](https://github.com/InstaZDLL/waveflow-plugins)
repository describes **sandboxed WebAssembly components**: one portable
`plugin.wasm`, run under wasmtime with a fuel and memory budget, and an **HTTP
allow-list declared in the manifest and enforced by the host** — a plugin reaches
only the hosts it asked for. The registry pins the version and the `blake3`. Two
such plugins already exist, `apple-artwork` and `spotify-canvas`, and the README
claims the same file runs "on Windows, macOS, Linux, **and the server**".

**The server knows nothing of it**: no occurrence of `plugin` under `src/`. So
the right RFC is probably not "the server calls Last.fm" but **"the server hosts
the plugin engine"** — permission, the sandbox and the privacy question are
already handled there by construction, and a metadata source becomes one plugin
among others rather than one more dependency in the binary. That is more
ambitious and more correct; to be settled in the RFC, not here.

What remains doable right away and with no external source: an artist's **most
played tracks**, which local history can compute — which fills `getTopSongs`
along the way.

> **State on 2026-10-12 (#215).** Done, and the table above loses its first
> line: `getTopSongs` no longer returns an empty container.
>
> **What is computed is not what the reference computes.** The reference server
> asks a remote service what is popular; this one counts what **this account**
> listened to, in `play_event`. It is a narrower question, and the only one the
> data supports. Said that way, the rest follows: a track never played is
> **absent** and not last, two members of one library get two answers, and a
> fresh account gets an empty list — which is the true answer, not a missing
> feature. That is why the artist page only shows the section when there is a
> history: a heading over an empty table would read as a failure.
>
> **A track counts for an artist in two ways**: it credits them in its own
> `artist` role, or its album is credited to the artist as `albumartist` — the
> same `album_participant` relation the page already draws its albums from.
> Without the second, a record whose files credit the performers track by track
> would have a page full of albums and an empty list.
>
> **A latent defect found while filling the container**: `topSongs.song` came out
> of the JSON renderer as an **object** as soon as there was only one track.
> Nothing had ever told the renderer otherwise, because the container had never
> held anything — while the frozen contract says "JSON collections are arrays
> even with one item". The test found it before the review.
>
> **What stays empty, and why**: `getSimilarSongs`, `getSimilarSongs2`,
> `getArtistInfo`, `getArtistInfo2`. None of that can be computed without an
> external source, and it is **#218** — the plugin-engine RFC — that settles the
> source, not this ticket.

> **State on 2026-10-29.** The rest of the table is filled too. #218 was
> settled by [RFC-011](../rfcs/RFC-011-metadata-enrichment.md) (#262,
> 2026-10-14) as native enrichment with no plugin engine; `getArtistInfo` and
> `getArtistInfo2` read its cache since #272, `getSimilarSongs` and
> `getSimilarSongs2` draw from the artist and their neighbours since #273 and
> #274 (all 2026-10-15), portraits came with #275 and the artist page with #276
> (2026-10-16). Biographies stay empty until an operator turns enrichment on,
> which is off by default. Album notes (`getAlbumInfo`) are still not enriched,
> one of RFC-011's open questions.

### What the server already knows, and does not say

The technical metadata is **stored and already projected in SQL** — `codec`,
`bitrate`, `sample_rate`, `bit_depth`, `channels`, `duration_ms`, `size` appear
in the migrations and in the `song_select!` projection.

But **they do not come out**: none appears in `src/api/`'s responses, and
the client under `webapp/src/api/` does not know them. So file information and a quality bar
**require no migration** — only serialising them, extending the client type, and
drawing. It is the document's biggest saving.

Symmetrically, a reservation about the Settings page: the tokens are listed only
by an **administration** route —
`/api/v2/admin/users/{username}/tokens`. Showing a user **their own** registered
clients therefore requires a non-administrator route, scoped to self. Small, but
it is server work.

### What one cannot see of a file

Absent from the web interface, present on the desktop:

- **the file's information** — container, codec, bitrate, sample rate, bit
  depth;
- **a quality bar** summarising it at a glance;
- **a lyrics view**;
- **an immersive view**.

Lyrics are a case apart: the server serves them (`getLyrics`,
`getLyricsBySongId`), the client reads them on "Now playing", but there is no
dedicated view.

> **State on 2026-10-11 (#213).** Both views exist. "Now playing" carries three
> modes, held by the **location** and not by component state: `detail`
> (unchanged), `?view=lyrics` and `?view=immersive`. The reason for the search
> parameter is the Back button — on a phone it is the press everybody makes to
> leave whatever they are in, and a mode kept in `useState` would have swallowed
> it to leave the full-screen view.
>
> The lyrics view adds two things the strip could not give: the sung line is
> **held in the centre**, and where the server supplies timings each line is a
> **button that seeks to it**. A sheet with no timings gets neither, and says so
> by having no buttons.
>
> The immersive view is a `<dialog>` opened by `showModal` — the focus trap,
> Escape and the inertness behind come from the platform, as they do for the
> navigation sheet. It **carries the transport controls**, because it covers the
> player bar: a view that fills the screen and leaves no way to pause is a trap
> and not a mode.
>
> Along the way, `PlayingPage` left `pages.tsx` for its own module — 2,301 lines
> against 1,238 for the three page modules together — and `EmptyState` joined
> `page-shell.tsx`, where `Loading`, `Waiting` and `PageHeader` already lived.

### Spacing, and the "cardboard"

The session's observation: menus too tight, and an interface that "looks like
cardboard" — stacked rectangles, with no hierarchy and no breathing room. The
reference cited is TPI-Flow.

This point is not a list of fixes: it is a design pass. It deserves being
treated as one — a survey of the spacings, sizes and contrast levels actually in
use, then a chosen scale, rather than a touch-up screen by screen.

**A clue as to where it is decided**: `webapp/src/styles/styles.css` has **two naming
systems** living side by side. On one side roles — `--panel`, `--panel-soft`,
`--panel-strong`, `--line`, `--line-strong`, `--muted`; on the other surfaces
named after their appearance — `--color-surface-dark`, `--color-surface-light`,
and their `-elevated` variants. Two systems for the same thing is precisely what
produces a "cardboard" screen: no rule says which to use any more, so every
screen decides for itself. There is **no spacing scale at all** — no variable,
no step — when a scale is exactly what this observation needs.

> **State on 2026-10-11 (#214).** Two things measured, and the first contradicts
> what precedes it.
>
> **The two naming systems are not competing, they are layered.**
> `--color-surface-*` and `--accent-*` are written **at runtime** by
> `applyTheme` (`webapp/src/styles/design-tokens/apply.ts`) from a `ThemePreset`;
> `--panel`, `--panel-soft` and `--panel-strong` are `color-mix()`es **derived**
> from them. So there is a rule after all: a theme writes surfaces, a screen uses
> roles. And it is respected — every `--color-surface-*` usage is inside the
> blocks that define the tokens, no screen crosses the layer. The npm package
> `@waveflow/design-tokens` is not a dependency, but **a module of the same name
> has lived in the tree** since 2026-08-14 and it is what drives the theme.
>
> **The absence of a scale, on the other hand, was real and worse than
> described.** Surveyed: **49 distinct spacing values and no variable at all**,
> 24 of them used once or twice; and **37 font sizes**, of which **seven between
> 0.70 and 0.86 rem** — the same size spelled seven ways, which is exactly why
> nothing looked more important than anything else.
>
> Laid here rather than in a package: nine spacing steps on a quarter-rem grid,
> seven text steps and three display ones. Ties resolve **upwards**, since the
> complaint was tightness. 267 literals carried onto the scale: 96 widened, 87
> tightened, **a net +4.42 rem**. A `rem` in a width, a height, a radius or an
> offset is left alone — that is a dimension, not a step — and
> `webapp/src/styles/styles.test.ts` holds the rule rather than a list of exceptions:
> inside the scale's range, the scale is what is used.
>
> What is **not** done: the taste half of a design pass. Judging a hierarchy
> takes eyes on the result, and the agent has none.

Issue #63 (closed) already spoke of unifying those surfaces onto a
`@waveflow/design-tokens` package in OKLCH. That package **is not a dependency
of the client** and the CSS contains no `oklch`. To check before choosing: adopt
that package, or lay the scale here.

## 3. What is missing, feature-wise

### In the web client

- **No "Songs" list.** One browses by album, artist, genre, favourites, history,
  random — never "all my tracks, sorted". The component exists (`SongTable`,
  used in five places); the route does not. It is the gap a user coming from
  Navidrome notices first.

  **And the server cannot answer either.** Issue #179 said that
  `GET /api/v2/songs` required a `genre` and could therefore list nothing; it
  was closed on 2026-09-10 **by renaming the route** to
  `/api/v2/songs/by-genre`. The contract became honest, the feature did not
  appear: only `by-genre` and `random` remain. The "Songs" list therefore
  requires **a new server route** — paged and sortable — before any page. That is
  a correction to the estimate: this point runs across the thread.
- **No `.m3u` import.** Nothing in the repository. It is what lets somebody
  arrive with an existing collection: a barrier to adoption, not a convenience.

  > **State on 2026-10-12 (#216).** Done, in both directions, and one premise of
  > the issue turned out to be impossible.
  >
  > **"Path first, then the content hash, like the scanner"**: the second rung
  > cannot exist here. The scanner holds the file and can hash it; a playlist
  > entry hands over a path and, at best, a display title. The last resort is
  > therefore the **`#EXTINF` credit**, matched exactly on `Artist - Title`, and
  > only when it names one track. An entry whose path moved *and* whose credit
  > was never written is **reported**, not lost in silence.
  >
  > **The path comparison is symmetric, and my first version was not.** An entry
  > can be longer than the stored path (absolute, on somebody else's disk) but
  > just as easily **shorter**: a `.m3u` written in the album folder names
  > `01 Hidden Place.flac` where the catalogue stores
  > `Björk/Vespertine/01 Hidden Place.flac` — and that is the common case. Asking
  > SQL for the stored path to equal one of the entry's suffixes answers the
  > first case and never the second. The file name therefore selects the
  > candidates, and the deepest agreement on the parent folders decides. **The
  > test found it before the review**, because it carried a file name shared by
  > two albums.
  >
  > **The export is in the same pull request**, which the issue left to decide.
  > The reason is not that it is small: it is that the round trip — export, then
  > import again, and resolve everything — is the strongest evidence the reader
  > and the writer agree, and it costs one assertion. It is also **the one
  > `/api/v2` route that does not answer JSON**: the resource *is* a file.
  >
  > **The panel asks before it writes.** A file from elsewhere names paths from
  > elsewhere, so some of it will always be missing. A playlist that silently
  > arrives shorter is worse than a refused import, so the first gesture is a dry
  > run: the server resolves everything, creates nothing, and says what it could
  > not place and why.
  >
  > **A defect found next door and not fixed**: `create_playlist` with the same
  > identifier twice violates `playlist_track`'s primary key and answers 500. The
  > import cannot fall into it — it deduplicates and reports the duplicate — but
  > the creation route stays exposed. Its fix is its own.
  >
  > **State on 2026-10-29.** The 500 is gone: RFC-020 (#327, 2026-10-25) allows
  > a track twice in a playlist, and `playlist_track` is now keyed by position
  > (`migrations-v2/20261026000000_playlists_and_shares.sql`).
- No smart playlists, no transcoding profiles.

  > **State on 2026-10-29.** Smart playlists exist since RFC-025 (#333,
  > 2026-10-25). Transcoding profiles are **still absent**: the server has only
  > its concurrency ceilings, and no per-user format or bitrate.

### In the Subsonic facade

Fifty-nine methods were served when this was surveyed, and the three radio
writes of #217 bring it to sixty-two — a count carried forward rather than
recensused. The absences, with what the server says about them:

| Absent | Declared role | State |
|---|---|---|
| Jukebox (`jukeboxControl`) | `jukeboxRole: false` | nothing |
| Podcasts (7 methods) | `podcastRole: false` | nothing |
| Chat (`getChatMessages`, `addChatMessage`) | `commentRole: false` | nothing |
| Video (`getVideos`, `hls`, `getCaptions`…) | `videoConversionRole: false` | nothing |
| Upload through Subsonic | `uploadRole: false` | exists on `/api/v2`, not exposed |
| `search` (legacy v1) | — | `search2` and `search3` are served |

**None of them is a lie**: every role is declared `false`, so a client does not
offer the button. Radio was the only one that went further — an empty container
says "zero stations" and not "unsupported", which is the only choice the
protocol leaves, so a client could not read the absence as one.

> **State on 2026-10-12 (#217).** Radio has left this table. The four methods
> are served against a `radio_station` row per account, so the container now
> says what it always claimed to: the number of stations this account keeps.
>
> **The question the issue asked to settle first — proxy or hand over — is
> settled as hand over.** The server validates an address as absolute `http` or
> `https`, stores it as typed and never connects to it. A server that fetched a
> user-supplied URL would be a request forger aimed at whatever it can reach
> from the inside, and refusing that role is what leaves the feature with no
> allowlist, no timeout and no egress policy to get wrong.
>
> **Two deviations from the reference server**, both deliberate and both now in
> `docs/compatibility/subsonic-compatibility.md`: the list is per account rather than global
> and administrator-only, because WaveFlow has no global library setting of any
> kind; and `homePageUrl` is returned with a capital `P` while `homepageUrl`
> sets it, which is the specification's own asymmetry.
>
> **And the JSON trap that #215 found waited here too.** `json_array_field` is
> an allowlist, so `internetRadioStations.internetRadioStation` had never been
> declared a collection — a container that has never held anything cannot have
> been noticed. It is declared now, asserted at exactly one child, which is the
> only length that tells an array from an object.
>
> **One limit, stated rather than hidden**: the web page plays a station from
> its own audio element and stops it when the page is left. The queue player
> assumes a track with a duration to seek in, an id to persist and something to
> scrobble, and a stream has none of the three; carrying radio into the player
> bar is a transport change, not a wiring change.

> **State on 2026-10-29.** Podcasts have left this table too: RFC-025 (#333,
> 2026-10-25) serves the eight podcast methods, one list per instance, and
> `podcastRole` is now set per account. That transport change was made as well:
> web radio and podcast episodes play through the play bar since #338
> (2026-10-26). By a count of the method names in the dispatch
> (`src/subsonic/mod.rs`), 72 are now routed, `getAvatar` among them answering
> "not found". **Still absent**, each still declared `false` or unrouted:
> jukebox, chat, video, upload through Subsonic, and the legacy `search`.

### In the server

- **Nothing is compressed.** `tower-http` is compiled with `cors`,
  `request-id`, `timeout`, `trace`; no compression layer exists. The client
  leaves raw: **497 kB instead of about 152 kB**. One feature and one layer. No
  gain on a local network, decisive from outside. **State on 2026-10-29:** done
  by #243 (#209, 2026-10-11), one `CompressionLayer` in `src/app/router.rs`.
- **Animated artwork does not exist.** The video canvas, on the other hand, is
  delivered (RFC-009) — the two are regularly confused. Still true on
  2026-10-29.
- **External scrobbling is complete in the code**: `lastfm`, `listenbrainz`,
  `maloja`. What blocks Last.fm is registering an application with them, not a
  line to write.

### In the documentation: two languages inside one series

The repository is public and under AGPL, and its design documentation is written
half in English and half in French — **including inside the RFC series**, which
is the hardest point to defend:

| | Files | Lines |
|---|---|---|
| **RFCs in French** | 004, 007, 008, 009, 010 | **2,692** |
| **RFCs in English** | 002, 003 | 299 |
| **Analyses and plans, in French** | `web-client-gap-analysis`, `opensubsonic-gap-analysis`, `desktop-v2-integration-gap`, this document | **1,595** |
| **Archives, in French** | `M4-handoff`, `M3-symfonium-validation` | 602 |

Everything else is already in English: `README.md`, `CLAUDE.md`, `AGENTS.md`,
the two API guides, the compatibility matrix and the **eight handoffs**. So what
faces outward already speaks English; what explains the decisions does not.

**Two reservations, because a botched translation is worth less than the
original.**

1. **An RFC is worth what its precision is worth.** RFC-010 is a thousand lines
   of design reasoning, with numbered decisions the text refers back to ("by
   decision 11"), `Implémentée par` lines naming pull requests, and
   cross-references. Translating by the yard breaks all of that in silence. The
   work is done document by document, re-reading the references.
2. **The two archives are a case apart.** `M3-symfonium-validation` dates a
   campaign to 2026-08-23 and `M4-handoff` recounts a milestone that closed.
   Translating an archive is rewriting a record. To be settled separately:
   leaving them as they are is defensible, translating them too, but not without
   saying so.

> **State on 2026-10-12 (#222).** Done, and this section now describes a state
> that no longer exists: **the design documentation is in English**, the five
> RFCs and the four analyses included — this document among them.
>
> **The header keys travelled with the prose.** `Statut` became `Status`,
> `Implémentée par` became `Implemented by`, and `Auteurs`, `Dépend de`,
> `Révisée`, `Préalable de`, `Rend possible` and `Clôt` likewise, to the
> spelling RFC-002 and RFC-003 already used. That is what makes the series one
> series rather than two halves — and the paragraph above, which names the
> French key, is kept as it was written because it dates a state rather than
> describing one.
>
> **It closed a mismatch rather than opening one.** The code comments already
> said "RFC-010 decision 4", "decision 5", "decision 13" in English, against
> documents that said `décision`. The numbering is untouched — translating in
> place cannot move it — so every reference from the code now lands on the words
> it quotes. `CLAUDE.md` says which language documentation is written in, which
> is what let the split open at all.
>
> **Nothing was summarised, corrected or improved on the way through.** A stale
> statement stays stale, an addendum keeps its date, the six self-corrections of
> `web-client-gap-analysis` keep their count, and the two documents carrying a
> *kept as written* banner keep it. What a dated record says is evidence about
> that date; rewriting it is the one change a translation must not make.
>
> **And the two archives were left in French**, which was reservation 2's
> question. They are records: `M3-symfonium-validation` dates a client campaign
> and `M4-handoff` recounts a closed milestone, and translating either rewrites
> what was written then. Each now carries a banner saying so, so that the choice
> reads as a decision rather than as an oversight. Reversing it costs two
> translations and no risk; it is not this pull request's to take.

### A structural gap, with no visible effect

Four screens — the tag editor, the canvas panel, the upload, the scrobbling —
import `pages.tsx` for **three primitives**: `Loading`, `PageHeader`,
`useReload`. They thereby pull in 70 kB of module and, transitively,
`player.tsx`.

**Measured, and counter-intuitive**: extracting them and then splitting by route
would save **about 18 kB out of 466** (4%). React and TanStack weigh **276 kB,
59% of the bundle**, and are required at the first byte whatever the page. The
extraction is worth it for the cleanliness of the module graph, **not for
speed** — and a split by route must not be sold as an optimisation.

## 4. An order, if one is to be chosen

1. **The two defects of section 1.** They are faulty behaviours, not missing
   features, and the second leaves a dead session on screen.
2. **The mark**, **live search** and **the "Songs" list**. Little code, highly
   visible; the list already has its component, and the mark is the one point of
   the document that is an identity divergence rather than a gap.
3. **Server compression.** One feature, one layer, one test.
4. **The Settings page**, registered clients included — it tidies away what is
   lying about and exposes what is hidden today.
5. **The brand loading screen** and the transitions. With no artificial floor.
6. **An artist's most played tracks**, computed from local history — the one
   half of the artist page that needs no external source, and it fills
   `getTopSongs` along the way. **Biography and similar artists leave this
   list**: they require an RFC on an external metadata source. *(That RFC is
   RFC-011, carried out by #270 to #276 on 2026-10-15 and 2026-10-16.)*
7. **The design pass** — spacings, hierarchy. To be treated as a subject, not as
   a list.
8. **File information, a quality bar, a lyrics view, an immersive view.**
9. **`.m3u` import**, then internet radio. *(Both done on 2026-10-12, #216
   and #217.)*
10. Smart playlists, jukebox, podcasts: real projects. *(Smart playlists and
    podcasts done on 2026-10-25, #333; jukebox still open.)*

## 5. The split into pull requests

Eleven pull requests, plus an RFC. The sizes use the repository's label scale
(`size: xs` < 10 lines, `s` 10-50, `m` 50-200, `l` 200-500, `xl` > 500) and
remain **estimates**, tests included.

| # | Issue | Title | What it carries | Size |
|---|---|---|---|---|
| 1 | **#206**, **#207** | A stable answer is not asked for again | The two defects of section 1 | `m` |
| 2 | **#208** | One mark, and a screen that carries it | The desktop's identity, three copies reduced to one SVG, the loading screen and the transitions | `l` |
| 3 | **#210** | Let the catalogue be browsed | A server route listing the tracks, a "Songs" page, search as one types, the three primitives extracted out of `pages.tsx` | `l` |
| 4 | **#209** | The server compresses what it sends | One `tower-http` feature, one layer, one test | `s` |
| 5 | **#211** | What a file is, and a bar that says it | Serialise five already-projected fields, extend the client type, draw | `l` |
| 6 | **#212** | The settings stop lying about in a corridor | A Settings page, registered clients, a self-scoped token route | `l` |
| 7 | **#213** | The lyrics and the immersive view | Two views on "Now playing" | `l` |
| 8 | **#214** | The design pass | A spacing scale, one naming system, a hierarchy | `xl` |
| 9 | **#215** | An artist's most played | Computed from local history; fills `getTopSongs` | `l` |
| 10 | **#216** | `.m3u` import | Parsing, path resolution, server and client | `xl` |
| 11 | **#217** | Internet radio | A migration, three Subsonic writes, `/api/v2`, a page | `xl` |
| 12 | **#222** | One language for the documentation | The five French RFCs and the four analyses move to English; the fate of the two archives is settled separately | `xl` |

**Outside the pull requests — one RFC, tracked by #218**: the server-side plugin
engine, which biography and similar artists depend on. See the "artist pages"
section.

The first row carries **two** issues because it carries two distinct defects: an
issue describes a problem, a pull request may close several.

### The order, and what constrains it

- **1 to 4 depend on nothing** and can go together. Two real defects, the
  identity, the two most visible gaps and the document's only transfer gain.
- **7 assumes 2**: the immersive view reuses the brand screen.
- **8 comes after 3, 5, 6 and 7**, or the pass is redone over markup that has
  changed in the meantime.
- **9 is the one half of the artist page** that does not wait for the RFC.
- **12 depends on nothing and blocks nothing**, but is better done early: every
  document written before it lands adds to what there is to translate, and this
  document is one of them.

### Two groupings refused, and why

The instruction was to group as much as possible. Two exceptions, accepted:

1. **Compression stays on its own** (4) rather than joining `.m3u` or the radio,
   the other Rust work. Grouping it would make forty high-yield lines wait
   behind a project of several hundred.
2. **5 and 7 stay separate** although they touch the same screen. 5 **changes
   the shape of the wire**, 7 does not. Mixing a wire change with interface work
   is exactly what let PR #203's four defects through: the mocks confirmed the
   types instead of contradicting them. A pull request that touches the wire
   deserves its own look.

## What was not verified

One point only, observed on screen and **not investigated**: searching "Tz"
returns two albums, `abouTZU` (credited `TZUYU, PENIEL`) and `abouTZU - EP`
(credited `TZUYU`), and the EP shows two tracks numbered 3 and 6. That may be
perfectly accurate — two distinct releases, a partial extraction — or the sign of
an album identity splitting on the credit. Since album identifiers are derived
from the tags (`WAVEFLOW_PID_ALBUM`), the question is settled by reading the
files' tags, not by reading the screen. To be done before concluding anything
from it.

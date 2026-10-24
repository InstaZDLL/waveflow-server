# RFC-015 — Splitting a joined artist credit, and a page for what needs a decision

- **Status**: Proposed
- **Implemented by**: in progress, in the order of "The order of the work"
  below: [#315](https://github.com/InstaZDLL/waveflow-server/pull/315) (this
  RFC). The *Status* field above never flips in this project; this line is the
  one that says what is running.
- **Date**: 2026-10-24
- **Revised**: 2026-10-24, after review on #315. A split's provenance is a
  relation (`track_participant_source`) rather than one column, because one
  part can come from several credits. Sort forms are paired before the
  fragments are deduplicated. And an undo finds its rows by `credit_key`.
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (identity, the
  frozen Subsonic contract), [RFC-003](RFC-003-waveflow-sync-v2.md) (the user
  journal), [RFC-007](RFC-007-library-event-stream.md) (the library feed),
  [RFC-012](RFC-012-library-browser.md) (corrections that belong to the
  library), [RFC-013](RFC-013-chosen-artist-imagery.md) (choices keyed by the
  artist row)
- **Amends**: nothing. No identity rule moves: decision 4 says why a split
  re-identifies only the artists it names, and never an album or a track.

## Problem

Streaming-era files often write several performers into one `ARTIST` value
with a comma: `Ashreveal, Ashish Swargiary`, `Zedd, Alessia Cara`,
`Tom Morello, grandson, Roman Morello`. The server cuts a single-valued
`ARTIST` on `" / "`, `" feat. "`, `" feat "`, `" ft. "`, `" ft "` and `"; "`
(`src/tags.rs`, `ARTIST_SEPARATORS`), never on a comma, so each of those
values becomes **one artist row** named after the whole string. On the
instance that found this, 48 of 161 artists are such rows. They have no
albums of their own, no portrait and no banner, since neither Deezer nor
TheAudioDB knows anyone by that name. And they hide the real artists: the
`Ashreveal` who also has solo tracks in the same library does not show this
one.

A comma is not a separator a server may cut on by itself. `Tyler, The
Creator`, `Earth, Wind & Fire` and `Crosby, Stills, Nash & Young` are single
artists, and nothing in the tags says which kind a given value is.

The desktop app (`InstaZDLL/WaveFlow`) answers this with a person, not a
rule, and this document brings that answer to the server.

## What the desktop does

Read at the revision the server pins (`waveflow-core` 1.9.1):

- **The scanner splits on `"; "` and nothing else**
  (`crates/core/src/scanner/upserts.rs`, `split_artist_name`). Its own comment
  names the comma bands as the reason. There is no exception list, no setting
  and no automatic comma split anywhere.
- **"Needs attention"** (French "À corriger") is the last tab of the Library
  view. It lists fourteen categories (`crates/app/src/commands/inventory.rs`,
  `CATEGORIES`). Thirteen are **tracks** that match a criterion: a missing
  title, artist, album, year, track number or cover, an album whose tracks
  disagree on the year, a compilation with no album artist, a duplicated track
  number or a gap in the numbering, a probable duplicate, and two that only a
  desktop can act on (an untaggable `.dff` file and a probable fake lossless
  file). Each one opens its tracks in the ordinary table, and the user fixes
  them there; the category offers no action of its own.
- The fourteenth is **artists to split** ("Artistes à scinder"). It lists every
  artist whose name, cut on `,`, `，`, `、` or `､`, gives at least two distinct
  fragments and that is credited on at least one available track. Each
  fragment says whether an artist of that name already exists. The rows are
  sorted by the number of known fragments, then by track count, then by name.
  Each row offers **Split** (the button asks for a second click to confirm) and
  **Don't split**.
- **Split** (`crates/app/src/commands/artist_split.rs`) replaces the artist,
  in place, by its fragments on every track that credits it. Each fragment
  takes the role the joined artist had, the positions are renumbered, an
  existing artist of the same name is reused, and the joined row is deleted
  once nothing names it. **The decision is recorded nowhere.** It lives only in
  the rewritten credits, which survive a rescan of an unchanged file. A file
  that changes is read again from its tag, and the joined artist comes back
  (`docs/features/library.md` says so).
- **Don't split** appends the artist's canonical name to a profile setting,
  `inventory.dismissed_phantoms`. Only the list reads it. The scanner never
  does, and nothing in the interface can take a dismissal back.

The server keeps the rule and the two actions, and fixes what the desktop's
own documentation calls a limit: **a decision is recorded, and every scan
applies it**.

## Decision 1 — no automatic split beyond what the server does today

The server's automatic separators stay exactly as they are. That set is
already wider than the desktop's `"; "`. **No comma, ideographic comma or
`" & "` is ever cut without a decision.** An upgrade therefore changes no
credit, no artist id and no album id on any instance. Until an owner decides
something, the catalogue is the one the previous version built.

What *is* new is a second step, applied to each name the tag mapper
produces:

```text
tag value ── tags::credits (unchanged) ──▶ credit names ── decisions ──▶ artists
```

For each credit name, the library's decision for that name, if any, says
whether it stands as it is or is replaced by its parts. A name with no
decision stands. The step is a pure function of the name, its sort form and
the library's decisions. That is what lets decision 5 apply it to the stored
catalogue without reading a file.

## Decision 2 — the fragments, as the desktop cuts them

A credit's **fragments** are its name cut on the desktop's four characters:
`,` (U+002C), `，` (U+FF0C), `、` (U+3001) and `､` (U+FF64). The cut is
unpadded, as the desktop's is, so `蔡依林，周杰伦` and `A､B, C` split. Each
fragment is trimmed, empty fragments are dropped, and duplicates are dropped
by canonical name, keeping the first.

One rule is added to the desktop's: **a comma inside parentheses does not
cut**. This is the rule `split_on` already applies to every other separator
(`src/tags.rs`), and for the same reason: `Ian Asher, Olly Alexander (Years &
Years)` names two artists, and a parenthesis qualifies a name rather than
dividing it. It is reused, not rewritten.

A name is a **candidate** when it has at least two fragments and none of them
has the same canonical name as the whole credit. `Solo,` is therefore never
one.

`" & "` is not among the characters. The desktop does not cut on it, and the
names that carry it (`Simon & Garfunkel`, `Years & Years`) are bands far more
often than commas are. On this instance's list, `TZUYU & pH-1` is the one that
plainly is not a band. "What is still open" takes this up.

## Decision 3 — a decision belongs to the library, and is written down

Splitting or keeping a credit changes how the catalogue is presented to every
member of the library, and nothing in a file. It is therefore a library
correction, like a tag correction (`track_override`) and like RFC-013's
choice: **owners and managers may make it**
(`LibraryRole::may_write_metadata`), and anyone else is answered with the same
`404` a foreign library would give.

```sql
CREATE TABLE artist_credit_decision (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    -- The credit name folded the way the artist identity folds it: lower case,
    -- then `pid::str_clear`. One key per artist row the default spec derives.
    credit_key TEXT NOT NULL,
    -- The name as it read when the decision was taken, for display.
    credit TEXT NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('split', 'keep')),
    -- For a split, the fragments as they were shown and accepted, in order.
    parts TEXT CHECK (
        (decision = 'split' AND json_type(parts) = 'array')
        OR (decision = 'keep' AND parts IS NULL)),
    decided_by TEXT NOT NULL,
    decided_at INTEGER NOT NULL,
    UNIQUE (library_id, credit_key)
) STRICT;
```

- **Keyed by the folded credit, not by the artist id.** An artist id is
  derived under `WAVEFLOW_PID_ARTIST`, and a change of that spec re-derives
  every id (CLAUDE.md, "Identifiers and time"). A decision keyed by an id
  would then be lost along with RFC-013's choices, for no reason. The folded
  name is what the default spec hashes, so the key names exactly the names
  that make one artist row. It also survives any spec change, because it never
  depended on one.
- **The parts are stored, not recomputed.** A split means what the owner saw
  and accepted. If decision 2's characters ever change, a recorded split keeps
  its meaning, and a credit that newly becomes a candidate is simply listed
  again.
- **Per library.** The same joined credit in two libraries is decided twice,
  once by each owner, as RFC-013's choices are. Nothing copies a decision
  across.
- **`keep` is a decision too.** It is what makes the list shrink, and it is
  what the desktop's "Don't split" was trying to be: here it is a row, and the
  undo of decision 7 can remove it.

## Decision 4 — what a split re-identifies, and what it does not

A split moves exactly one kind of identifier: **the artists the credit names**.

- **The parts take derived ids from their own names**, through the ordinary
  `upsert_artist`. An `Ashreveal` who already has solo tracks in the library
  has the id that `Ashreveal` derives, and the split credit lands on that same
  row. This is the point of the feature, and the desktop's "reuse an existing
  row" falls out of derivation without a lookup.
- **The joined artist disappears** once nothing credits it. The end-of-scan
  sweep in `consolidate_catalog_derivations` already deletes an artist row that
  no participant and no album names.
- **Album ids do not move.** An album's id hashes its album-artist **display
  string** (`album_artist_display`). That is the `ALBUMARTIST` tag as written
  when there is one, and otherwise the credit names of the first step joined
  by `"; "`. **The second step never feeds it.** It reads the names as
  `tags::credits` produced them, before any decision, so `Zedd, Alessia Cara`
  with no album-artist tag stays the display, and the identity, of its album.
  The album's `album_artist_id`, which is the first credit and the frozen
  `artistId`, does follow the decision and names the first part. Without this
  rule, every album with no album-artist tag whose artist was split would get a
  new id, and album favourites cannot be carried across (CLAUDE.md says so).
- **Track relocation hints do not move**, for the same reason: `track_pid`
  reads the same display string.
- **A track's `artist_display` does not move.** It is what the file wrote, and
  the Subsonic `artist` and `displayArtist` keep showing it. Only the credited
  list changes: `artists[]`, the participants and the per-artist pages.
  `track_override.artists` is the opposite case: a list someone typed is
  final, and decisions are not applied to it.

### What follows the joined artist, and what does not

- **Favourites and ratings.** `user_star` and `user_rating` name an artist by
  an untyped id with no foreign key, so a swept artist would leave them
  dangling. When a split removes an artist, each account's favourite on it
  moves onto **every part**, and its rating onto every part that account has
  not rated itself. Both moves go through each account's journal (RFC-003), as
  the delete of the old `favorite` or `rating` and the upsert of each new one.
  These are the entity kinds the journal already admits, so a desktop mirror
  converges without a contract change. This is unlike
  `remap_artist_user_data`, which moves rows silently after a spec change. That
  function is not changed here.
- **RFC-013's choice and RFC-011's links** are keyed by the artist row and go
  with it (`ON DELETE CASCADE`). A portrait chosen for a duo is not a portrait
  of either member, so it is not carried over. The parts get their own
  enrichment the first time anyone opens them, and this is the second gain of
  the feature: Deezer and TheAudioDB know `Ashreveal`.
- **Undoing a split** (decision 7) brings the joined artist back under the same
  derived id, because its name has not changed. Favourites already moved onto
  the parts stay there. They are not moved back, because the account may like
  the parts on their own merits by then.

## Decision 5 — a decision applies at once, and every scan applies it again

**At once.** Taking a decision rewrites the stored catalogue in one
transaction, under the writer gate. No file is read:

1. Every `track_participant` row that names the joined artist, in any role, is
   replaced in place by the parts. Each part takes that row's role, sub-role
   and position, and the later positions of the same track and role shift up.
   A part the track already credits in the same role and sub-role is not
   credited twice, and positions are renumbered so they stay contiguous. The
   relation's `UNIQUE (track_id, role, position)` requires this.
2. Each row the split touches records where it came from, in a new table.
   One participant row can come from several credits: `ARTISTS = ["A, B",
   "A, C"]` with both split gives one `A` that two decisions produced, and an
   `A` credited on its own as well as inside `A, B` is a third case. So the
   provenance is a relation, not a column:

   ```sql
   CREATE TABLE track_participant_source (
       track_id TEXT NOT NULL,
       artist_id TEXT NOT NULL,
       library_id TEXT NOT NULL,
       role TEXT NOT NULL,
       sub_role TEXT NOT NULL DEFAULT '',
       -- The decision's key, or the part's own key when the track also
       -- credits that part in its own right.
       credit_key TEXT NOT NULL,
       -- The credit as this track's tags spelled it, and its sort form, so an
       -- undo restores this track's spelling rather than another track's.
       credit TEXT NOT NULL,
       credit_sort TEXT,
       PRIMARY KEY (track_id, artist_id, role, sub_role, credit_key),
       FOREIGN KEY (track_id, artist_id, role, sub_role)
           REFERENCES track_participant(track_id, artist_id, role, sub_role)
           ON DELETE CASCADE
   ) STRICT;
   ```

   A participant row with no source row was never touched by a split, and
   every row written before this RFC is in that case. A row that a split
   touched and that the track also credits on its own carries a source row
   under its own key, so an undo can tell it apart from a part that exists
   only because of the split.
3. The joined credit's sort form, if any, is cut by decision 2's rule and
   paired with the fragments **before** they are deduplicated, position by
   position, or not at all when the counts differ, as `paired_sorts` does for
   a joined tag. The pairs are then deduplicated on the name's canonical form,
   keeping the first. `A, B, B, C` with the sort form `a, b, b, c` therefore
   gives three parts that keep their three sort forms. Pairing after the
   deduplication would compare three names with four sort forms and drop all
   of them. `paired_sorts` itself, which serves the first step, is unchanged.
4. `album.album_artist_id` is repointed to the first part wherever it named
   the joined artist.
5. Then the two end-of-scan derivations run for the library, as they do after
   a scan. `consolidate_catalog_derivations` rebuilds the album credits, the
   per-role counts, the derived MusicBrainz ids, the artist sweep and the
   artist search index. `consolidate_sort_names` re-votes the sort names. All
   of these are already derived from `track_participant`, so they follow
   without new code.
6. The search index's copy of each affected track's credited names is
   rewritten, as `set_track_metadata` already does after a correction.

**On every scan.** `apply_catalog_track_in_transaction` applies the second
step of decision 1 to the credits it derives, reading the library's decisions
**inside the transaction that writes the batch**. A decision taken while a
scan runs is therefore either before a batch, which then applies it, or after
it, in which case the rewrite above finds the rows that batch wrote. Nothing
falls between the two. A file that changes, and a full rescan, write the parts
again, with their source rows, which the desktop does not do.

A scan's skip path does not rewrite credits, so an unchanged file keeps what
the rewrite gave it. That is the same result, reached by doing nothing.

**The rule has one implementation.** The rewrite and the scan call the same
function, which turns a credit name, its sort form and the decisions into a
list of names. Both paths are tested against each other, so an immediate split
and a rescan of the same files end in identical rows.

## Decision 6 — clients learn of it as they learn of a scan

A decision's rewrite records on the library feed (RFC-007) what a scan of the
same change would record:

- a `track` `upsert` for every track whose credits moved, with its
  `full_hash`, exactly as a correction records one;
- an `album` `upsert` for every album whose credits moved;
- an `artist` `delete` for a joined artist the sweep removed, and an
  `artist` `upsert` for each part that was created or whose counts changed.

The vocabulary already admits all three kinds and both actions. No migration
touches `library_event`.

The Subsonic façade changes data and not shape. `artists[]` on a song, and
`albumArtists[]`, now list the parts. `artistId` names the first part. A
client that cached the joined artist's id is answered error 70 for it, exactly
as after a retag that renamed an artist. `getArtists` and `getIndexes` list
the parts and no longer list the joined name. Nothing is added to or removed
from any element, so the frozen v2.0-beta contract (RFC-002) is untouched. It
is a contract about shape, as RFC-013 decision 3 already relied on.

## Decision 7 — a way back

Removing a decision restores the state without it:

- **Removing a split** finds its rows by the decision's **`credit_key`**, never
  by comparing spellings. A file retagged from `A, B` to `a, b` after the
  decision still falls under it, because the key folds case, and the undo
  still finds that file's rows. On every track, in one transaction under the
  writer gate:
  1. the source rows under that key are deleted;
  2. a participant row the split touched that has no source row left existed
     only because of the split, and is deleted. A row whose only remaining
     source is its own key is credited in its own right: it stays, and that
     source row goes, since the row is no longer touched by any split. A row
     with another decision's key left, such as the `A` that `A, C` also
     produced, stays exactly as it is;
  3. one row naming the joined artist is upserted again under its derived id,
     where the first of its parts stood, with the spelling and the sort form
     this track's source rows recorded. Positions are renumbered as in
     decision 5.

  Then the same derivations, search-index rewrite and events as decision 5
  follow. The result is the catalogue a scan without the decision would have
  written, and the tests check exactly that.
- **Removing a keep** changes no row. The credit simply becomes a candidate
  again and returns to the list.

The desktop has no way back from "Don't split". Here, a mistaken click costs
one click to undo.

## Decision 8 — what "Needs attention" lists

The page is per library, for its owners and managers. A listener can fix
nothing on it, so a listener is not shown it, and its routes answer `404`.

### Credits to split

The candidates of decision 2 that have no decision, among the credits that
name an available track in the role `artist` or `albumartist`. Contributor
roles are left out, as the desktop leaves them out by having none. Composer
credits on this instance are heavily comma-joined, so including them is a
choice of its own (see "What is still open"). A decision, once taken, still
applies to the name in every role, because it is one artist row.

Each item carries the joined artist's id and name, the number of available
tracks crediting it, and its fragments. Each fragment carries its name and,
when an artist of that name already exists in the library, that artist's id
and album count. The items are sorted as the desktop sorts them: known
fragments first, then track count, then name. The offered actions are **Split**
and **Keep as one**.

A **Decided** view lists the library's decisions, newest first. Each one can be
removed (decision 7).

### Tracks to fix

The desktop's track categories that the server can both compute and act on.
They are computed over the **effective** values, which means the file's tag
unless `track_override` corrects it, so a track corrected in the editor leaves
its category at once. Only available tracks are counted.

| Category | Criterion |
| --- | --- |
| `missing_artist` | the track credits no `artist` |
| `missing_album` | `album_id IS NULL` |
| `missing_year` | effective year `NULL` or `0` |
| `missing_track_number` | effective track number `NULL` or `<= 0` |
| `missing_cover` | neither the track nor its album has artwork |
| `album_year_conflict` | its album's available tracks carry more than one non-zero effective year |
| `compilation_no_album_artist` | flagged a compilation, and the file names no album artist (it was filed under `Various Artists` by the fallback) |
| `duplicate_track_number` | another track of the same album has the same `COALESCE(disc, 1)` and the same number `> 0` |
| `track_number_gap` | on its disc, `MAX - MIN + 1 > COUNT(DISTINCT number)` |
| `probable_duplicate` | same normalised title and same credited artists, durations chained within 2 seconds. This reuses `waveflow_core::metadata::name_match::normalize_name` and `waveflow_core::inventory::chain_by_duration`, both public at the pinned revision |

Three of the desktop's categories are left out, for reasons particular to the
server:

- **`untaggable`** is about writing tags, and the server never writes any
  (RFC-002), so on the server every file would be in it.
- **`probable_fake_lossless`** needs the desktop's spectral analysis
  (`track_analysis`), which the server does not run.
- **`missing_title`** relies on a flag the server does not store: the scanner
  falls back to the file name without recording that it did. Adding the flag
  takes a column and a rescan to fill it, and it is left to a later change.

Like the desktop's, a track category offers **no action of its own**: it lists
tracks, and the track editor (`webapp/src/components/track-editor.tsx`) is where they are fixed. A
category with nothing in it is shown greyed out, as the desktop shows it.

### Counts

The page header gives the sum of the counts. The navigation shows no badge,
as on the desktop. A number that changes after every scan, in a corner of
every page, is a notification nobody asked for.

## The surface

All under `/api/v2`, tenancy-checked in the queries. `404` for a library the
caller is not an owner or manager of, and for an id not in it.

| Route | What |
| --- | --- |
| `GET /libraries/{id}/attention` | `[{ category, count }]` for `split_credits` and each track category, in display order. |
| `GET /libraries/{id}/attention/credits` | The candidates of decision 8, sorted as stated, with their fragments. |
| `GET /libraries/{id}/attention/tracks?category=&offset=&limit=` | The tracks in one category, as the song items `GET /libraries/{id}/tracks` already returns. `probable_duplicate` adds a group key to each item, so a client can draw the groups. `422` for an unknown category. |
| `GET /libraries/{id}/credit-decisions` | The decisions, newest first: id, credit, decision, parts, who and when. |
| `POST /libraries/{id}/credit-decisions` | `{ "artist_id": …, "decision": "split" \| "keep" }`. The artist must be a candidate. A split takes its fragments as decision 2 computes them now, and the server stores them. `201` with the decision. `409` when the credit already has one (remove it first). `422` for an artist that is not a candidate. |
| `DELETE /libraries/{id}/credit-decisions/{decision_id}` | Decision 7. `204`. |

The request names an **artist id**, not a string. A client can only decide on
something the server listed, and the folded key is computed on the server
from the row's name, so two clients cannot disagree about which credit they
meant.

Every write goes through `DomainServices` (CLAUDE.md, "One set of domain
services"), so the desktop can reach the same rules when it chooses to.

## What this RFC changes elsewhere

- **`src/tags.rs`**: nothing. The first step stays what it is.
- **`src/catalog.rs`**: the second step in `apply_catalog_track_in_transaction`.
  The album display and the track hint are pinned to the first step's names,
  and a test proves it: an album with no album-artist tag keeps its id across
  a split.
- **CLAUDE.md**, "Identifiers and time": one sentence saying that a credit
  decision re-identifies the artists it names and never an album or a track.
- **The Subsonic façade**: no code change. `docs/subsonic-compatibility.md`
  needs no new row, because no wire shape moves.
- **The desktop app** (`InstaZDLL/WaveFlow`): nothing in this repository. Its
  own list keeps writing its own profile. Reading a server library's decisions,
  or sending one, is the desktop's own change, made against the routes above.
- **Migrations**: one new dated file with `artist_credit_decision` and
  `track_participant_source`. Both start empty, which is the correct state for
  a catalogue nobody has decided anything about, so the upgrade schedules no
  rescan. None edits an applied migration.

## The order of the work

Each step is its own pull request, shippable without the next:

1. **This RFC.**
2. **Decisions, served**: the migration, the second step in the apply, the
   rewrite and its undo, the moves of favourites and ratings through the
   journal, the library events, `GET …/attention/credits` and the three
   decision routes. Tests:
   - a split and a rescan agree;
   - an album id survives a split;
   - a decision taken during a scan is not lost;
   - an undo restores the rows a scan without the decision writes;
   - `ARTISTS = ["A, B", "A, C"]` with both split, then one undone, keeps `A`
     and `C` and restores `A, B` alone;
   - `A, B, B, C` keeps a sort form on each of its three parts;
   - a file retagged from `A, B` to `a, b` after the split is still undone,
     with its own spelling.
3. **The page, for credits**: a "Needs attention" page in the web client's
   "More" menu, shown to members who own or manage at least one library, with
   the library scope the Folders and Upload pages already use. It lists the credits to
   split with their known fragments, offers Split (confirmed by a second click,
   as on the desktop) and Keep as one, and has a Decided view with undo. The
   artist page gains "Split into N artists" for an owner or manager when the
   artist is a candidate, which is the desktop's other entry point.
4. **Tracks to fix**: `GET …/attention`, `GET …/attention/tracks` and the
   track categories on the page, each opening its tracks in the song table
   where the editor already is.

Steps 2 and 3 fix what was observed. Step 4 brings the rest of the desktop's
page, and does not depend on them.

## What is still open

- **`" & "` and other connectors.** Not cut and not listed, as on the desktop
  (decision 2). Listing them too would put every `Simon & Garfunkel` on the
  page for a human to dismiss once. That may be the right trade on a library
  full of collaborations. If so, it is an addition to the candidate rule,
  because the decision table already stores parts and would not change.
- **Editing the parts.** A split takes the fragments as computed. Splitting
  `A, B & C` into three, or correcting a fragment's spelling, would mean
  accepting typed parts. The table can already store them, but the route does
  not take them until someone needs it.
- **Contributor credits.** Comma-joined composer and lyricist credits are
  common (`Daniel Durn, Katrine Neya Klith, 김우상, pdly & 쯔위`). A decision
  already applies to them when the same name is also a main credit. Listing
  them on their own is a separate choice about how long the page may get.
- **`missing_title`**, as stated in decision 8: it needs a flag the scanner
  does not write yet.
- **An instance-wide decision.** An operator with several libraries decides
  once per library. Whether one decision should reach every library is not
  decided here, as RFC-013 left it open for chosen images.

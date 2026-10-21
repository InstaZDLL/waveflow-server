import AxeBuilder from "@axe-core/playwright";
import { blake3 } from "@noble/hashes/blake3.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { expect, type Page, test } from "@playwright/test";

const session = {
  access_token: "e2e-access-token",
  user: { id: "user-1", username: "listener", role: "admin" },
  device_id: "device-1",
};

const albums = [
  {
    id: "album-1",
    library_id: "library-1",
    title: "Post",
    artist: "Björk",
    artist_id: "artist-1",
    artwork_hash: null,
    year: 1995,
    starred_at: null,
    user_rating: null,
  },
  {
    id: "album-2",
    library_id: "library-1",
    title: "Vespertine",
    artist: "Björk",
    artist_id: "artist-1",
    artwork_hash: null,
    year: 2001,
    starred_at: null,
    user_rating: null,
  },
];

const track = {
  id: "song-1",
  library_id: "library-1",
  album_id: "album-1",
  title: "Army of Me",
  album: "Post",
  artist: "Björk",
  artist_id: "artist-1",
  artwork_hash: null,
  duration_ms: 234_000,
  track: 1,
  disc: 1,
  starred_at: null,
  user_rating: null,
};

/**
 * 120 distinct plays, newest first. More than the screen resolves, which is the
 * point: each distinct track costs its own request.
 */
const history = Array.from({ length: 120 }, (_, index) => ({
  track_id: `t${index}`,
  submission: true,
  played_at: 1_000_000 - index,
}));

/**
 * The whole `Library` shape, not just what the picker reads. A fixture that
 * only carries the fields one screen happens to use is how the admin page went
 * down on an absent `folder_ids`: the next screen to touch it reads a field
 * nobody remembered to mock.
 */
const library = (id: string, name: string) => ({
  id,
  name,
  visibility: "private" as const,
  role: "owner" as "owner" | "manager" | "listener",
  last_scan_started_at: null,
  last_scan_completed_at: null,
  accepts_uploads: false as boolean,
  accepts_canvas: false as boolean,
});

/** Replaced by the scoping test; one library elsewhere, so no picker appears. */
let libraries: Array<ReturnType<typeof library>> = [library("library-1", "Ma musique")];

/** Flipped by the two tests that check a failure is shown as one. */
let membersFail = false;
let tokensFail = false;
/**
 * The clients this account has registered, as the self-scoped route answers
 * them. Mutable, because the test registers one and revokes one and the point
 * is that the list follows.
 */
type Client = {
  id: string;
  name: string;
  scopes: string[];
  expires_at: number | null;
  created_at: number;
  last_used_at: number | null;
  revoked_at: number | null;
};
let ownClients: Client[] = [];
/**
 * The Subsonic credential this account holds, or `null` for none.
 *
 * Two states and not one: the panel says different things about "you hold
 * none" and "the request has not answered", so the mock has to be able to say
 * the first.
 */
let ownSubsonicCredential: { created_at: number; updated_at: number } | null =
  null;
/**
 * The saved positions the playing screen lists under the track.
 *
 * Empty by default, which is what it was: `/api/v2/bookmarks` fell through to
 * the mock's 404, so the list the screen renders was never drawn by any test.
 * One that needs the page taller than the viewport fills it.
 */
let bookmarks: Array<{
  position_ms: number;
  comment: string | null;
  created_at: number;
  updated_at: number;
  song: {
    id: string;
    title: string;
    artist: string | null;
    artwork_hash?: string | null;
  };
}> = [];
/** Every registration the clients route received. */
let clientWrites: string[] = [];
/** Held open by the double-press test; instant for everyone else. */
let clientGate: Promise<void> = Promise.resolve();
/** Every method the credential route received, in order. */
let subsonicWrites: string[] = [];
/** Held open by the double-press test; instant for everyone else. */
let subsonicGate: Promise<void> = Promise.resolve();
function freshClients(): Client[] {
  return [
    {
      id: "client-1",
      name: "the kitchen speaker",
      scopes: [],
      expires_at: 1_800_000_000_000,
      created_at: 1_700_000_000_000,
      last_used_at: 1_700_600_000_000,
      revoked_at: null,
    },
    {
      // Already revoked, so the page has something it must leave out: this
      // list answers who can read the library, and a credential that cannot
      // is not an answer to it.
      id: "client-2",
      name: "a laptop that is gone",
      scopes: [],
      expires_at: null,
      created_at: 1_690_000_000_000,
      last_used_at: null,
      revoked_at: 1_695_000_000_000,
    },
    {
      // Not revoked, and past its expiry, which the server refuses. It became
      // a reachable shape when a self-minted credential started carrying an
      // expiry, and it has to be left out for the reason the revoked one is.
      id: "client-3",
      name: "a tablet whose year ran out",
      scopes: [],
      expires_at: 1_600_000_000_000,
      created_at: 1_560_000_000_000,
      last_used_at: 1_590_000_000_000,
      revoked_at: null,
    },
  ];
}
let scanFails = false;
/** The stream never answers at all, which is what a lost network looks like. */
let scanDrops = false;

/** Resolved by default; one test replaces it to stall `album-1`. */
let slowAlbum: Promise<void> = Promise.resolve();

const song = (
  index: number,
  title: string,
  rating: number,
  starred: boolean,
) => ({
  id: `song-${index}`,
  library_id: "library-1",
  album_id: "album-2",
  title,
  album: "Vespertine",
  artist: "Björk",
  artist_id: "artist-1",
  artwork_hash: null,
  duration_ms: 300_000,
  track: index,
  disc: 1,
  starred_at: starred ? 1 : null,
  user_rating: rating,
  // What the file is, as `SongItem` has always sent it. Present here because
  // the screen now draws it — and worth nothing as evidence of the wire,
  // which is asserted server-side against a real file by
  // `the_technical_fields_reach_the_wire_and_match_the_file`.
  codec: "FLAC",
  suffix: "flac",
  size: 27_983_872,
  bitrate: 957,
  sample_rate: 44_100,
  channels: 2,
  bit_depth: 16,
});

const albumDetail = {
  ...albums[1],
  songs: [
    song(1, "Hidden Place", 5, true),
    song(2, "Cocoon", 0, false),
    song(3, "Undo", 2, false),
  ],
};

/**
 * The artist page's own answer: a name and its albums. The most played list is
 * deliberately not in it — the page asks for that separately, because it
 * changes every time something is played while the discography does not.
 */
const artistDetail = {
  id: "artist-1",
  name: "Björk",
  album_count: 2,
  artwork_hash: null,
  albums,
};

/**
 * What this account played, in the order the server ranked it. Reassigned by
 * the test that empties it: an artist with no history must carry no heading
 * rather than a heading over nothing.
 */
let artistPlayed: Array<ReturnType<typeof song>> = [];
/** The limits the page asked for, so a screenful can be told from a catalogue. */
let topSongAsks: Array<string | null> = [];
/** How many times the artist page was asked for. */
let artistAsks = 0;
/**
 * The `enrichment` object of the artist's nth answer, counting from one.
 * Undefined leaves it out, as a server from before RFC-011 does.
 */
let enrichmentOf: (ask: number) => unknown = () => undefined;
/** Every artist answer after the first is a 500. */
let artistFailsAfterFirst = false;

/**
 * A dry run's answer: three of five entries placed, two it could not.
 *
 * Two entries short on purpose — the panel's whole reason to exist is showing
 * what an import would lose before it happens, so the report has to carry
 * losses for the test to mean anything.
 */
const importReport = {
  playlist: null,
  name: "From another player",
  encoding: "windows-1252",
  entries: 5,
  resolved: 3,
  unresolved: [
    {
      line: 7,
      entry: "common.flac",
      title: null,
      reason: "ambiguous",
    },
    {
      line: 9,
      entry: "http://radio.example/stream",
      title: "A stream",
      reason: "not-a-path",
    },
  ],
};
/** Every import request, so a dry run can be told from a commit. */
let importAsks: Array<{ dryRun: boolean; name: string | null }> = [];
/** What the export route was asked for, and what it answered. */
let exportAsks: string[] = [];

/**
 * The account's radio stations, which this harness really does keep: the page
 * writes and reads them back, so a mock answering a fixed list would pass a
 * delete that deleted nothing.
 */
/** Every write the radio page sent, so one click can be told from two. */
let radioWrites: string[] = [];
/** Held open by the double-press test; instant for everyone else. */
let radioGate: Promise<void> = Promise.resolve();

let stations: Array<{
  id: string;
  name: string;
  stream_url: string;
  homepage_url: string | null;
  created_at: number;
  updated_at: number;
}> = [];

const playlists = [
  {
    id: "playlist-1",
    name: "Evening",
    comment: null,
    public: false,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    songs: [song(1, "Hidden Place", 5, true)],
  },
];

const genres = [
  { name: "Art Pop", song_count: 412, album_count: 31 },
  { name: "Shoegaze", song_count: 233, album_count: 18 },
];

const genreSongs = [
  song(1, "Hidden Place", 5, true),
  song(2, "Cocoon", 0, false),
];

/**
 * A track another client has already corrected: a comment over two lines, and
 * an artist fixed. A save that does not touch the comment must leave it out of
 * the body — the server keeps whatever a patch does not mention, but only if
 * the client does not send it back.
 *
 * Built afresh for each test, because the PATCH mock applies what it receives.
 * The editor reads the corrections back after a save, and a mock that ignored
 * the write would let a form that never reloaded pass for one that did.
 */
function correctableTrack() {
  const overrides: Record<string, unknown> = {
    title: null,
    sort_title: null,
    year: null,
    track_number: null,
    disc_number: null,
    musicbrainz_recording_id: null,
    comment: "Remastered\nBonus edition",
    artists: ["Skunk Anansie"],
    genres: null,
  };
  return {
    song: {
      ...song(9, "Army Of Me", 0, false),
      year: 1995,
      artists: [{ id: "artist-2", name: "Skunk Anansie" }],
      genres: ["Trip Hop"],
    },
    tracked: {
      source: {
        title: "Army Of Me",
        sort_title: null,
        year: 1995,
        track_number: 9,
        disc_number: 1,
        musicbrainz_recording_id: null,
        comment: null,
      },
      overrides,
    },
  };
}

/** What the file credits: where a removed artists correction goes back to. */
const fileArtists = [{ id: "artist-1", name: "Björk" }];

let correctable = correctableTrack();

/** Every correction body sent, in order. */
let corrections: unknown[] = [];

type Offer = { full_hash: string; size_bytes: number; extension: string };

const freshUploadSession = () => ({
  session_id: "upload-1",
  next_chunk: 0,
  received_bytes: 0,
  chunk_bytes: 8,
  expires_at: 0,
});

/** What the upload mock received, in order. */
let uploadOffers: Offer[] = [];
let uploadChunks: Array<{ index: number; bytes: number[] }> = [];
let uploadCommits: string[] = [];
/** The one session the mock holds, advanced by each fragment it writes. */
let uploadSession = freshUploadSession();
/**
 * Set by the resume test: the fragment at this index is written, and its
 * acknowledgement lost — the session advances and the client is told 409.
 */
let loseAcknowledgementOf: number | null = null;

/**
 * External scrobbling — RFC-010.
 *
 * Two instances of one recipient, because that is the case the whole named
 * instances slice exists for and the one a screen keyed on the recipient alone
 * would collapse. Last.fm is declared and unavailable, which is its real shape
 * on a server whose operator registered no application.
 */
type Recipient = "listenbrainz" | "maloja" | "lastfm";

/**
 * What `lastfm_unavailability` answers — a case, not a sentence.
 *
 * It used to be the server's own English prose, copied here and asserted
 * verbatim below, so the test agreed with the server about a string neither of
 * them should have been sending to a client that ships in two languages.
 */
const NO_PUBLIC_URL = "browser_journey_needs_https";

const freshDestinations = () => [
  { provider: "listenbrainz" as const, destination: "default", available: true },
  { provider: "maloja" as const, destination: "alice", available: true },
  { provider: "maloja" as const, destination: "bob", available: true },
  {
    provider: "lastfm" as const,
    destination: "default",
    available: false,
    unavailable: NO_PUBLIC_URL,
  },
];

/**
 * A link is named by the **pair**, here as on the server.
 *
 * The recipient was fixed at `maloja` and the mock matched on the destination
 * alone, which works only while no two recipients share an instance name —
 * and `default` is the name they all use. Unlinking `maloja/default` would
 * have withdrawn `listenbrainz/default` beside it: the precise fault named in
 * the migration that rebuilt `scrobble_link_live_idx`, reproduced in the mock
 * that exists to catch it.
 */
const linkState = (provider: Recipient, destination: string) => ({
  provider,
  destination,
  health: "degraded" as const,
  pending: 12,
  retrying: 3,
  uncertain: 1,
  oldest_pending_at: 1_700_000_000_000,
  last_success_at: 1_700_000_000_000,
  last_failure: "rate_limited",
});

/** The one ambiguous listen, played when `t5` was, so history can name it. */
const freshUncertain = () => [
  {
    id: "entry-1",
    provider: "maloja" as const,
    destination: "alice",
    played_at: 1_000_000 - 5,
    attempts: 1,
    last_failure: "ambiguous",
    updated_at: 1_700_000_000_000,
  },
];

let destinations = freshDestinations();
let scrobbleLinks: Array<ReturnType<typeof linkState>> = [];
let uncertain = freshUncertain();
/** Every link, unlink and answer the mock received, in order. */
let scrobbleWrites: Array<{ method: string; path: string; body?: unknown }> = [];
/** Where the Last.fm journey says to send the browser. */
let lastFmAuthorizeUrl = "https://www.last.fm/api/auth/?api_key=k&cb=back";

/**
 * The words the mock holds for each track. Absent is a track without any.
 *
 * Shaped like `src/lyrics.rs` answers, `camelCase` envelope included — written
 * from the server rather than from `LyricsList` in `webapp/src/api/tracks.ts`, which
 * is how that type spent a release claiming `structured_lyrics` while every
 * read of it threw.
 */
let lyricSheets = new Map<
  string,
  Array<{
    displayArtist: string | null;
    displayTitle: string;
    lang: string;
    synced: boolean;
    line: Array<{ start?: number; value: string }>;
  }>
>();

/** Held open by the late-sheet test; instant for everyone else. */
let lyricsGate: Promise<void> = Promise.resolve();
/** The loop the mock holds for each track, as bytes. Absent is none. */
let canvases = new Map<string, number[]>();
/** Every placement and removal the mock received, in order. */
let canvasWrites: Array<{ method: string; track: string; bytes?: number[] }> =
  [];
/** Set by a test to refuse the next placement with this status. */
let refuseCanvasWith: number | null = null;
/**
 * Held by the playing page test, so a loop's bytes arrive only once it has
 * looked at the element: they are not a video, and the browser drops the
 * element the moment it finds that out.
 */
let canvasStreamGate: Promise<void> = Promise.resolve();

/**
 * Audit the page against WCAG A and AA, once it has stopped moving.
 *
 * The dark theme raises each block of a page into view, fading it in over
 * 640 ms. axe reads the colours as they are at the instant it looks, so an
 * audit that caught a block halfway measured its text at a fraction of its
 * opacity and reported a contrast the finished page does not have — on the
 * genre test, five runs in eight. Every finite animation is let finish first;
 * an infinite one, the pulse of a loading skeleton, would never settle, and is
 * left running.
 */
async function expectNoWcagViolations(page: Page) {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter(
          (animation) =>
            animation.effect?.getComputedTiming().endTime !== Infinity,
        )
        .map((animation) => animation.finished.catch(() => undefined)),
    ),
  );
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
}

async function mockAuthenticatedApi(page: Page) {
  await page.context().addCookies([
    {
      name: "waveflow-csrf",
      value: "e2e-csrf",
      url: "http://127.0.0.1:4173",
    },
  ]);
  await page.route("**/api/v2/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/v2/web/auth/refresh") {
      await route.fulfill({ json: session });
      return;
    }
    if (url.pathname === "/api/v2/albums/album-1") {
      // Held open by the concurrency test; instant for everyone else.
      await slowAlbum;
      await route.fulfill({ json: { ...albums[0], songs: [track] } });
      return;
    }
    if (url.pathname === "/api/v2/scans/scan-1/events" && scanDrops) {
      await route.abort("connectionfailed");
      return;
    }
    if (url.pathname === "/api/v2/scans/scan-1/events" && scanFails) {
      await route.fulfill({ status: 503, body: "" });
      return;
    }
    if (url.pathname === "/api/v2/scans/scan-1/events") {
      // Two frames, deliberately split so the client has to hold a partial
      // one between reads — which is what a real stream does.
      const job = (processed: number, status: string) =>
        JSON.stringify({
          id: "scan-1",
          library_id: "library-1",
          status,
          total_files: 10,
          processed_files: processed,
          added: processed,
          updated: 0,
          moved: 0,
          skipped: 0,
          unavailable: 0,
          errors: 0,
          current_path: `/music/track-${processed}.flac`,
          message: null,
        });
      await route.fulfill({
        status: 200,
        headers: { "content-type": "text/event-stream" },
        body:
          `event: snapshot\ndata: ${job(3, "running")}\n\n` +
          `: keep-alive\n\n` +
          `event: progress\ndata: ${job(10, "completed")}\n\n`,
      });
      return;
    }
    if (url.pathname.endsWith("/scans")) {
      await route.fulfill({ json: { scan_id: "scan-1" } });
      return;
    }
    if (url.pathname === "/api/v2/admin/users") {
      await route.fulfill({
        json: [
          {
            id: "user-1",
            username: "listener",
            role: "admin",
            disabled: false,
            has_subsonic_credential: false,
            folder_ids: ["library-1"],
          },
        ],
      });
      return;
    }
    if (url.pathname.endsWith("/members") && membersFail) {
      await route.fulfill({ status: 500, json: { error: "boom" } });
      return;
    }
    if (url.pathname.endsWith("/members")) {
      await route.fulfill({
        json: [
          {
            user_id: "user-1",
            username: "listener",
            role: "owner",
            created_at: 1,
          },
          {
            user_id: "user-2",
            username: "guest",
            role: "listener",
            created_at: 2,
          },
        ],
      });
      return;
    }
    if (url.pathname === "/api/v2/radio-stations") {
      if (route.request().method() === "POST") {
        radioWrites.push("POST");
        await radioGate;
        const draft = route.request().postDataJSON();
        const created = {
          id: `station-${stations.length + 1}`,
          name: String(draft.name).trim(),
          stream_url: draft.stream_url,
          homepage_url: draft.homepage_url ?? null,
          created_at: 1_700_000_000_000,
          updated_at: 1_700_000_000_000,
        };
        stations = [...stations, created].sort((a, b) =>
          a.name.localeCompare(b.name),
        );
        await route.fulfill({ status: 201, json: created });
        return;
      }
      await route.fulfill({ json: stations });
      return;
    }
    if (url.pathname.startsWith("/api/v2/radio-stations/")) {
      const id = url.pathname.slice("/api/v2/radio-stations/".length);
      if (route.request().method() === "DELETE") {
        stations = stations.filter((station) => station.id !== id);
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      // A replacement: every field arrives, so an absent homepage clears it.
      const draft = route.request().postDataJSON();
      const replaced = {
        id,
        name: String(draft.name).trim(),
        stream_url: draft.stream_url,
        homepage_url: draft.homepage_url ?? null,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_001,
      };
      stations = stations.map((station) =>
        station.id === id ? replaced : station,
      );
      await route.fulfill({ json: replaced });
      return;
    }
    if (url.pathname === "/api/v2/bookmarks") {
      await route.fulfill({ json: bookmarks });
      return;
    }
    if (url.pathname === "/api/v2/subsonic-credential") {
      const method = route.request().method();
      if (method !== "GET") {
        subsonicWrites.push(method);
        await subsonicGate;
      }
      if (method === "PUT") {
        // Rotation, never creation: an account holding none gets a 404, the
        // way the server answers it, so a panel that offered the control
        // anyway would fail here.
        if (ownSubsonicCredential === null) {
          await route.fulfill({ status: 404, json: { error: "not found" } });
          return;
        }
        ownSubsonicCredential = {
          ...ownSubsonicCredential,
          updated_at: 1_700_990_000_000,
        };
        await route.fulfill({ json: { api_key: "wfsk_shown_once_only" } });
        return;
      }
      if (method === "DELETE") {
        ownSubsonicCredential = null;
        await route.fulfill({ status: 204, body: "" });
        return;
      }
      await route.fulfill({ json: ownSubsonicCredential });
      return;
    }
    // Before the administrative `/tokens` branch below, which would swallow
    // it: this path is the account asking about itself.
    if (url.pathname === "/api/v2/api-tokens") {
      if (route.request().method() === "POST") {
        clientWrites.push("POST");
        await clientGate;
        const asked = route.request().postDataJSON() as { name: string };
        const created: Client = {
          id: `client-${ownClients.length + 1}`,
          name: asked.name,
          scopes: [],
          expires_at: 1_900_000_000_000,
          created_at: 1_700_900_000_000,
          last_used_at: null,
          revoked_at: null,
        };
        ownClients = [created, ...ownClients];
        await route.fulfill({
          status: 201,
          json: { ...created, secret: "wfapi_shown_once_and_never_again" },
        });
        return;
      }
      await route.fulfill({ json: ownClients });
      return;
    }
    if (url.pathname.startsWith("/api/v2/api-tokens/")) {
      const id = url.pathname.split("/").pop();
      ownClients = ownClients.map((client) =>
        client.id === id ? { ...client, revoked_at: 1_700_950_000_000 } : client,
      );
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname.endsWith("/tokens")) {
      if (tokensFail) {
        await route.fulfill({ status: 500, json: { error: "boom" } });
        return;
      }
      await route.fulfill({ json: [] });
      return;
    }
    if (url.pathname === "/api/v2/now-playing") {
      await route.fulfill({ json: [] });
      return;
    }
    if (url.pathname === "/api/v2/libraries") {
      await route.fulfill({ json: libraries });
      return;
    }
    if (url.pathname === "/api/v2/queue") {
      await route.fulfill({
        json: {
          current: "song-1",
          position_ms: 0,
          changed_by: null,
          updated_at: 1,
          songs: albumDetail.songs,
        },
      });
      return;
    }
    // Scrobbling — RFC-010. The order of these four matters: the authorise
    // path and the retry path each sit where a destination name and an entry
    // id would otherwise be read, which is why the server puts the literal
    // before the parameter, and why the mock has to match in the same order.
    if (url.pathname === "/api/v2/scrobble-destinations") {
      await route.fulfill({ json: destinations });
      return;
    }
    if (url.pathname === "/api/v2/scrobble-links") {
      await route.fulfill({ json: scrobbleLinks });
      return;
    }
    if (url.pathname.startsWith("/api/v2/scrobble-links/lastfm/authorize/")) {
      // The verb as it arrived, not the verb expected. Written in, the
      // recording agreed with the assertion whatever the client had sent, and
      // the three branches that record a write could not fail for the reason
      // they exist.
      scrobbleWrites.push({
        method: route.request().method(),
        path: url.pathname,
      });
      await route.fulfill({
        json: { authorize_url: lastFmAuthorizeUrl, expires_in: 720 },
      });
      return;
    }
    if (url.pathname.startsWith("/api/v2/scrobble-links/")) {
      const [, , , , provider, destination] = url.pathname.split("/");
      const method = route.request().method();
      scrobbleWrites.push({
        method,
        path: url.pathname,
        body: method === "PUT" ? route.request().postDataJSON() : undefined,
      });
      const recipient = provider as Recipient;
      // Both gestures address the pair. `PUT` replaces rather than adds, as
      // the route does: presenting a second token leaves one link, not two.
      const others = scrobbleLinks.filter(
        (link) =>
          link.provider !== recipient || link.destination !== destination,
      );
      scrobbleLinks =
        method === "PUT"
          ? [...others, linkState(recipient, destination)]
          : others;
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname.endsWith("/retry")) {
      const id = url.pathname.split("/")[5];
      scrobbleWrites.push({
        method: route.request().method(),
        path: url.pathname,
      });
      uncertain = uncertain.filter((entry) => entry.id !== id);
      await route.fulfill({ json: { id: "entry-2" } });
      return;
    }
    if (url.pathname.startsWith("/api/v2/scrobble-queue/uncertain/")) {
      const id = url.pathname.split("/")[5];
      scrobbleWrites.push({
        method: route.request().method(),
        path: url.pathname,
      });
      uncertain = uncertain.filter((entry) => entry.id !== id);
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname === "/api/v2/scrobble-queue/uncertain") {
      await route.fulfill({ json: uncertain });
      return;
    }
    if (url.pathname === "/api/v2/history") {
      await route.fulfill({ json: history });
      return;
    }
    if (url.pathname === "/api/v2/libraries/library-1/uploads") {
      const { offers } = route.request().postDataJSON() as { offers: Offer[] };
      uploadOffers.push(...offers);
      await route.fulfill({
        json: {
          verdicts: offers.map((offer) => ({
            full_hash: offer.full_hash,
            decision: "accepted",
            session: uploadSession,
          })),
        },
      });
      return;
    }
    if (url.pathname === "/api/v2/uploads/upload-1") {
      await route.fulfill({ json: uploadSession });
      return;
    }
    if (url.pathname.startsWith("/api/v2/uploads/upload-1/chunks/")) {
      const index = Number(url.pathname.split("/").pop());
      if (index !== uploadSession.next_chunk) {
        await route.fulfill({ status: 409, json: { code: "conflict" } });
        return;
      }
      const body = route.request().postDataBuffer() ?? Buffer.alloc(0);
      uploadChunks.push({ index, bytes: [...body] });
      uploadSession = {
        ...uploadSession,
        next_chunk: index + 1,
        received_bytes: uploadSession.received_bytes + body.length,
      };
      if (index === loseAcknowledgementOf) {
        loseAcknowledgementOf = null;
        await route.fulfill({ status: 409, json: { code: "conflict" } });
        return;
      }
      await route.fulfill({ json: uploadSession });
      return;
    }
    if (url.pathname === "/api/v2/uploads/upload-1/commit") {
      uploadCommits.push("upload-1");
      await route.fulfill({
        status: 201,
        json: { track_id: "song-uploaded", full_hash: uploadOffers[0]?.full_hash },
      });
      return;
    }
    if (
      url.pathname.startsWith("/api/v2/tracks/") &&
      url.pathname.endsWith("/lyrics")
    ) {
      // The playing page asks for these, and the catch-all below would answer
      // with a track, which is not a list of lyrics — no test visited that page
      // before the canvas put a loop on it. Empty unless a test says otherwise.
      const trackId = url.pathname.split("/")[4] as string;
      await lyricsGate;
      await route.fulfill({
        json: {
          trackId,
          structuredLyrics: lyricSheets.get(trackId) ?? [],
        },
      });
      return;
    }
    const parts = url.pathname.split("/");
    if (
      parts.length === 6 &&
      parts[3] === "tracks" &&
      (parts[5] === "canvas" || parts[5] === "canvas-ticket")
    ) {
      const trackId = parts[4] as string;
      const method = route.request().method();
      if (parts[5] === "canvas-ticket") {
        // What the server does: minting checks the link, so a track without
        // a canvas answers 404 to the ticket itself.
        if (canvases.has(trackId)) {
          await route.fulfill({
            json: {
              url: `/api/v2/canvas-stream/ticket-${trackId}`,
              expires_at: Date.now() + 3_600_000,
            },
          });
        } else {
          await route.fulfill({ status: 404, json: { error: "not found" } });
        }
        return;
      }
      if (method === "PUT") {
        const body = route.request().postDataBuffer() ?? Buffer.alloc(0);
        const bytes = [...body];
        canvasWrites.push({ method, track: trackId, bytes });
        if (refuseCanvasWith !== null) {
          const status = refuseCanvasWith;
          refuseCanvasWith = null;
          await route.fulfill({ status, json: { error: "refused" } });
          return;
        }
        canvases.set(trackId, bytes);
        await route.fulfill({
          json: {
            url: "/api/v2/canvas/loop",
            hash: "loop",
            format: "webm",
            byte_size: bytes.length,
          },
        });
        return;
      }
      if (method === "DELETE") {
        canvasWrites.push({ method, track: trackId });
        if (!canvases.delete(trackId)) {
          await route.fulfill({ status: 404, json: { error: "not found" } });
          return;
        }
        await route.fulfill({ status: 204, body: "" });
        return;
      }
    }
    if (url.pathname.startsWith("/api/v2/canvas-stream/")) {
      await canvasStreamGate;
      // The element that asked may be gone by the time the gate opens, and a
      // request it abandoned can no longer be answered.
      await route
        .fulfill({
          status: 200,
          headers: { "content-type": "video/webm" },
          body: "not a video",
        })
        .catch(() => undefined);
      return;
    }
    if (url.pathname === "/api/v2/tracks/song-9/overrides") {
      await route.fulfill({ json: correctable.tracked });
      return;
    }
    if (url.pathname === "/api/v2/tracks/song-9") {
      if (route.request().method() === "PATCH") {
        const body = route.request().postDataJSON() as Record<string, unknown>;
        corrections.push(body);
        // What the server does with it: a key present replaces its correction,
        // null removes it, and a list that is removed goes back to the file.
        Object.assign(correctable.tracked.overrides, body);
        if (typeof body.title === "string") correctable.song.title = body.title;
        if (body.artists === null) correctable.song.artists = fileArtists;
      }
      await route.fulfill({ json: correctable.song });
      return;
    }
    if (url.pathname.startsWith("/api/v2/tracks/")) {
      const id = url.pathname.split("/")[4];
      await route.fulfill({ json: { ...song(1, `Track ${id}`, 0, false), id } });
      return;
    }
    if (url.pathname === "/api/v2/genres") {
      await route.fulfill({ json: genres });
      return;
    }
    if (url.pathname === "/api/v2/songs/by-genre") {
      await route.fulfill({
        json: url.searchParams.get("genre") ? genreSongs : [],
      });
      return;
    }
    if (url.pathname === "/api/v2/albums/album-2") {
      await route.fulfill({ json: albumDetail });
      return;
    }
    if (url.pathname.startsWith("/api/v2/ratings/")) {
      await route.fulfill({ status: 204, body: "" });
      return;
    }
    if (url.pathname === "/api/v2/playlists") {
      await route.fulfill({ json: playlists });
      return;
    }
    if (url.pathname === "/api/v2/playlists/import") {
      importAsks.push({
        dryRun: url.searchParams.get("dry_run") === "true",
        name: url.searchParams.get("name"),
      });
      const dryRun = url.searchParams.get("dry_run") === "true";
      await route.fulfill({
        status: dryRun ? 200 : 201,
        json: {
          ...importReport,
          name: url.searchParams.get("name") ?? importReport.name,
          playlist: dryRun
            ? null
            : { ...playlists[0], name: url.searchParams.get("name") },
        },
      });
      return;
    }
    if (url.pathname.endsWith("/export")) {
      exportAsks.push(url.pathname);
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "audio/x-mpegurl; charset=utf-8",
          "content-disposition": 'attachment; filename="Evening.m3u8"',
        },
        body: "#EXTM3U\n#PLAYLIST:Evening\n#EXTINF:300,Björk - Hidden Place\nBjörk/Vespertine/01.flac\n",
      });
      return;
    }
    if (url.pathname === "/api/v2/artists") {
      await route.fulfill({
        json: [
          {
            id: artistDetail.id,
            name: artistDetail.name,
            album_count: artistDetail.album_count,
            artwork_hash: null,
          },
        ],
      });
      return;
    }
    if (url.pathname === "/api/v2/artists/artist-1/top-songs") {
      topSongAsks.push(url.searchParams.get("limit"));
      await route.fulfill({ json: artistPlayed });
      return;
    }
    if (url.pathname === "/api/v2/artists/artist-1/albums") {
      await route.fulfill({ json: albums });
      return;
    }
    if (url.pathname === "/api/v2/artists/artist-1") {
      artistAsks += 1;
      if (artistFailsAfterFirst && artistAsks > 1) {
        await route.fulfill({ status: 500, json: { error: "internal" } });
        return;
      }
      await route.fulfill({
        json: { ...artistDetail, enrichment: enrichmentOf(artistAsks) },
      });
      return;
    }
    if (url.pathname === "/api/v2/albums") {
      // Answering the order lets a test tell a refetch from a local re-sort.
      const sorted =
        url.searchParams.get("sort") === "newest"
          ? [...albums].reverse()
          : albums;
      await route.fulfill({ json: sorted });
      return;
    }
    await route.fulfill({ status: 404, json: { error: "not found" } });
  });
}

test.beforeEach(async ({ page }) => {
  libraries = [library("library-1", "Ma musique")];
  membersFail = false;
  tokensFail = false;
  ownClients = freshClients();
  bookmarks = [];
  importAsks = [];
  exportAsks = [];
  stations = [];
  radioWrites = [];
  radioGate = Promise.resolve();
  artistPlayed = [song(2, "Cocoon", 0, false), song(1, "Hidden Place", 5, true)];
  topSongAsks = [];
  artistAsks = 0;
  enrichmentOf = () => undefined;
  artistFailsAfterFirst = false;
  // **Reset here, and they were not.** These four reached the mock without
  // reaching this block, because a `replace` whose needle had already been
  // reformatted did nothing and said nothing. So `subsonicWrites`
  // accumulated across every test a worker ran, and the rotation assertion
  // passed only because its test and the one that writes before it happened to
  // land in different workers. A gate is worse: a test that fails before
  // releasing one leaves it pending, and every later test in that worker waits
  // on the route for ever.
  subsonicWrites = [];
  subsonicGate = Promise.resolve();
  clientWrites = [];
  clientGate = Promise.resolve();
  // An account the operator has granted one, because that is the state the
  // panel is about: it rotates and switches off, and never creates.
  ownSubsonicCredential = { created_at: 1_700_000_000_000, updated_at: 1_700_000_000_000 };
  scanFails = false;
  scanDrops = false;
  corrections = [];
  correctable = correctableTrack();
  uploadOffers = [];
  uploadChunks = [];
  uploadCommits = [];
  uploadSession = freshUploadSession();
  loseAcknowledgementOf = null;
  lyricSheets = new Map();
  lyricsGate = Promise.resolve();
  canvases = new Map();
  canvasWrites = [];
  refuseCanvasWith = null;
  canvasStreamGate = Promise.resolve();
  slowAlbum = Promise.resolve();
  destinations = freshDestinations();
  scrobbleLinks = [];
  uncertain = freshUncertain();
  scrobbleWrites = [];
  lastFmAuthorizeUrl = "https://www.last.fm/api/auth/?api_key=k&cb=back";
  await mockAuthenticatedApi(page);
});

test("renders the studio shell and persists appearance preferences", async ({
  page,
}, testInfo) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  await expect(page.getByText("Vespertine", { exact: true })).toBeVisible();

  // **The pickers live on the Settings page now**, not in the sidebar footer
  // and not in the mobile header. They are settings and not destinations, and
  // they sat under thirteen navigation entries; there is no copy left behind,
  // because two ways to ask one question is the thing that move was about.
  await page.goto("/settings");
  await expect(
    page.getByRole("heading", { name: "Settings" }),
  ).toBeVisible();
  const preferences = page.locator(".settings-panel").first();
  const theme = preferences.getByLabel("Theme");
  await theme.selectOption({ index: 1 });
  const selectedTheme = await theme.inputValue();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("waveflow.theme")))
    .toBe(selectedTheme);

  const language = preferences.getByLabel("Language");
  await language.selectOption("fr");
  await expect(page.getByRole("link", { name: "Recherche" }).first()).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");
  // And nowhere else: a picker still reachable from the corridor would mean
  // the move had added a place rather than chosen one.
  await expect(page.locator(".sidebar").getByLabel("Thème")).toHaveCount(0);
  await expect(page.locator(".mobile-header").getByLabel("Thème")).toHaveCount(
    0,
  );

  if (testInfo.project.name === "mobile") {
    await expect(page.locator(".mobile-navigation")).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);
  } else {
    await expect(page.locator(".sidebar")).toBeVisible();
  }
});

/**
 * The heard part of the line ends under the thumb, wherever the track is.
 *
 * The wave is placed in the progress grid's middle column. Named by its start
 * alone, an absolutely placed item runs to the container's edge, so the wave
 * was measured against the gap and the duration's column as well and outran
 * the thumb further the later the track. Driven through `--played`, the value
 * the player writes, so every position is reached without playing anything.
 */
test("ends the played wave under the progress thumb", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "the wave is drawn beside the rail");
  await page.goto("/");
  const progress = page.locator(".player-progress");
  await expect(progress).toBeVisible();
  const measured = await progress.evaluate((node: HTMLElement) => {
    const wave = node.querySelector(".progress-wave") as HTMLElement;
    const input = node.querySelector("input") as HTMLInputElement;
    // The thumb's size as the stylesheet declares it, in rem — and the size it
    // has always had where no token names it, so a stylesheet without one is
    // judged on where its wave ends rather than on a missing variable.
    const rem =
      Number.parseFloat(
        getComputedStyle(node).getPropertyValue("--fe-thumb"),
      ) || 0.625;
    const thumb =
      rem *
      Number.parseFloat(getComputedStyle(document.documentElement).fontSize);
    // The range's own bounds. A mocked track may carry no duration, which
    // leaves `max` at zero and every value clamped to it, so the range is
    // given a span to move along when it has none.
    const min = Number(input.min) || 0;
    if (!(Number(input.max) > min)) input.max = String(min + 100);
    const max = Number(input.max);
    return [0, 0.25, 0.5, 0.81, 1].map((played) => {
      // The range moved, and the share the player would write beside it.
      input.value = String(min + played * (max - min));
      node.style.setProperty("--played", String(played));
      // Where the native thumb sits, read back from the range's value rather
      // than from `--played`: the wave has to follow the thumb, not itself.
      const fraction = (Number(input.value) - min) / (max - min);
      const w = wave.getBoundingClientRect();
      const i = input.getBoundingClientRect();
      return {
        played,
        waveEnd: w.right,
        thumbCentre: i.left + thumb / 2 + fraction * (i.width - thumb),
        trackEnd: i.right,
      };
    });
  });
  for (const { played, waveEnd, thumbCentre, trackEnd } of measured) {
    expect(Math.abs(waveEnd - thumbCentre), `at ${played}`).toBeLessThanOrEqual(1);
    expect(waveEnd, `at ${played}`).toBeLessThanOrEqual(trackEnd);
  }
});

test("keeps navigation and player controls within their own space", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".player")).toBeVisible();

  if (testInfo.project.name === "mobile") {
    await expect(page.locator(".sidebar")).toBeHidden();
    const bounds = await page.evaluate(() => {
      const track = document.querySelector(".player-track")?.getBoundingClientRect();
      const progress = document.querySelector(".player-progress")?.getBoundingClientRect();
      const player = document.querySelector(".player")?.getBoundingClientRect();
      const navigation = document.querySelector(".mobile-navigation")?.getBoundingClientRect();
      return {
        trackBottom: track?.bottom ?? 0,
        progressTop: progress?.top ?? 0,
        playerBottom: player?.bottom ?? 0,
        navigationTop: navigation?.top ?? 0,
        overflow: document.documentElement.scrollWidth - window.innerWidth,
      };
    });
    expect(bounds.progressTop).toBeGreaterThanOrEqual(bounds.trackBottom);
    expect(bounds.navigationTop).toBeGreaterThanOrEqual(bounds.playerBottom);
    expect(bounds.overflow).toBeLessThanOrEqual(1);
    await page.setViewportSize({ width: 320, height: 700 });
    const narrow = await page.evaluate(() => ({
      trackBottom: document.querySelector(".player-track")?.getBoundingClientRect().bottom ?? 0,
      progressTop: document.querySelector(".player-progress")?.getBoundingClientRect().top ?? 0,
      playerBottom: document.querySelector(".player")?.getBoundingClientRect().bottom ?? 0,
      navigationTop: document.querySelector(".mobile-navigation")?.getBoundingClientRect().top ?? 0,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    }));
    expect(narrow.progressTop).toBeGreaterThanOrEqual(narrow.trackBottom);
    expect(narrow.navigationTop).toBeGreaterThanOrEqual(narrow.playerBottom);
    expect(narrow.overflow).toBeLessThanOrEqual(1);
  } else {
    const bounds = await page.evaluate(() => {
      const sidebar = document.querySelector(".sidebar")?.getBoundingClientRect();
      const footer = document.querySelector(".sidebar-footer")?.getBoundingClientRect();
      const navigation = document.querySelector(".primary-navigation")?.getBoundingClientRect();
      return {
        sidebarBottom: sidebar?.bottom ?? 0,
        footerBottom: footer?.bottom ?? 0,
        navigationBottom: navigation?.bottom ?? 0,
      };
    });
    expect(bounds.footerBottom).toBeLessThanOrEqual(bounds.sidebarBottom);
    expect(bounds.navigationBottom).toBeLessThanOrEqual(bounds.footerBottom);
    await expect(page.locator(".sidebar-footer").getByRole("button", { name: "Sign out" })).toBeVisible();
    await page.setViewportSize({ width: 821, height: 650 });
    const compact = await page.evaluate(() => ({
      sidebarBottom: document.querySelector(".sidebar")?.getBoundingClientRect().bottom ?? 0,
      footerBottom: document.querySelector(".sidebar-footer")?.getBoundingClientRect().bottom ?? 0,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    }));
    expect(compact.footerBottom).toBeLessThanOrEqual(compact.sidebarBottom);
    expect(compact.overflow).toBeLessThanOrEqual(1);
  }
});

test("shows one current navigation item and separates administration panels", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "sidebar layout is desktop-only");
  await page.goto("/settings/scrobbling");
  const current = page.locator(".primary-navigation a[data-status='active']");
  await expect(current).toHaveCount(1);
  await expect(current).toHaveText("Scrobbling");

  const settings = page.locator(".primary-navigation").getByRole("link", {
    name: "Settings",
  });
  await settings.hover();
  const activeBackground = await current.evaluate((node) =>
    getComputedStyle(node).backgroundColor,
  );
  const hoverBackground = await settings.evaluate((node) =>
    getComputedStyle(node).backgroundColor,
  );
  expect(hoverBackground).not.toBe(activeBackground);

  await page.goto("/admin");
  await expect(page.locator(".admin-page > .stack .user-row")).toBeVisible();
  await expect(page.locator(".now-playing-panel")).toBeVisible();
  const gap = await page.evaluate(() => {
    const accounts = document.querySelector(".admin-page > .stack");
    const playing = document.querySelector(".now-playing-panel");
    return (
      (playing?.getBoundingClientRect().top ?? 0) -
      (accounts?.getBoundingClientRect().bottom ?? 0)
    );
  });
  expect(gap).toBeGreaterThanOrEqual(24);
});

test("puts the songs playback action beside the sort and names its scope", async ({
  page,
}) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({
      json: [track, { ...track, id: "song-2", title: "Second track" }],
    }),
  );
  await page.goto("/songs");
  const play = page.locator(".page-header").getByRole("button", {
    name: "Play all 2 tracks",
  });
  await expect(play).toBeVisible();
  await play.click();
  await expect(page.locator(".player-track strong")).toHaveText("Army of Me");
});

/**
 * The right click on a track, and the properties it opens (RFC-012 decision 4).
 *
 * Driven from the keyboard first, because a menu reachable only by a right
 * click is one the accessibility gate is right to fail: Shift+F10 on the
 * focused row, arrows to Properties, Enter. The panel shows where the file
 * sits in its library — the one field only `GET /tracks/{id}` sends — and a
 * corrected tag beside what the file said. Escape gives focus back to the row
 * the menu was opened from, through the menu that no longer exists.
 */
test("opens a track's properties from its context menu", async ({ page }) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [correctable.song] }),
  );
  await page.route("**/api/v2/tracks/song-9", (route) =>
    route.fulfill({
      json: {
        ...correctable.song,
        relative_path: "Björk/Post/09 Army Of Me.flac",
        full_hash: "a".repeat(64),
        created_at: Date.UTC(2026, 8, 15, 12, 0),
        album_artist: "Björk",
        comment: "Remastered\nBonus edition",
        musicbrainz_id: null,
        isrc: ["GBAAA9500009"],
        bpm: 92,
        replay_gain_track_gain: -6.5,
        replay_gain_track_peak: 0.98,
        replay_gain_album_gain: null,
        replay_gain_album_peak: null,
      },
    }),
  );
  await page.goto("/songs");
  const row = page.locator(".songs tbody tr").first();
  const title = row.locator("button.song-title");
  await title.focus();

  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Actions for Army Of Me" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem")).toHaveText([
    "Play",
    "Add to queue",
    "Download",
    "Properties",
    "Edit tags",
    "Move to folder…",
  ]);
  await expect(menu.getByRole("menuitem", { name: "Play" })).toBeFocused();
  // Where focus is shows on its own, not only as a faint tint.
  await expect(menu.getByRole("menuitem", { name: "Play" })).toHaveCSS(
    "outline-style",
    "solid",
  );
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitem", { name: "Properties" })).toBeFocused();
  await page.keyboard.press("Enter");

  const panel = page.getByRole("dialog", { name: "Army Of Me" });
  await expect(panel).toBeVisible();
  await expect(menu).toHaveCount(0);
  await expect(panel).toContainText("Björk/Post/09 Army Of Me.flac");
  await expect(panel).toContainText("a".repeat(64));
  await expect(panel).toContainText("957 kbps");
  await expect(panel).toContainText("44.1 kHz");
  await expect(panel).toContainText("-6.5 dB, peak 0.98");
  // The comment is a correction; the file carried none.
  await expect(panel).toContainText("Corrected — the file says —");
  await expectNoWcagViolations(page);

  // A click in the panel's own padding lands on the dialog element, as a
  // click on the backdrop does; only the second may close it.
  await panel.click({ position: { x: 6, y: 6 } });
  await expect(panel).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(title).toBeFocused();

  // The backdrop, outside the panel's box, does close it.
  await page.keyboard.press("Shift+F10");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(panel).toBeVisible();
  await page.mouse.click(2, 2);
  await expect(panel).toHaveCount(0);

  // The pointer reaches the same menu, and a click away closes it.
  await row.click({ button: "right" });
  await expect(menu).toBeVisible();
  await page.mouse.click(5, 5);
  await expect(menu).toHaveCount(0);

  // The wheel closes it even turned over the menu, which does not scroll:
  // the page beneath would move and leave the menu off its row. Dispatched
  // on the entry itself, because a real wheel in a headless browser brings
  // side effects of its own that close the menu either way.
  await row.click({ button: "right" });
  await expect(menu).toBeVisible();
  await menu
    .getByRole("menuitem", { name: "Play" })
    .dispatchEvent("wheel", { deltaY: 120, bubbles: true });
  await expect(menu).toHaveCount(0);
});

/**
 * The lyrics in a track's properties (RFC-012 decision 6).
 *
 * The sheets come in the server's order, the first marked as the one shown,
 * each naming its source. An owner uploads a sheet as the bytes it is and
 * takes an upload back; anyone downloads the shown sheet, saved under the
 * name the server gives in `Content-Disposition`.
 */
test("uploads, removes and downloads lyrics from a track's properties", async ({
  page,
}) => {
  const sheet = (source: string, synced: boolean, value: string) => ({
    displayArtist: "Björk",
    displayTitle: "Army Of Me",
    lang: "eng",
    synced,
    line: [{ start: synced ? 1000 : undefined, value }],
    source,
  });
  let sheets = [sheet("sidecar_lrc", true, "from the sidecar")];
  const sent: Array<{ method: string; path: string; body: string }> = [];
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [correctable.song] }),
  );
  await page.route("**/api/v2/tracks/song-9/lyrics", (route) =>
    route.fulfill({
      json: { trackId: "song-9", structuredLyrics: sheets },
    }),
  );
  await page.route("**/api/v2/tracks/song-9/lyrics/ttml", (route) => {
    const request = route.request();
    sent.push({
      method: request.method(),
      path: "ttml",
      body: request.postData() ?? "",
    });
    if (request.method() === "PUT") {
      sheets = [sheet("upload_ttml", true, "bonjour"), ...sheets];
      return route.fulfill({
        json: { trackId: "song-9", structuredLyrics: sheets },
      });
    }
    sheets = sheets.filter((entry) => entry.source !== "upload_ttml");
    return route.fulfill({ status: 204, body: "" });
  });
  await page.route("**/api/v2/tracks/song-9/lyrics/file", (route) =>
    route.fulfill({
      body: "[00:01.00]from the sidecar\n",
      headers: {
        "content-type": "application/octet-stream",
        "content-disposition":
          "attachment; filename=\"Bj_rk - Army Of Me.lrc\"; filename*=UTF-8''Bj%C3%B6rk%20-%20Army%20Of%20Me.lrc",
      },
    }),
  );
  await page.goto("/songs");
  await page.evaluate(() => {
    const seen = window as unknown as { __saved: string[] };
    seen.__saved = [];
    HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
      seen.__saved.push(this.download);
    };
  });
  await page
    .locator(".songs tbody tr")
    .first()
    .locator("button.song-title")
    .focus();
  await page.keyboard.press("Shift+F10");
  await page
    .getByRole("menu", { name: "Actions for Army Of Me" })
    .getByRole("menuitem", { name: "Properties" })
    .click();
  const panel = page.getByRole("dialog", { name: "Army Of Me" });
  const lyrics = panel.locator(".lyrics-section");
  await expect(lyrics).toContainText("Shown");
  await expect(lyrics).toContainText("LRC file beside the track");

  // An upload goes as the file's own bytes, and leads once it is in.
  await panel.locator('input[type="file"]').setInputFiles({
    name: "Army Of Me.ttml",
    mimeType: "application/ttml+xml",
    buffer: Buffer.from('<tt xmlns="http://www.w3.org/ns/ttml"/>'),
  });
  await expect(lyrics).toContainText("Uploaded TTML");
  expect(sent).toEqual([
    {
      method: "PUT",
      path: "ttml",
      body: '<tt xmlns="http://www.w3.org/ns/ttml"/>',
    },
  ]);
  await expect(lyrics.locator(".property-row").first()).toContainText(
    "Uploaded TTML",
  );

  // A file that is not a sheet is refused before anything is sent.
  await panel.locator('input[type="file"]').setInputFiles({
    name: "notes.srt",
    mimeType: "text/plain",
    buffer: Buffer.from("1"),
  });
  await expect(lyrics.getByRole("alert")).toHaveText(
    "Choose a .lrc, .txt or .ttml file.",
  );
  expect(sent).toHaveLength(1);

  // Taking the upload back leaves the scanned sheet shown again.
  await lyrics.getByRole("button", { name: "Remove the TTML" }).click();
  await expect(lyrics).not.toContainText("Uploaded TTML");
  expect(sent.at(-1)?.method).toBe("DELETE");

  // The download is saved under the server's name, accents and all.
  await lyrics.getByRole("button", { name: "Download lyrics" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { __saved: string[] }).__saved),
    )
    .toEqual(["Björk - Army Of Me.lrc"]);
  await expectNoWcagViolations(page);
});

/**
 * Download is the stream ticket with `?download=1` (RFC-012 decision 5).
 *
 * The browser cannot send the session on a navigation, so the menu mints a
 * ticket and saves from it through a link with `download` and no value,
 * which leaves the file's name to the server's `Content-Disposition`. That
 * header is asserted server-side, against a real file; what is asserted here
 * is the link the page builds. Chromium hands a `download` link straight to
 * its download manager, past every route a test could install, so the link
 * is caught as it is clicked rather than on the wire.
 */
test("downloads a track from its context menu", async ({ page }) => {
  await page.route("**/api/v2/songs?*", (route) =>
    route.fulfill({ json: [correctable.song] }),
  );
  await page.route("**/api/v2/tracks/song-9/stream-ticket", (route) =>
    route.fulfill({
      json: { url: "/api/v2/stream/ticket-9", expires_at: Date.now() + 60_000 },
    }),
  );
  await page.goto("/songs");
  await page.evaluate(() => {
    const seen = window as unknown as {
      __saved: Array<{ href: string | null; download: string }>;
    };
    seen.__saved = [];
    HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
      seen.__saved.push({
        href: this.getAttribute("href"),
        download: this.getAttribute("download") ?? "absent",
      });
    };
  });
  await page
    .locator(".songs tbody tr")
    .first()
    .locator("button.song-title")
    .focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Actions for Army Of Me" });
  await menu.getByRole("menuitem", { name: "Download" }).click();
  await expect(menu).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __saved: unknown[] }).__saved,
      ),
    )
    .toEqual([{ href: "/api/v2/stream/ticket-9?download=1", download: "" }]);
  // Nothing is left in the document once the link has been followed.
  await expect(page.locator("a[download]")).toHaveCount(0);

  // A ticket that cannot be minted says so, rather than a click doing
  // nothing; and saves nothing.
  await page.route("**/api/v2/tracks/song-9/stream-ticket", (route) =>
    route.fulfill({ status: 503, body: "" }),
  );
  await page.keyboard.press("Shift+F10");
  await menu.getByRole("menuitem", { name: "Download" }).click();
  const failed = page
    .getByRole("alert")
    .filter({ hasText: "The download could not start" });
  await expect(failed).toHaveText("The download could not start. Try again.");
  await failed.getByRole("button", { name: "Dismiss" }).click();
  await expect(failed).toHaveCount(0);
  expect(
    await page.evaluate(
      () => (window as unknown as { __saved: unknown[] }).__saved.length,
    ),
  ).toBe(1);
});

test("gives random playback one clear primary action", async ({ page }) => {
  await page.route("**/api/v2/songs/random?*", (route) =>
    route.fulfill({ json: [track] }),
  );
  await page.goto("/random");
  const controls = page.locator(".page-header");
  const play = controls.getByRole("button", { name: "Play", exact: true });
  const queue = controls.getByRole("button", { name: "Add queue" });
  const redraw = controls.getByRole("button", { name: "Draw again" });
  await expect(play).toBeVisible();
  await expect(queue).toBeVisible();
  await expect(redraw).toBeVisible();
  const playFill = await play.evaluate((node) => getComputedStyle(node).backgroundColor);
  const queueFill = await queue.evaluate((node) => getComputedStyle(node).backgroundColor);
  expect(playFill).not.toBe(queueFill);
});

test("offers a keyboard skip link and a localized not-found route", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to content" });
  await expect(skip).toBeFocused();
  await skip.press("Enter");
  await expect(page.locator("#main-content")).toBeFocused();

  await page.goto("/a-room-that-does-not-exist");
  await expect(
    page.getByRole("heading", { name: "This room is silent" }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: "Back to the library" })).toBeVisible();
});

test("has no automated WCAG A or AA violations", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expectNoWcagViolations(page);
});

test("keeps the light appearance readable", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("waveflow.theme", "default");
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "default");
  await expectNoWcagViolations(page);
});

/**
 * The browse controls are the whole of lot A that has behaviour rather than
 * appearance. Sorting has to reach the server — it is `AlbumOrder` there, and
 * four of its values filter as well as order — while filtering must not, since
 * the client already holds the list.
 */
test("sorts through the server and filters in the browser", async ({
  page,
}) => {
  const requested: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/albums") {
      requested.push(url.searchParams.get("sort"));
    }
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();

  const titles = page.locator(".grid strong");
  await expect(titles).toHaveText(["Post", "Vespertine"]);
  // The page names its order on the first load rather than relying on the
  // server default, so the menu and the request cannot disagree.
  expect(requested).toEqual(["alphabeticalByName"]);

  await page.getByLabel("Sort").selectOption("newest");
  await expect(titles).toHaveText(["Vespertine", "Post"]);
  expect(requested).toEqual(["alphabeticalByName", "newest"]);

  // Filtering narrows what is already loaded: no further request is made, and
  // the header count reports the subset against the whole.
  await page.getByLabel("Filter by title or artist").fill("vesper");
  await expect(titles).toHaveText(["Vespertine"]);
  await expect(page.getByText("1 of 2 shown")).toBeVisible();
  expect(requested).toEqual(["alphabeticalByName", "newest"]);

  // A filter matching nothing says so instead of showing an empty grid.
  await page.getByLabel("Filter by title or artist").fill("zzz");
  await expect(page.getByText("Nothing matches that filter.")).toBeVisible();
});

/**
 * Covers are asked for when they are about to be seen, not when the page mounts.
 *
 * `/api/v2/artwork/{hash}` needs an `Authorization` header, so each thumbnail
 * costs a `fetch` of its own. A library of 164 tracks opened with 127 requests
 * in half a second against a connection limit of six, and the covers below the
 * fold were queued ahead of the ones being looked at.
 *
 * Asserted on one card at the far end rather than on a count: a count depends
 * on the viewport and would have to be loosened until it stopped meaning
 * anything. What this pins is the property — off screen is not asked for, and
 * scrolling to it asks.
 */
test("asks for a cover when it comes into view, and not before", async ({
  page,
}) => {
  const asked: string[] = [];
  const shelf = Array.from({ length: 60 }, (_, index) => ({
    id: `album-lazy-${index}`,
    library_id: "library-1",
    title: `Shelf ${String(index).padStart(2, "0")}`,
    artist: "Rue Delacour",
    artist_id: "artist-1",
    artwork_hash: `hash-${index}`,
    year: 2000 + index,
    starred_at: null,
    user_rating: null,
  }));

  // Registered after `mockAuthenticatedApi`, so these win: Playwright tries
  // handlers newest first.
  await page.route("**/api/v2/albums*", async (route) => {
    await route.fulfill({ json: shelf });
  });
  await page.route("**/api/v2/artwork/*", async (route) => {
    asked.push(new URL(route.request().url()).pathname.split("/").pop() ?? "");
    // A 404 is enough: this is about which requests leave, not what comes back.
    await route.fulfill({ status: 404, body: "" });
  });

  await page.goto("/");
  const last = page.getByText("Shelf 59");
  await expect(last).toBeVisible();

  // The top of the shelf is on screen and has been asked for; the bottom is
  // sixty cards away and has not.
  await expect.poll(() => asked).toContain("hash-0");
  expect(asked).not.toContain("hash-59");

  await last.scrollIntoViewIfNeeded();
  await expect.poll(() => asked).toContain("hash-59");
});

/**
 * The card actions guard themselves while their album's tracks are being
 * fetched. The guard was one album id, which meant two cards in flight shared
 * a single slot: pressing the second card cleared the first one's guard
 * outright, and its buttons came back to life with its own request still out.
 *
 * That mattered because one of the two actions appends. A second press on "add
 * to queue" while the first was unanswered queued the album twice.
 */
test("keeps each card's actions guarded while its own fetch is out", async ({
  page,
}) => {
  let releaseSlow = () => {};
  slowAlbum = new Promise<void>((resolve) => {
    releaseSlow = resolve;
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();

  const slowPlay = page.getByRole("button", { name: "Play: Post" });
  const fastQueue = page.getByRole("button", {
    name: "Add to queue: Vespertine",
  });

  await page.locator(".grid li").first().hover();
  await slowPlay.click();
  await expect(slowPlay).toBeDisabled();

  // The second album answers at once while the first is still held open.
  // Synchronising on the response and not on the button is the point: the
  // button is enabled before the click too, so `toBeEnabled` can resolve on
  // its first poll, before React has even applied the disabling update — and
  // the assertion below would then run at a moment when nothing has happened,
  // which is exactly when the single-slot guard still looks correct.
  const answered = page.waitForResponse(
    (response) => new URL(response.url()).pathname === "/api/v2/albums/album-2",
  );
  await page.locator(".grid li").nth(1).hover();
  await fastQueue.click();
  await answered;
  await expect(fastQueue).toBeEnabled();

  // The first album has not answered, so its actions must still be refused.
  await expect(slowPlay).toBeDisabled();

  releaseSlow();
  await expect(slowPlay).toBeEnabled();
});

test("album view plays its tracks and keeps its compact list readable", async ({
  page,
}) => {
  // "More by" reads the artist's albums alone. The library listing would page
  // through every album to keep six, and the artist's page asks a metadata
  // provider; neither has any business being asked from here.
  const listings: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path === "/api/v2/albums" || path === "/api/v2/artists/artist-1") {
      listings.push(path);
    }
  });
  await page.goto("/albums/album-2");
  await expect(page.locator(".album-hero h2")).toHaveText("Vespertine");
  await expect(page.locator(".album-meta")).toContainText("3 tracks");
  await expect(page.locator(".album-meta")).toContainText("15:00");
  await expect(page.locator(".album-songs tbody tr")).toHaveCount(3);
  await expect(page.locator(".album-songs .song-cover")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "More by Björk" })).toBeVisible();
  await expect(page.locator(".album-related-grid a")).toHaveCount(1);
  expect(listings).toEqual([]);
  const actions = await page.locator(".album-actions button").evaluateAll((buttons) =>
    buttons.map((button) => {
      const style = getComputedStyle(button);
      return {
        left: Number.parseFloat(style.paddingLeft),
        right: Number.parseFloat(style.paddingRight),
      };
    }),
  );
  expect(actions).toHaveLength(2);
  for (const action of actions) {
    expect(action.left).toBeGreaterThanOrEqual(20);
    expect(action.right).toBeGreaterThanOrEqual(20);
  }

  await page.getByRole("button", { name: "Play album" }).click();
  await expect(page.locator(".player-track strong")).toHaveText("Hidden Place");
  await page.locator(".album-songs .song-title").nth(1).click();
  await expect(page.locator(".player-track strong")).toHaveText("Cocoon");
  expect(
    await page.evaluate(() =>
      document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});

/**
 * The song table carries the controls the albums grid does not — a five-star
 * rating and a favourite — so the accessibility sweep has to reach a page that
 * shows one. Until this test the sweep only ever loaded the grid.
 */
test("rates a track and stays free of WCAG A or AA violations", async ({
  page,
}) => {
  const rated: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith("/api/v2/ratings/")) {
      rated.push(`${request.method()} ${url.pathname}`);
    }
  });

  await page.goto("/albums/album-2");
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();

  // The rating is a radio group, so the stored value is a checked radio rather
  // than a class on a span.
  const hidden = page.getByRole("group", { name: "Rating: Hidden Place" });
  await expect(hidden.getByRole("radio", { name: "5 stars" })).toBeChecked();
  const cocoon = page.getByRole("group", { name: "Rating: Cocoon" });
  await expect(cocoon.getByRole("radio", { checked: true })).toHaveCount(0);

  await cocoon.getByRole("radio", { name: "4 stars" }).check();
  await expect(cocoon.getByRole("radio", { name: "4 stars" })).toBeChecked();
  expect(rated).toEqual(["PUT /api/v2/ratings/track/song-2"]);

  await expectNoWcagViolations(page);
});

/**
 * Genres were a route the server answered and the client never called. The
 * navigation is the part worth pinning: the genre name travels into the query,
 * and it is a display string — "Hip-Hop" and "hip hop" are one genre to the
 * server, which canonicalises before matching.
 */
test("browses into a genre and keeps WCAG A and AA clean", async ({ page }) => {
  const asked: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/songs/by-genre") {
      asked.push(url.searchParams.get("genre"));
    }
  });

  await page.goto("/genres");
  await expect(page.getByRole("heading", { name: "Genres" })).toBeVisible();
  await expect(page.getByText("412 tracks · 31 albums")).toBeVisible();

  await expectNoWcagViolations(page);

  await page.getByRole("link", { name: "Art Pop" }).click();
  await expect(page.getByRole("heading", { name: "Art Pop" })).toBeVisible();
  await expect(page.locator(".songs tbody tr")).toHaveCount(2);
  expect(asked).toEqual(["Art Pop"]);

  await expectNoWcagViolations(page);
});

/**
 * A playlist file arrives, and nothing is created until the losses are shown.
 *
 * The panel is two steps because the failure it exists to prevent is a quiet
 * one: a file from another player names paths from another filesystem, so some
 * entries will not be here, and an import that silently produced a shorter
 * playlist would leave no way to know. So the first press only asks, and the
 * report is on screen — by line, with a reason — before anything is written.
 */
test("previews an .m3u import, shows what it cannot place, then imports", async ({
  page,
}) => {
  await page.goto("/playlists");
  await expect(
    page.getByRole("heading", { name: "Bring playlists from another player" }),
  ).toBeVisible();

  // The file is chosen, and only a dry run happens.
  await page.getByLabel("Choose a playlist file").setInputFiles({
    name: "Soirée.m3u",
    mimeType: "audio/x-mpegurl",
    buffer: Buffer.from("#EXTM3U\nA/01.flac\n"),
  });
  await expect(
    page.getByText("3 of 5 entries are in your library."),
  ).toBeVisible();
  expect(importAsks).toEqual([{ dryRun: true, name: "Soirée" }]);
  // The name offered is the file's own, without its extension, and editable.
  await expect(page.getByLabel("Playlist name", { exact: true })).toHaveValue("Soirée");

  // Both losses are named, with their line and their reason.
  await expect(page.getByText("2 entries cannot be placed:")).toBeVisible();
  const losses = page.locator(".import-unresolved li");
  await expect(losses).toHaveCount(2);
  await expect(losses.first()).toContainText("Line 7");
  await expect(losses.first()).toContainText("matches several tracks");
  await expect(losses.nth(1)).toContainText("not a file in a library");
  // And the encoding, because a file that was not UTF-8 may have mangled
  // names and the reader should know which guess was made.
  await expect(
    page.getByText(
      "The file was not UTF-8 and was read as windows-1252. Check the names below before importing.",
    ),
  ).toBeVisible();

  await expectNoWcagViolations(page);

  // Cancelling writes nothing, and asks nothing more.
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByLabel("Playlist name", { exact: true })).toHaveCount(0);
  expect(importAsks).toHaveLength(1);

  // And the panel opens again after a cancel, on a fresh choice.
  //
  // **It does not pin the input being cleared between choices**, which is what
  // lets a real user re-choose a file they have just fixed: `setInputFiles`
  // fires a change event whether or not the file changed, so this harness
  // cannot tell the two apart. Removing the reset leaves these assertions
  // green — measured, not assumed.
  await page.getByLabel("Choose a playlist file").setInputFiles({
    name: "Soirée.m3u",
    mimeType: "audio/x-mpegurl",
    buffer: Buffer.from("#EXTM3U\nA/01.flac\n"),
  });
  await expect(page.getByLabel("Playlist name", { exact: true })).toBeVisible();
  expect(importAsks).toHaveLength(2);

  // A name of the reader's choosing, then the commit.
  await page.getByLabel("Playlist name", { exact: true }).fill("Evening from elsewhere");
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.getByLabel("Playlist name", { exact: true })).toHaveCount(0);
  expect(importAsks[2]).toEqual({
    dryRun: false,
    name: "Evening from elsewhere",
  });
});

/**
 * Saving a playlist as a file, which is the other half of arriving with one.
 */
test("saves a playlist as an .m3u8 the next player can read", async ({
  page,
}) => {
  await page.goto("/playlists");
  await expect(page.getByRole("heading", { name: "Evening" })).toBeVisible();

  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Save as .m3u8" }).click();
  const saved = await download;
  // Named after the playlist, because a folder of "playlist.m3u8" is no use.
  expect(saved.suggestedFilename()).toBe("Evening.m3u8");
  expect(exportAsks).toEqual(["/api/v2/playlists/playlist-1/export"]);
});

/**
 * A playlist holds a position, not a set: the same track can be on it twice.
 * Each row's rating is a radio group named after the row, and a name built
 * from the track alone put both copies in one group — the browser keeps one
 * checked radio per group, so the first copy showed no rating at all.
 */
test("shows the same track twice in a playlist, each with its rating", async ({
  page,
}) => {
  await page.route("**/api/v2/playlists", (route) =>
    route.fulfill({
      json: [
        {
          ...playlists[0],
          songs: [
            song(1, "Hidden Place", 5, true),
            song(2, "Cocoon", 0, false),
            song(1, "Hidden Place", 5, true),
          ],
        },
      ],
    }),
  );
  await page.goto("/playlists");
  await expect(page.getByRole("heading", { name: "Evening" })).toBeVisible();

  const copies = page.getByRole("group", { name: "Rating: Hidden Place" });
  await expect(copies).toHaveCount(2);
  await expect(
    copies.first().getByRole("radio", { name: "5 stars" }),
  ).toBeChecked();
  await expect(
    copies.last().getByRole("radio", { name: "5 stars" }),
  ).toBeChecked();
});

/**
 * An artist page carries what this account played, and nothing when it played
 * nothing.
 *
 * The desktop client fills this part of an artist page from a remote service.
 * This server queries none, so what is counted is local listening — which also
 * means a fresh account sees no section at all, and that is the half worth
 * pinning: a heading over an empty table would read as a broken feature rather
 * than as an honest answer.
 */
test("an artist page shows this account's most played, and no heading without a history", async ({
  page,
}) => {
  await page.goto("/artists/artist-1");
  await expect(
    page.getByRole("heading", { name: "Björk", level: 2 }),
  ).toBeVisible();

  await expect(
    page.getByRole("heading", { name: "Most played here" }),
  ).toBeVisible();
  // The server ranked them; the client renders that order and does not re-sort.
  await expect(page.locator(".artist-played .songs tbody tr")).toHaveCount(2);
  await expect(
    page.locator(".artist-played .song-title").first(),
  ).toContainText("Cocoon");
  // A screenful, not the discography.
  expect(topSongAsks).toEqual(["10"]);
  // And the albums are still what the page is.
  await expect(page.getByRole("link", { name: "Open the album: Vespertine" })).toBeVisible();

  await expectNoWcagViolations(page);

  // Nothing played: the section is absent, not empty.
  artistPlayed = [];
  await page.goto("/artists/artist-1");
  await expect(
    page.getByRole("heading", { name: "Björk", level: 2 }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: "Open the album: Vespertine" })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Most played here" }),
  ).toHaveCount(0);
});

/**
 * RFC-011 decision 11 on the screen: what a provider said, and which one said
 * it. The first answer is `pending`, as an artist's first visit is; the page
 * asks again on its own, since the web client reads no event stream, and the
 * second answer fills it.
 */
test("an artist page shows what the server learned, and where each part came from", async ({
  page,
}) => {
  const lastfm = { provider: "lastfm", fetched_at: Date.UTC(2026, 9, 14) };
  enrichmentOf = (ask) =>
    ask === 1
      ? { state: "pending", bio: null, image: null, similar: null }
      : {
          state: "ready",
          bio: {
            summary: "An Icelandic singer and songwriter.",
            full: "An Icelandic singer and songwriter.\nHer albums are many.",
            source: lastfm,
          },
          image: {
            artwork_hash: "a".repeat(64),
            source: { provider: "deezer", fetched_at: Date.UTC(2026, 9, 15) },
          },
          similar: {
            artists: [
              {
                id: "artist-2",
                name: "Sigur Rós",
                album_count: 1,
                artwork_hash: null,
              },
            ],
            source: lastfm,
          },
        };
  // Only the portrait's hash answers with an image: the artist row names
  // none, so the hero can show it only through `enrichment.image`.
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route(`**/api/v2/artwork/${"a".repeat(64)}`, async (route) => {
    await route.fulfill({ contentType: "image/png", body: png });
  });

  await page.goto("/artists/artist-1");
  await expect(page.getByRole("status")).toContainText(
    "Looking this artist up",
  );
  await expect(page.getByRole("heading", { name: "About" })).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByText("An Icelandic singer and songwriter.").first()).toBeVisible();
  await expect(page.getByText(/^Biography from Last\.fm, /)).toBeVisible();
  await expect(page.getByText(/^Portrait from Deezer, /)).toBeVisible();
  await expect(page.locator(".artist-hero img.artist-portrait")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Similar artists in your library" }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: /Sigur Rós/ })).toBeVisible();
  await expect(page.getByText(/^Suggested by Last\.fm, /)).toBeVisible();
  await expect(page.getByText("Looking this artist up")).toHaveCount(0);
  expect(artistAsks).toBe(2);

  await expectNoWcagViolations(page);
});

/** `none` is an answer: nothing is shown, and nothing is asked again. */
test("an artist nobody knows shows no enrichment and is not asked again", async ({
  page,
}) => {
  enrichmentOf = () => ({ state: "none", bio: null, image: null, similar: null });
  await page.goto("/artists/artist-1");
  await expect(page.getByRole("link", { name: "Open the album: Vespertine" })).toBeVisible();
  await page.waitForTimeout(4_000);
  await expect(page.getByRole("heading", { name: "About" })).toHaveCount(0);
  await expect(page.getByText("Looking this artist up")).toHaveCount(0);
  expect(artistAsks).toBe(1);
});

/**
 * A server that fails after a `pending` answer leaves that answer on screen,
 * and a failed ask does not move the count of answers. The failures are
 * counted with them, so the page still stops at the bound.
 */
test("a pending artist whose server keeps failing is asked a bounded number of times", async ({
  page,
}) => {
  test.setTimeout(45_000);
  artistFailsAfterFirst = true;
  enrichmentOf = () => ({ state: "pending", bio: null, image: null, similar: null });
  await page.goto("/artists/artist-1");
  await expect(page.getByRole("heading", { name: "Björk", level: 2 })).toBeVisible();
  await expect.poll(() => artistAsks, { timeout: 30_000 }).toBe(6);
  await page.waitForTimeout(5_000);
  expect(artistAsks).toBe(6);
  await expect(page.getByText("Looking this artist up")).toHaveCount(0);
});

/**
 * `GET /history` answers plays and not songs, so every distinct track on this
 * screen is a request of its own, and they all leave together. Without a cap a
 * long history meant a hundred-odd round trips for a list nobody reads to the
 * bottom of. The window stays wide — 200 plays asked for — and only what is
 * resolved is bounded.
 */
test("resolves a bounded number of tracks from a long history", async ({
  page,
}) => {
  // The track itself and nothing under it: `/tracks/{id}/stream-ticket` shares
  // the prefix, and the restored queue asks for those as soon as it loads.
  const asked: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (/^\/api\/v2\/tracks\/[^/]+$/.test(path)) asked.push(path);
  });

  await page.goto("/history");
  await expect(
    page.getByRole("heading", { name: "Recently played" }),
  ).toBeVisible();

  await expect(page.locator(".songs tbody tr")).toHaveCount(50);
  expect(asked).toHaveLength(50);
  // The newest plays are the ones kept, not an arbitrary fifty.
  expect(asked[0]).toBe("/api/v2/tracks/t0");
  expect(asked.at(-1)).toBe("/api/v2/tracks/t49");
});

/**
 * The player bar's modes. `shuffledOrder`, `advance` and `retreat` are unit
 * tested; this is the wiring — that the buttons reach them, and that repeat
 * says which of its three states is on rather than only naming the action. A
 * screen reader hearing "repeat" alone could not tell.
 *
 * No audio is served: the bar renders from the restored queue, which is enough
 * to press its controls.
 */
test("carries shuffle and the three repeat states in the player bar", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  const shuffle = page.getByRole("button", { name: "Shuffle" });

  if (testInfo.project.name === "mobile") {
    // Deliberate: a narrow bar keeps the transport and drops the modes. The
    // layout that came before dropped previous and next instead and kept only
    // play, which is the worse trade on a phone.
    await expect(shuffle).toBeHidden();
    await expect(page.getByRole("button", { name: "Repeat: off" })).toBeHidden();
    await expect(page.getByRole("button", { name: "Next" })).toBeVisible();
    return;
  }

  await expect(shuffle).toBeVisible();
  await expect(shuffle).toHaveAttribute("aria-pressed", "false");
  await shuffle.click();
  await expect(shuffle).toHaveAttribute("aria-pressed", "true");
  await shuffle.click();
  await expect(shuffle).toHaveAttribute("aria-pressed", "false");

  // Off → whole queue → this track → off.
  const repeat = page.getByRole("button", { name: "Repeat: off" });
  await repeat.click();
  await expect(
    page.getByRole("button", { name: "Repeat: the whole queue" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Repeat: the whole queue" }).click();
  await expect(
    page.getByRole("button", { name: "Repeat: this track" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Repeat: this track" }).click();
  await expect(page.getByRole("button", { name: "Repeat: off" })).toBeVisible();

  // The cover is the way back to the album that is playing.
  await expect(
    page.getByRole("link", { name: "Open the album: Vespertine" }),
  ).toHaveAttribute("href", "/albums/album-2");
});

test("shows queue artwork beside tracks without crowding times or actions", async ({
  page,
}) => {
  const songs = [
    { ...track, artwork_hash: "queue-cover-1" },
    {
      ...track,
      id: "song-2",
      title: "A much longer title in the listening queue",
      duration_ms: 92_000,
      artwork_hash: "queue-cover-2",
    },
    {
      ...track,
      id: "song-3",
      title: "Rising",
      duration_ms: 236_000,
      artwork_hash: "queue-cover-3",
    },
  ];
  await page.route("**/api/v2/queue", async (route) => {
    await route.fulfill({
      json: {
        current: "song-1",
        position_ms: 0,
        changed_by: null,
        updated_at: 1,
        songs,
      },
    });
  });
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/api/v2/artwork/*", async (route) => {
    await route.fulfill({ contentType: "image/png", body: png });
  });

  await page.goto("/queue");
  const rows = page.locator(".queue-list li");
  await expect(rows).toHaveCount(3);
  await rows.last().scrollIntoViewIfNeeded();
  const covers = page.locator(".queue-list img.queue-cover");
  await expect(covers).toHaveCount(3);
  await expect
    .poll(() =>
      covers.evaluateAll((images) =>
        images.every((image) => (image as HTMLImageElement).naturalWidth > 0),
      ),
    )
    .toBe(true);
  const timeEdges = await page.locator(".queue-duration").evaluateAll((times) =>
    times.map((time) => Math.round(time.getBoundingClientRect().right)),
  );
  expect(new Set(timeEdges).size).toBe(1);

  await page.setViewportSize({ width: 320, height: 700 });
  const narrow = await rows.first().evaluate((row) => {
    const [song, time, remove] = [...row.children].map((child) =>
      child.getBoundingClientRect(),
    );
    return {
      songRight: song?.right ?? 0,
      timeLeft: time?.left ?? 0,
      timeRight: time?.right ?? 0,
      removeLeft: remove?.left ?? 0,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    };
  });
  expect(narrow.songRight).toBeLessThanOrEqual(narrow.timeLeft);
  expect(narrow.timeRight).toBeLessThanOrEqual(narrow.removeLeft);
  expect(narrow.overflow).toBeLessThanOrEqual(1);
});

/**
 * The web client works inside one library at a time — decided 2026-09-07, and
 * the contract is that changing library changes the catalogue's scope rather
 * than merging catalogues. What is worth pinning is that the scope actually
 * travels: a picker that changed nothing on the wire would look right and be
 * wrong.
 */
test("scopes the catalogue to the active library, and hides the picker when there is one", async ({
  page,
}, testInfo) => {
  const scopes: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/albums") {
      scopes.push(url.searchParams.get("library_id"));
    }
  });

  libraries = [
    library("library-1", "Ma musique"),
    library("library-2", "Les enfants"),
  ];

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();

  // The picker exists in the sidebar and in the mobile header; only one of the
  // two is on screen, so the locator has to say which — as the appearance test
  // above already does for theme and language.
  const chrome =
    testInfo.project.name === "mobile"
      ? page.locator(".mobile-header")
      : page.locator(".sidebar");
  const picker = chrome.getByLabel("Library");
  await expect(picker).toBeVisible();
  // The first library leads until one has been chosen.
  await expect.poll(() => scopes.at(-1)).toBe("library-1");

  await picker.selectOption("library-2");
  await expect.poll(() => scopes.at(-1)).toBe("library-2");

  // Remembered across a reload rather than reset to the first.
  await page.reload();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect.poll(() => scopes.at(-1)).toBe("library-2");
});

test("shows no library picker when the account has only one", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  // A select with a single option is a control that does nothing.
  await expect(page.getByLabel("Library")).toHaveCount(0);
});

test("keeps the catalogue unasked until the active library is known", async ({
  page,
}) => {
  // The first render used to fire an unscoped request and then a scoped one,
  // so for a moment the screen showed every library's albums — the exact thing
  // the single-library scope exists to prevent.
  const scopes: Array<string | null> = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v2/albums") {
      scopes.push(url.searchParams.get("library_id"));
    }
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(scopes).toEqual(["library-1"]);
});

/**
 * Live scan progress. The stream is read with `fetch` and not `EventSource`,
 * because the route authenticates on a bearer token and `EventSource` sends no
 * headers — the same wall `<audio src>` hits. That puts the framing on the
 * client, so this covers a stream that arrives in pieces with a keep-alive
 * comment in the middle of it.
 */
test("follows a scan while it runs", async ({ page }) => {
  await page.goto("/admin");
  await expect(
    page.getByRole("heading", { name: "Administration" }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Scan now" }).first().click();

  const panel = page.locator(".scan-panel");
  await expect(panel).toBeVisible();
  // The snapshot lands first, then the progress frame replaces it whole.
  await expect(panel.getByText("10 of 10 files")).toBeVisible();
  await expect(panel.getByText("Finished")).toBeVisible();
  await expect(
    panel.getByText("10 added · 0 updated · 0 moved · 0 unchanged · 0 errors"),
  ).toBeVisible();
  // A finished scan stops showing the file it is on.
  await expect(panel.locator(".scan-path")).toHaveCount(0);
});

/**
 * API tokens are rendered once per account, so loading them on mount meant one
 * request per account every time the admin screen opened — for a list almost
 * nobody opens. They wait for the panel to be opened.
 */
test("asks for a account's tokens only when its panel is opened", async ({
  page,
}) => {
  const asked: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/tokens")) asked.push(path);
  });

  await page.goto("/admin");
  await expect(
    page.getByRole("heading", { name: "Administration" }),
  ).toBeVisible();
  const disclosure = page.getByRole("button", {
    name: "API tokens for listener",
  });
  await expect(disclosure).toHaveAttribute("aria-expanded", "false");
  expect(asked).toEqual([]);

  await disclosure.click();
  await expect(disclosure).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByText("No token on this account.")).toBeVisible();
  expect(asked).toEqual(["/api/v2/admin/users/listener/tokens"]);
});

/**
 * Both of these were failures wearing the clothes of an ordinary state: a
 * token list that could not be fetched read as "no token on this account", and
 * a progress stream that could not open left the panel waiting for a first
 * reading that was never coming. That is the kind of defect nobody reports,
 * because the screen looks like it is working.
 */
test("says a token list failed instead of calling it empty", async ({
  page,
}) => {
  tokensFail = true;
  await page.goto("/admin");
  await page
    .getByRole("button", { name: "API tokens for listener" })
    .click();

  // Scoped to the panel: the player bar raises an alert of its own here, its
  // stream being mocked away.
  await expect(page.locator(".token-panel").getByRole("alert")).toHaveText(
    "We could not load this view",
  );
  await expect(page.getByText("No token on this account.")).toHaveCount(0);
});

test("says the progress stream was lost instead of waiting for ever", async ({
  page,
}) => {
  scanFails = true;
  await page.goto("/admin");
  await page.getByRole("button", { name: "Scan now" }).first().click();

  const panel = page.locator(".scan-panel");
  await expect(panel.getByRole("alert")).toContainText(
    "The progress stream could not be opened",
  );
  await expect(panel.getByText("Waiting for the first reading")).toHaveCount(0);
});

/**
 * A refused status is the tidy failure. The likelier one is the request never
 * completing at all, and that arrives as a rejected `fetch` rather than a
 * response — which the stream reader used to swallow whole, leaving the panel
 * waiting on a reading that had no chance of coming.
 */
test("says so when the progress stream never connects", async ({ page }) => {
  scanDrops = true;
  await page.goto("/admin");
  await page.getByRole("button", { name: "Scan now" }).first().click();

  const panel = page.locator(".scan-panel");
  await expect(panel.getByRole("alert")).toContainText(
    "The progress stream could not be opened",
  );
});

/**
 * Library membership could be written since M4 and never read, so an interface
 * could grant and revoke without ever showing who already had access. The list
 * is behind a disclosure for the same reason the tokens are: the panel renders
 * once per library, and loading on mount would ask for every membership list
 * every time the admin screen opened.
 */
test("lists who may see a library, once its panel is opened", async ({
  page,
}) => {
  const asked: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/members")) asked.push(path);
  });

  // Two libraries, because one cannot tell the two layouts apart: with a single
  // library, "list then panel" and "list, then every panel" produce the same
  // rows. The defect only shows from the second library on.
  libraries = [
    library("library-1", "Ma musique"),
    library("library-2", "Les enfants"),
  ];

  await page.goto("/admin");
  const disclosure = page
    .getByRole("button", { name: "Who may see this library" })
    .first();
  await expect(disclosure).toHaveAttribute("aria-expanded", "false");
  expect(asked).toEqual([]);

  // Each library's membership sits directly under that library, not after the
  // whole list: two render passes put the third library's members four rows
  // below it, which reads as belonging to whatever is above them.
  const rows = page.locator(".admin-panel .resource-list > li");
  await expect(rows.nth(0)).toContainText("Ma musique");
  await expect(rows.nth(1)).toContainText("Who may see this library");
  await expect(rows.nth(2)).toContainText("Les enfants");
  await expect(rows.nth(3)).toContainText("Who may see this library");

  await disclosure.click();
  await expect(page.getByText("guest")).toBeVisible();
  expect(asked).toEqual(["/api/v2/libraries/library-1/members"]);

  // The owner is shown and offers no role control: the route refuses `owner`
  // outright, so a select here would be offering a refusal.
  await expect(page.getByText("owner, and stays one")).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Role: listener" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("combobox", { name: "Role: guest" }),
  ).toHaveValue("listener");
});

/**
 * A membership list that could not be read is not an empty one. Standing an
 * empty array in for "not known yet" made every account on the server look like
 * a non-member, so the panel offered access to people who already had it —
 * printed underneath the notice saying the list could not be read.
 */
test("offers no membership to grant while the list is unknown", async ({
  page,
}) => {
  membersFail = true;
  await page.goto("/admin");
  await page
    .getByRole("button", { name: "Who may see this library" })
    .first()
    .click();

  const panel = page.locator(".member-row .admin-panel").first();
  await expect(panel.getByRole("alert")).toHaveText(
    "We could not load this view",
  );
  // The grant control is built from the list, so without one there is nothing
  // to build it from.
  await expect(panel.getByLabel("Give access to")).toHaveCount(0);
});

/**
 * The editor's whole job, seen from the wire. The body must name the field that
 * changed and the list handed back to the file, and nothing else — above all
 * not the comment another client added, which a body sending the whole form
 * would have erased before #177 and would pin now.
 */
test("corrects a track's tags and sends only what changed", async ({
  page,
}) => {
  await page.goto("/tracks/song-9/edit");
  await expect(
    page.getByRole("heading", { name: "Correct tags" }),
  ).toBeVisible();

  const title = page.getByLabel("Title", { exact: true });
  await expect(title).toHaveValue("Army Of Me");
  // Provenance is shown here and nowhere else: the corrected value, and what
  // the file says beneath it.
  // A tagger's comment keeps both of its lines: an `<input>` would flatten it.
  await expect(page.getByLabel("Comment", { exact: true })).toHaveValue(
    "Remastered\nBonus edition",
  );
  await expect(page.getByText("The file says nothing here")).toBeVisible();

  await title.fill("Army of Me");
  await page
    .getByRole("button", { name: /^Restore the file.s value: Artists$/ })
    .click();
  await expect(page.getByLabel("Artists", { exact: true })).toBeDisabled();

  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Saved.")).toBeVisible();
  expect(corrections).toEqual([{ title: "Army of Me", artists: null }]);

  // Read back, not assumed. The form now stands on the corrections the server
  // holds: the artists are the file's again, and no longer corrected.
  await expect(title).toHaveValue("Army of Me");
  const artists = page.getByLabel("Artists", { exact: true });
  await expect(artists).toBeEnabled();
  await expect(artists).toHaveValue("Björk");
  await expect(
    page.getByRole("button", { name: /^Restore the file.s value: Artists$/ }),
  ).toHaveCount(0);

  await expectNoWcagViolations(page);
});

/**
 * Offered where the server would accept it, and nowhere else. The listener half
 * is not vacuous: the album waits for the list of libraries before it renders,
 * so the rows below exist only once the role that decides them is known.
 */
test("offers a tag correction only to an owner or a manager", async ({
  page,
}) => {
  await page.goto("/albums/album-2");
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  const links = page.getByRole("link", { name: /^Correct tags: / });
  await expect(links).toHaveCount(3);
  await expect(links.first()).toHaveAttribute("href", "/tracks/song-1/edit");

  // A manager may correct as well: the other half of the pair the server
  // names, and the half a regression to owner-only would drop.
  libraries = [{ ...library("library-1", "Ma musique"), role: "manager" }];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  await expect(links).toHaveCount(3);

  libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  // The row's own marks cell, where the link would be. Once its rating is in
  // the page, the absence of the link is an answer and not a race.
  await expect(
    page.getByRole("group", { name: "Rating: Hidden Place" }),
  ).toBeAttached();
  await expect(links).toHaveCount(0);
});

/**
 * A file goes through the three steps RFC-008 describes, and the wire carries
 * what the server checks: the whole file's BLAKE3 as the offer, fragments of
 * exactly the advertised size in order, and one commit.
 *
 * The acknowledgement of the second fragment is lost on purpose. The client
 * must read the session back and carry on from where the server stands: not
 * send the written fragment again, and not skip the next one.
 */
test("uploads a file in fragments, resuming from the server's account", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  loseAcknowledgementOf = 1;
  const bytes = Buffer.from(Array.from({ length: 20 }, (_, i) => i));

  await page.goto("/upload");
  await expect(
    page.getByRole("heading", { name: "Upload to Ma musique" }),
  ).toBeVisible();
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "Army of Me.flac",
    mimeType: "audio/flac",
    buffer: bytes,
  });
  await expect(page.getByText("Added to the library")).toBeVisible();

  // The browser's digest is the one the server recomputes, computed here by an
  // independent call rather than read back from the page.
  expect(uploadOffers).toEqual([
    { full_hash: bytesToHex(blake3(bytes)), size_bytes: 20, extension: "flac" },
  ]);
  expect(uploadChunks.map(({ index, bytes: sent }) => [index, sent.length])).toEqual([
    [0, 8],
    [1, 8],
    [2, 4],
  ]);
  expect(uploadChunks.flatMap(({ bytes: sent }) => sent)).toEqual([...bytes]);
  expect(uploadCommits).toEqual(["upload-1"]);

  await expectNoWcagViolations(page);
});

/**
 * An extension the scanner does not index is refused on the spot. Hashing it
 * first would spend the time of a full read to be told `unsupported_format`.
 */
test("refuses a file the library cannot index, before hashing it", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  await page.goto("/upload");
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("not audio"),
  });
  await expect(
    page.getByText("Refused: not an audio format the library can index"),
  ).toBeVisible();
  expect(uploadOffers).toEqual([]);
});

/**
 * A worker that cannot load sends no message at all. Waiting for one would leave
 * the row on "Fingerprinting…" for ever, and the queue — one file at a time —
 * stuck behind it. Both files must fail, the second without waiting on the
 * first, and nothing may be offered to the server.
 */
test("fails a file whose fingerprinting worker cannot load, and moves on", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  await page.route("**/assets/hash-worker-*.js", (route) =>
    route.fulfill({ status: 404, body: "" }),
  );
  await page.goto("/upload");
  await page.getByLabel("Choose audio files").setInputFiles([
    { name: "first.flac", mimeType: "audio/flac", buffer: Buffer.from([1, 2, 3]) },
    { name: "second.flac", mimeType: "audio/flac", buffer: Buffer.from([4, 5, 6]) },
  ]);
  await expect(
    page.getByText("Failed: this browser could not fingerprint the file"),
  ).toHaveCount(2);
  expect(uploadOffers).toEqual([]);
});

/**
 * Two locks, and the link needs both: a role that may upload, and a library
 * its operator has opened. The page behind it says which one is missing when
 * reached directly.
 */
test("offers the upload only where the library takes files and the role may add them", async ({
  page,
}) => {
  // Counted in the DOM, not by role: a role query skips hidden elements, and
  // the sidebar is hidden on a phone — the count would be 0 there whether the
  // link exists or not, which makes the zero below prove nothing.
  const link = page.locator('.primary-navigation a[href="/upload"]');

  // The fixture's default: an owner, in a library closed to files.
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(link).toHaveCount(0);

  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(link).toHaveCount(1);

  libraries = [
    {
      ...library("library-1", "Ma musique"),
      accepts_uploads: true,
      role: "listener",
    },
  ];
  await page.reload();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(link).toHaveCount(0);
  await page.goto("/upload");
  await expect(
    page.getByText(
      "Only an owner or a manager of this library can add files to it.",
    ),
  ).toBeVisible();
});

/**
 * RFC-009 from the editor: a loop sent as the file's own bytes, read back
 * through its ticket rather than assumed from the answer, refused in words,
 * and taken away.
 */
test("places, replaces and removes a track's canvas", async ({ page }) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_canvas: true }];
  await page.goto("/tracks/song-9/edit");
  await expect(page.getByText("This track has no canvas.")).toBeVisible();

  const loop = Buffer.from([0x1a, 0x45, 0xdf, 0xa3, 0x01, 0x02]);
  await page.getByLabel("Choose a loop").setInputFiles({
    name: "loop.webm",
    mimeType: "video/webm",
    buffer: loop,
  });
  await expect(page.getByText("Canvas saved.")).toBeVisible();
  const preview = page.getByLabel("Canvas preview");
  await expect(preview).toHaveAttribute(
    "src",
    "/api/v2/canvas-stream/ticket-song-9",
  );

  // Refused: said in words, and the loop that was there is still there.
  refuseCanvasWith = 422;
  await page.getByLabel("Replace the loop").setInputFiles({
    name: "episode.mp4",
    mimeType: "video/mp4",
    buffer: Buffer.from([9, 9, 9]),
  });
  await expect(
    page.getByText(
      "The server refused this file: it takes a short mp4 or webm loop with a picture.",
    ),
  ).toBeVisible();
  await expect(preview).toHaveAttribute(
    "src",
    "/api/v2/canvas-stream/ticket-song-9",
  );

  await expectNoWcagViolations(page);

  await page.getByRole("button", { name: "Remove the canvas" }).click();
  await expect(page.getByText("Canvas removed.")).toBeVisible();
  await expect(page.getByText("This track has no canvas.")).toBeVisible();
  expect(canvasWrites).toEqual([
    { method: "PUT", track: "song-9", bytes: [...loop] },
    { method: "PUT", track: "song-9", bytes: [9, 9, 9] },
    { method: "DELETE", track: "song-9" },
  ]);
});

/**
 * A library its operator has not opened to loops offers no new canvas, and
 * still lets go of the one a track carries: the server does not gate removal on
 * the flag, and the screen must not strand what the server would release.
 */
test("offers removal but no new canvas where the library takes none", async ({
  page,
}) => {
  canvases.set("song-9", [1, 2, 3]);
  await page.goto("/tracks/song-9/edit");
  // The notice waits for the list of libraries. Until it answers the picker is
  // offered, so its absence below is only an answer once this is on screen.
  await expect(
    page.getByText(
      "This library does not take canvases. Its operator decides that on the server.",
    ),
  ).toBeVisible();
  await expect(page.getByLabel("Canvas preview")).toBeAttached();
  await expect(
    page.getByRole("button", { name: "Remove the canvas" }),
  ).toBeVisible();
  await expect(page.locator(".canvas-panel input[type=file]")).toHaveCount(0);
});

/**
 * The loop over the cover, rendered as the desktop renders it: muted, looping
 * and hidden from assistive technology, with the cover underneath keeping its
 * name. It steps aside when motion is reduced, and when the browser cannot play
 * it — the cover shows then, never an empty frame.
 */
/** Saved positions use their song artwork and a shared time column. */
test("shows saved tracks with artwork and one aligned time column", async ({
  page,
}) => {
  bookmarks = ["Soleil Bleu", "A much longer song title", "Rising"].map(
    (title, index) => ({
      position_ms: [224_000, 236_000, 92_000][index] as number,
      comment: null,
      created_at: 1,
      updated_at: 1,
      song: {
        id: `saved-${index}`,
        title,
        artist: "Rue Delacour",
        artwork_hash: `saved-cover-${index}`,
      },
    }),
  );
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/api/v2/artwork/*", async (route) => {
    await route.fulfill({ contentType: "image/png", body: png });
  });

  await page.goto("/playing");
  const rows = page.locator(".bookmark-list li");
  await expect(rows).toHaveCount(3);
  await rows.last().scrollIntoViewIfNeeded();
  const covers = page.locator(".bookmark-list img.bookmark-cover");
  await expect(covers).toHaveCount(3);
  await expect
    .poll(() =>
      covers.evaluateAll((images) =>
        images.every((image) => (image as HTMLImageElement).naturalWidth > 0),
      ),
    )
    .toBe(true);
  const timeEdges = await page.locator(".bookmark-position").evaluateAll(
    (times) => times.map((time) => Math.round(time.getBoundingClientRect().right)),
  );
  expect(new Set(timeEdges).size).toBe(1);
  await expect(page.locator(".playing-bookmarks")).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await page.setViewportSize({ width: 320, height: 700 });
  const narrow = await rows.first().evaluate((row) => {
    const [track, time, forget] = [...row.children].map((child) =>
      child.getBoundingClientRect(),
    );
    return {
      trackRight: track?.right ?? 0,
      timeLeft: time?.left ?? 0,
      timeRight: time?.right ?? 0,
      forgetLeft: forget?.left ?? 0,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    };
  });
  expect(narrow.trackRight).toBeLessThanOrEqual(narrow.timeLeft);
  expect(narrow.timeRight).toBeLessThanOrEqual(narrow.forgetLeft);
  expect(narrow.overflow).toBeLessThanOrEqual(1);
});

/**
 * The playing page, reading what the server actually sends.
 *
 * Until 2026-09-15 `LyricsList` declared a `snake_case` envelope the server has
 * never sent, so this page threw on its first property and showed nothing at
 * all. Nothing caught it because the mock above was written from that type. An
 * assertion on the words themselves is what tells the two shapes apart: an
 * empty sheet renders the same "no lyrics" line under either name.
 */
test("shows the words a track travels with", async ({ page }) => {
  lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: false,
      line: [{ value: "the lamps come on along the quay" }],
    },
  ]);
  await page.goto("/playing");
  await expect(
    page.getByRole("listitem").filter({
      hasText: "the lamps come on along the quay",
    }),
  ).toBeVisible();
});

/**
 * Most tracks carry no loop, and minting a ticket is how the client asks — so
 * most of the time the answer is 404. The playing screen asks from an effect on
 * mount, which made that 404 a per-navigation cost with no end to it: measured
 * in a real session as a burst in the console, one request per visit, for ever.
 *
 * Client-side navigation both ways, deliberately. `page.goto` loads a fresh
 * document and empties everything the module remembers, so a test built on it
 * would prove the reload rather than the fix.
 */
test("asks once whether a track carries a canvas, not once per visit", async ({
  page,
}) => {
  // `canvases` is left empty, so every ticket answers 404 — the shape at issue.
  const tickets: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/canvas-ticket")) tickets.push(path);
  });

  await page.goto("/playing");
  await expect(page.locator(".canvas-stage .cover")).toBeVisible();
  await expect.poll(() => tickets).toEqual([
    "/api/v2/tracks/song-1/canvas-ticket",
  ]);

  for (const _visit of [1, 2]) {
    await page.getByRole("link", { name: "Albums" }).click();
    await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
    await page.getByRole("link", { name: "Now playing" }).click();
    await expect(page.locator(".canvas-stage .cover")).toBeVisible();
  }

  // Waited for rather than asserted straight away: a request issued by the
  // mount effect leaves after the frame that made the cover visible, so
  // counting at that moment would pass whether or not one was on its way.
  await page.waitForLoadState("networkidle");
  expect(tickets).toHaveLength(1);
});

test("plays a track's canvas over its cover, and steps aside for it", async ({
  page,
}) => {
  canvases.set("song-1", [1, 2, 3]);
  let release = () => {};
  canvasStreamGate = new Promise<void>((resolve) => {
    release = () => resolve();
  });
  await page.goto("/playing");
  const stage = page.locator(".canvas-stage");
  const video = stage.locator("video");
  await expect(video).toHaveAttribute(
    "src",
    "/api/v2/canvas-stream/ticket-song-1",
  );
  await expect(video).toHaveAttribute("aria-hidden", "true");
  await expect(video).toHaveJSProperty("muted", true);
  await expect(video).toHaveJSProperty("loop", true);
  await expect(stage.locator(".cover")).toBeVisible();
  await expectNoWcagViolations(page);

  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(video).toHaveCount(0);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(video).toHaveCount(1);

  // Bytes that are not a video: the browser gives up on them, and the cover is
  // what is left rather than a frame with nothing in it.
  release();
  await expect(video).toHaveCount(0);
  await expect(stage.locator(".cover")).toBeVisible();
});

/**
 * External scrobbling — RFC-010, the four gestures a client owes.
 *
 * The path is the server's: `lastfm_callback` redirects to
 * `/settings/scrobbling`, so a page anywhere else leaves the last step of the
 * journey on the not-found screen.
 */
test("offers each instance by name, and says why one cannot be linked", async ({
  page,
}) => {
  await page.goto("/settings/scrobbling");

  await expect(
    page.getByRole("heading", { name: "External scrobbling" }),
  ).toBeVisible();
  // Two instances of one recipient, each named. A screen keyed on the
  // recipient would show "Maloja" once and hide whichever it linked second.
  await expect(page.getByText('Maloja — instance “alice”')).toBeVisible();
  await expect(page.getByText('Maloja — instance “bob”')).toBeVisible();
  await expect(
    page.getByText('ListenBrainz — instance “default”'),
  ).toBeVisible();

  // Declared and unusable is a real shape, and the reason is published so the
  // person is not left to discover it when a link fails later. Asserted on the
  // words this client chose for `NO_PUBLIC_URL`, not on the code it was given:
  // a case printed raw would satisfy any assertion made on the code itself.
  await expect(
    page.getByText(
      "Unavailable here: the browser journey needs this server’s public " +
        "address to be an https one — an operator can still link an account " +
        "from the command line",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Connect to Last.fm" }),
  ).toHaveCount(0);

  await expectNoWcagViolations(page);
});

/**
 * A server newer than the client reading it.
 *
 * `unavailable` carries a case rather than a sentence, so this client has a
 * table of words for the cases it knows — and a case it does not know must not
 * become a blank where a reason should be. The type is open for the same
 * reason: a union naming only today's cases would be an assertion about the
 * wire that the wire never made, which is the fault this whole change set
 * exists to remove.
 */
test("says a reason it does not recognise is a reason, not a blank", async ({
  page,
}) => {
  const lastfm = destinations.find((row) => row.provider === "lastfm");
  if (lastfm) lastfm.unavailable = "a_case_from_a_later_release";

  await page.goto("/settings/scrobbling");

  await expect(
    page.getByText("Unavailable here: this server did not say why"),
  ).toBeVisible();
});

/**
 * `toString` is not a translation key.
 *
 * Opening the union widened the lookup key to `string`, and a plain object
 * answers for names nobody put in it: `toString` and `constructor` come back
 * off the prototype, truthy, and would reach `t()` as if they were keys. Every
 * inherited name is a case this client does not know, and must read as one.
 */
for (const inherited of ["toString", "constructor", "hasOwnProperty"]) {
  test(`treats the inherited name ${inherited} as a case it does not know`, async ({
    page,
  }) => {
    const lastfm = destinations.find((row) => row.provider === "lastfm");
    if (lastfm) lastfm.unavailable = inherited;

    await page.goto("/settings/scrobbling");

    await expect(
      page.getByText("Unavailable here: this server did not say why"),
    ).toBeVisible();
  });
}

test("links one instance by its key, and leaves the other alone", async ({
  page,
}) => {
  await page.goto("/settings/scrobbling");
  await expect(page.getByText('Maloja — instance “alice”')).toBeVisible();

  await page
    .getByLabel('API key: Maloja — instance “alice”')
    .fill("alice-api-key");
  await page
    .locator(".scrobble-row", { hasText: "alice" })
    .getByRole("button", { name: "Link" })
    .click();

  await expect(
    page.getByRole("button", { name: 'Unlink: Maloja — instance “alice”' }),
  ).toBeVisible();
  expect(scrobbleWrites).toEqual([
    {
      method: "PUT",
      path: "/api/v2/scrobble-links/maloja/alice",
      body: { secret: "alice-api-key" },
    },
  ]);
  // The pair names the resource: bob is untouched and still offers its field.
  await expect(
    page.getByLabel('API key: Maloja — instance “bob”'),
  ).toBeVisible();

  // The queue behind the link is a state and four counters, never an echo of
  // what was heard.
  await expect(page.getByText("The queue is not moving")).toBeVisible();
  await expect(page.getByText("12 waiting")).toBeVisible();
  await expect(page.getByText("1 waiting on you")).toBeVisible();
  await expect(
    page.getByText("Last refusal: the destination asked for room."),
  ).toBeVisible();
});

test("sends the browser to Last.fm rather than asking for a key", async ({
  page,
}) => {
  // Last.fm hands out no secret a person can hold, so the only gesture is a
  // journey. Declared *and* usable here, unlike the listing test.
  destinations = [
    { provider: "lastfm" as const, destination: "default", available: true },
  ];
  await page.route("https://www.last.fm/**", (route) =>
    route.fulfill({ status: 200, body: "<title>last.fm</title>" }),
  );

  await page.goto("/settings/scrobbling");
  await page.getByRole("button", { name: "Connect to Last.fm" }).click();

  await page.waitForURL("https://www.last.fm/**");
  expect(scrobbleWrites).toEqual([
    {
      method: "POST",
      path: "/api/v2/scrobble-links/lastfm/authorize/default",
      body: undefined,
    },
  ]);
});

test("says so when the journey comes back, and names the instance", async ({
  page,
}) => {
  // What the callback redirects to once it has finished the link itself. The
  // value is shown and nothing more: anybody can type this address, and the
  // listing beside it is what actually says a link exists.
  await page.goto(
    "/settings/scrobbling?linked=lastfm&destination=default&keep=me",
  );

  await expect(page.getByText("Last.fm is linked: default.")).toBeVisible();

  // Said once, then spent. A reload is a later navigation and not this one, so
  // the pair leaves the address while the notice stays on the page — and a URL
  // copied out of the bar no longer tells somebody else their Last.fm is
  // linked. Anything else in the query is left where it was.
  await expect(page).toHaveURL(/\/settings\/scrobbling\?keep=me$/);
  await page.reload();
  await expect(page.getByText("Last.fm is linked: default.")).toHaveCount(0);
});

test("names the track behind an ambiguous listen, and answers it once", async ({
  page,
}) => {
  scrobbleLinks = [linkState("maloja", "alice")];
  await page.goto("/settings/scrobbling");

  const entry = page.locator(".scrobble-uncertain li");
  // The entry carries no title — decision 12 keeps the envelope out of the API
  // — so the screen matches `played_at` against the account's own history,
  // which is what that field is documented to be for. A bare timestamp would
  // ask somebody to decide about a listen they cannot recognise.
  await expect(entry.getByText("Track t5")).toBeVisible();
  await expect(entry.getByText(/instance “alice”/)).toBeVisible();
  await expect(entry.getByText(/the answer never arrived/)).toBeVisible();

  await entry.getByRole("button", { name: "Send it again" }).click();

  await expect(page.getByText("Nothing is waiting on you.")).toBeVisible();
  expect(scrobbleWrites).toEqual([
    { method: "POST", path: "/api/v2/scrobble-queue/uncertain/entry-1/retry" },
  ]);
});

test("throws an ambiguous listen away when that is the answer", async ({
  page,
}) => {
  scrobbleLinks = [linkState("maloja", "alice")];
  await page.goto("/settings/scrobbling");

  const entry = page.locator(".scrobble-uncertain li");
  await expect(entry.getByText("Track t5")).toBeVisible();
  await entry.getByRole("button", { name: "Throw it away" }).click();

  await expect(page.getByText("Nothing is waiting on you.")).toBeVisible();
  expect(scrobbleWrites).toEqual([
    { method: "DELETE", path: "/api/v2/scrobble-queue/uncertain/entry-1" },
  ]);
});

test("unlinks one instance and leaves a namesake at another recipient", async ({
  page,
}) => {
  // `default` is the name every recipient uses, so the pair is the only thing
  // that tells these two apart. A screen — or a mock — matching on the
  // instance name alone withdraws both, which is the fault the unique index
  // on `(user_id, provider, destination)` was rebuilt to make impossible.
  destinations = [
    { provider: "listenbrainz" as const, destination: "default", available: true },
    { provider: "maloja" as const, destination: "default", available: true },
  ];
  scrobbleLinks = [
    linkState("listenbrainz", "default"),
    linkState("maloja", "default"),
  ];

  await page.goto("/settings/scrobbling");
  await page
    .getByRole("button", {
      name: 'Unlink: ListenBrainz — instance “default”',
    })
    .click();

  await expect(
    page.getByRole("button", { name: 'Unlink: Maloja — instance “default”' }),
  ).toBeVisible();
  await expect(
    page.getByLabel('API key: ListenBrainz — instance “default”'),
  ).toBeVisible();
  expect(scrobbleWrites).toEqual([
    { method: "DELETE", path: "/api/v2/scrobble-links/listenbrainz/default" },
  ]);
});

/**
 * The mobile bar holds six slots and the client has sixteen screens. Until
 * 2026-09-15 the other eight were reachable by typing their address and by
 * nothing else: the sidebar that carries them is `display: none` below 820px.
 * The ninth is Songs, added with the listing itself, and the tenth is
 * Settings — which is where the theme and the language went when they stopped
 * living in the sidebar footer.
 * Seven of the eight had been in that state since lot A, and no test saw it —
 * every mobile case here navigates by `page.goto`.
 */
test("opens the rest of the client from the mobile bar", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "mobile", "about the phone bar");
  // An administrator of a library that takes files, so all eight are this
  // account's to see. The default fixture takes none, which left `Upload` out
  // of the list below while the comment on it claimed otherwise — and nothing
  // then proved the sheet ever shows an upload entry at all. Its absence in
  // the test beside this one is over-determined: that account is a listener
  // *and* its library is closed, so it would pass against a sheet that had
  // dropped the entry entirely.
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  await page.goto("/");

  const bar = page.locator(".mobile-navigation");
  // Six links and the seventh slot, on one row. The grid counted five for as
  // long as there were five, and a sixth primary entry added later had been
  // wrapping onto a second row ever since.
  await expect(bar.locator("a")).toHaveCount(6);
  const rows = await bar.evaluate(
    (element) =>
      new Set(
        [...element.children].map((child) =>
          Math.round(child.getBoundingClientRect().top),
        ),
      ).size,
  );
  expect(rows).toBe(1);

  // Asserted before it is clicked. Clicking a control that is not there fails
  // by timing out, and a timeout in a CI log says a test was slow rather than
  // that the seventh slot is missing.
  const more = bar.getByRole("button", { name: "More" });
  await expect(more).toBeVisible();
  await more.click();

  const sheet = page.getByRole("dialog");
  await expect(sheet).toBeVisible();
  // The twelve the bar cannot hold — including the two this account is
  // entitled to as an administrator of a library that takes files. `Radio`
  // joined them with #217, and `Folders` with RFC-012's disk tree; this count
  // is what said so: a new entry in the sidebar reaches the sheet, and the
  // sheet is pinned.
  for (const name of [
    "Artists",
    "Songs",
    "Genres",
    "Folders",
    "Settings",
    "Random",
    "Recently played",
    "Shares",
    "Radio",
    "Scrobbling",
    "Upload",
    "Admin",
  ]) {
    await expect(sheet.getByRole("link", { name })).toBeVisible();
  }
  // Counted as well as named: a sheet that also offered something nobody asked
  // for would satisfy every line above.
  await expect(sheet.getByRole("link")).toHaveCount(12);

  await expectNoWcagViolations(page);

  // Escape is the keyboard's way out, and it is the element's own rather than
  // anything written here — which is why the sheet is a <dialog>.
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();

  await bar.getByRole("button", { name: "More" }).click();
  await sheet.getByRole("link", { name: "Scrobbling" }).click();
  await expect(
    page.getByRole("heading", { name: "External scrobbling" }),
  ).toBeVisible();
  // Closed on the way out: a sheet left open would sit over the page it just
  // reached.
  await expect(sheet).toBeHidden();
});

test("offers nothing in the sheet that the account may not reach", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "mobile", "about the phone bar");
  // A listener, in a library that takes no files: upload and admin are not
  // hidden by width, they are not this account's to see at all.
  await page.route("**/api/v2/web/auth/refresh", (route) =>
    route.fulfill({
      json: { ...session, user: { ...session.user, role: "user" } },
    }),
  );
  libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];

  await page.goto("/");
  await page.locator(".mobile-navigation").getByRole("button", { name: "More" }).click();

  const sheet = page.getByRole("dialog");
  await expect(sheet.getByRole("link", { name: "Shares" })).toBeVisible();
  await expect(sheet.getByRole("link", { name: "Admin" })).toHaveCount(0);
  await expect(sheet.getByRole("link", { name: "Upload" })).toHaveCount(0);
});

/** A tab switch keeps the current screen until the destination can render. */
test("keeps the current tab visible while the next tab loads", async ({
  page,
}) => {
  const asked: string[] = [];
  let answerGenres: (() => void) | undefined;
  const genresReady = new Promise<void>((resolve) => {
    answerGenres = resolve;
  });
  await page.route("**/api/v2/genres*", async (route) => {
    asked.push(new URL(route.request().url()).pathname);
    await genresReady;
    await route.fulfill({
      json: [{ name: "Trip hop", song_count: 4, album_count: 1 }],
    });
  });
  await page.route("**/api/v2/albums*", async (route) => {
    asked.push(new URL(route.request().url()).pathname);
    await route.fulfill({ json: albums });
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const mobile = await page.locator(".mobile-navigation").isVisible();
  const genresLink = mobile
    ? page.locator('.more-sheet a[href="/genres"]')
    : page.locator(".primary-navigation").getByRole("link", { name: "Genres" });
  async function visitGenres() {
    if (mobile) {
      await page.locator(".mobile-navigation").getByRole("button", { name: "More" }).click();
    }
    await genresLink.click();
  }
  await visitGenres();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await expect(page.locator(".skeleton-grid")).toHaveCount(0);
  await expect(genresLink).toHaveAttribute(
    "aria-busy",
    "true",
  );
  answerGenres?.();
  await expect(page.getByRole("heading", { name: "Genres" })).toBeVisible();
  await page.getByRole("link", { name: "Albums" }).click();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await visitGenres();
  await expect(page.getByRole("heading", { name: "Genres" })).toBeVisible();
  await expect(page.locator(".skeleton-grid")).toHaveCount(0);
  expect(asked).toEqual(["/api/v2/albums", "/api/v2/genres"]);
});

/**
 * A tab switch waits for its answer, but not for ever.
 *
 * Past a short deadline the destination is shown with its own wait. A server
 * that never answered held the previous screen, with nothing but a busy tab
 * to say anything was happening.
 */
test("shows the next tab with its wait when the answer is slow", async ({
  page,
}) => {
  // Never answered: the deadline is the only way out.
  await page.route("**/api/v2/genres*", () => new Promise<void>(() => {}));
  await page.route("**/api/v2/albums*", (route) =>
    route.fulfill({ json: albums }),
  );

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const mobile = await page.locator(".mobile-navigation").isVisible();
  if (mobile) {
    await page
      .locator(".mobile-navigation")
      .getByRole("button", { name: "More" })
      .click();
  }
  const genresLink = mobile
    ? page.locator('.more-sheet a[href="/genres"]')
    : page.locator(".primary-navigation").getByRole("link", { name: "Genres" });
  await genresLink.click();
  await expect(page).toHaveURL(/\/genres$/);
  // The destination's own wait, which is what the visitor now looks at.
  await expect(page.locator("main .skeleton-grid")).toBeVisible();
});

/**
 * The current tab's link still goes to its plain address.
 *
 * A click on the tab already open is how a second click cancels a switch
 * still loading. With nothing loading, swallowing it left `/playing?view=lyrics`
 * where it was, though the link says `/playing`.
 */
test("takes the current tab back to its plain address", async ({ page }) => {
  await page.goto("/playing?view=lyrics");
  await expect(page).toHaveURL(/\/playing\?view=lyrics$/);
  const navigation = (await page.locator(".mobile-navigation").isVisible())
    ? page.locator(".mobile-navigation")
    : page.locator(".primary-navigation");
  await navigation.getByRole("link", { name: "Now playing" }).click();
  await expect(page).toHaveURL(/\/playing$/);
});

/**
 * The random tab draws once.
 *
 * Its answer is held for no time at all, so a draw made ahead of the switch
 * was thrown away before the page mounted, and the page drew again: two
 * requests, and a wait for both.
 */
test("draws the random tab once when switching to it", async ({ page }) => {
  let draws = 0;
  await page.route("**/api/v2/songs/random*", async (route) => {
    draws += 1;
    await route.fulfill({ json: [track] });
  });
  await page.route("**/api/v2/albums*", (route) =>
    route.fulfill({ json: albums }),
  );

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  if (await page.locator(".mobile-navigation").isVisible()) {
    await page.locator(".mobile-navigation").getByRole("button", { name: "More" }).click();
    await page.locator('.more-sheet a[href="/random"]').click();
  } else {
    await page.locator(".primary-navigation").getByRole("link", { name: "Random" }).click();
  }
  await expect(page.getByRole("heading", { name: "Random" })).toBeVisible();
  await expect(page.locator(".songs")).toContainText("Army of Me");
  // Long enough for a second draw from the mount to have been asked.
  await page.waitForTimeout(500);
  expect(draws).toBe(1);
});

/**
 * A tab that is still loading does not overrule a later move.
 *
 * The switch waits for the destination's first answer, and the page stays
 * usable meanwhile. Following a link in it during that wait is a newer
 * decision than the tab: arriving on the tab afterwards took the visitor away
 * from the album they had just opened.
 */
test("leaves a tab switch behind when the visitor moves on while it loads", async ({
  page,
}) => {
  let answerGenres: (() => void) | undefined;
  const genresReady = new Promise<void>((resolve) => {
    answerGenres = resolve;
  });
  let genresAnswered = false;
  await page.route("**/api/v2/genres*", async (route) => {
    await genresReady;
    await route.fulfill({
      json: [{ name: "Trip hop", song_count: 4, album_count: 1 }],
    });
    genresAnswered = true;
  });
  await page.route("**/api/v2/albums*", (route) =>
    route.fulfill({ json: albums }),
  );

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  const mobile = await page.locator(".mobile-navigation").isVisible();
  if (mobile) {
    await page.locator(".mobile-navigation").getByRole("button", { name: "More" }).click();
  }
  const genresLink = mobile
    ? page.locator('.more-sheet a[href="/genres"]')
    : page.locator(".primary-navigation").getByRole("link", { name: "Genres" });
  await genresLink.click();
  await expect(genresLink).toHaveAttribute("aria-busy", "true");

  await page.locator('.grid a[href="/albums/album-1"]').click();
  await expect(page).toHaveURL(/\/albums\/album-1$/);

  answerGenres?.();
  await expect.poll(() => genresAnswered).toBe(true);
  await expect(genresLink).not.toHaveAttribute("aria-busy", "true");
  // Long enough for the navigation the answer used to trigger to land.
  await page.waitForTimeout(500);
  await expect(page).toHaveURL(/\/albums\/album-1$/);
  await expect(page.getByRole("heading", { name: "Genres" })).toHaveCount(0);
});

/**
 * Signing out empties what was answered for the account that left.
 *
 * The cache is the first thing in this client that outlives a screen, and a
 * sign-out here is a client-side navigation — the document is never reloaded,
 * so nothing is forgotten on its own. A catalogue is scoped by library
 * membership and an administrator sees more than that, so carrying any of it
 * across would show the next person what the last one could see.
 *
 * Asserted on the request, because that is the only thing that distinguishes an
 * emptied cache from a full one: both render the same albums, and only one of
 * them had to ask.
 */
test("forgets the catalogue when the session changes", async ({
  page,
}, testInfo) => {
  // The sidebar that carries the sign-out is hidden below 820px, where the
  // same gesture lives behind the overflow sheet. What is being measured is
  // the cache, not where the button sits, so it is measured once.
  test.skip(testInfo.project.name !== "desktop", "about the sidebar");
  let asked = 0;
  await page.route("**/api/v2/albums*", async (route) => {
    asked += 1;
    await route.fulfill({ json: albums });
  });
  await page.route("**/api/v2/web/auth/login", async (route) => {
    await route.fulfill({ json: session });
  });

  await page.goto("/");
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(1);

  // Away and straight back, without signing out: this is the cache doing its
  // job, and the contrast that makes the count below mean something.
  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("link", { name: "Queue" }).click();
  await expect(page.getByRole("heading", { name: "Queue" })).toBeVisible();
  await sidebar.getByRole("link", { name: "Albums" }).click();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(1);

  // Dispatched rather than clicked: the sidebar's footer sits under the fixed
  // player bar at this viewport, so a real click waits on hit-testing forever.
  // What this test is about is what the sign-out does to the cache, and the
  // gesture itself is covered by the accessibility pass.
  await page
    .getByRole("button", { name: "Sign out" })
    .dispatchEvent("click");
  await expect(page).toHaveURL(/\/login$/);

  // Signed in again through the form, not by reloading the page. A reload
  // would destroy the cache whatever this code does, and would prove nothing:
  // the leak this guards against is exactly the one that needs no reload.
  await page.getByLabel("Username").fill("nocturne");
  await page.getByLabel("Password").fill("correct horse battery staple");
  await page.getByRole("button", { name: "Sign in" }).click();

  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(2);
});

/**
 * A cover already in memory does not flash grey on the way back.
 *
 * The bytes are held in a map keyed by hash, but that map holds *promises*, so
 * nothing in it could be read while rendering: every mount started with no
 * image and put the placeholder on screen for a frame — on every navigation
 * back to a grid already visited, for every cover in it. Navidrome's own image
 * cache reads its entry synchronously into `useState` for exactly this reason.
 *
 * Watched frame by frame rather than checked once, because a single frame is
 * the whole of what went wrong: an assertion that ran after the navigation
 * settled would have found the covers in place and called it fixed.
 */
test("shows a held cover at once, without a grey frame in between", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "about the sidebar");

  const shelf = albums.map((album, index) => ({
    ...album,
    artwork_hash: `hash-${index}`,
  }));
  await page.route("**/api/v2/albums*", async (route) => {
    await route.fulfill({ json: shelf });
  });
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("**/api/v2/artwork/*", async (route) => {
    await route.fulfill({ contentType: "image/png", body: png });
  });

  await page.goto("/");
  // Both covers in place, so their bytes are held before the round trip below.
  await expect(page.locator("img.cover")).toHaveCount(2);

  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("link", { name: "Queue" }).click();
  await expect(page.getByRole("heading", { name: "Queue" })).toBeVisible();

  const greyFrames = await page.evaluate(async () => {
    const link = [...document.querySelectorAll("nav a")].find((anchor) =>
      anchor.textContent?.includes("Albums"),
    ) as HTMLElement;
    let grey = 0;
    link.click();
    const started = performance.now();
    return await new Promise<number>((resolve) => {
      const tick = () => {
        grey += document.querySelectorAll(".grid .cover-fallback").length;
        if (performance.now() - started > 400) return resolve(grey);
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
  });

  // The catalogue is back and showing its covers. Asserted after the count and
  // not instead of it: zero grey frames is also what a navigation that never
  // arrived would produce, and this assertion is the whole difference between
  // measuring the fix and measuring nothing at all.
  await expect(page.locator(".grid img.cover")).toHaveCount(2);
  expect(greyFrames).toBe(0);
});

/**
 * A favourite asks the catalogue again, so the star survives the next visit.
 *
 * The table keeps its stars in a map of its own while the request is out, which
 * was enough when every screen re-read on mount: the server's answer replaced
 * that overlay on the way back. Against a cache it is not — the overlay dies
 * with the component and the held listing still carries the old `starred_at`,
 * so the star un-ticks itself on the next visit.
 *
 * Asserted on the request rather than on the star, because the mock answers a
 * fixture that never changes: what marks the difference is that the question
 * was asked at all.
 */
test("asks the catalogue again after a track is favourited", async ({
  page,
}) => {
  let asked = 0;
  await page.route("**/api/v2/albums/album-1", async (route) => {
    asked += 1;
    await route.fulfill({ json: { ...albums[0], songs: [track] } });
  });
  // The star only invalidates once the server has taken it. Answered here
  // because the harness has no favourites route, and a refused write must not
  // invalidate anything — which is the other half of what this checks.
  await page.route("**/api/v2/favorites/track/*", async (route) => {
    await route.fulfill({ status: 204, body: "" });
  });

  await page.goto("/albums/album-1");
  await expect(page.getByRole("heading", { name: "Post" })).toBeVisible();
  expect(asked).toBe(1);

  await page
    .getByRole("button", { name: "Add favourite: Army of Me" })
    .click();
  await expect
    .poll(() => asked, { message: "the album was re-asked" })
    .toBe(2);
});

/**
 * A cover that failed to load is asked for again; one the server does not hold
 * is not.
 *
 * The two used to be the same answer: any response that was not `ok`, and any
 * network failure, became `null` and was then held for the session — so a
 * single blip left a cover grey until the tab was reloaded. A 404 is the server
 * saying it holds no such artwork, which is worth keeping; anything else is a
 * server that could not answer, which is not.
 */
test("asks again for a cover that failed, and not for one that is absent", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "about the sidebar");

  const shelf = albums.map((album, index) => ({
    ...album,
    artwork_hash: index === 0 ? "hash-flaky" : "hash-absent",
  }));
  await page.route("**/api/v2/albums*", async (route) => {
    await route.fulfill({ json: shelf });
  });

  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  let flaky = 0;
  let absent = 0;
  await page.route("**/api/v2/artwork/hash-flaky", async (route) => {
    flaky += 1;
    // Down the first time, up afterwards.
    if (flaky === 1) return route.fulfill({ status: 503, body: "" });
    await route.fulfill({ contentType: "image/png", body: png });
  });
  await page.route("**/api/v2/artwork/hash-absent", async (route) => {
    absent += 1;
    await route.fulfill({ status: 404, body: "" });
  });

  await page.goto("/");
  // Waited on rather than sampled: the covers are asked for once the observer
  // says they are on screen, which is a frame or two after the grid renders.
  await expect.poll(() => flaky).toBe(1);
  await expect.poll(() => absent).toBe(1);
  await expect(page.locator(".grid .cover-fallback")).toHaveCount(2);

  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("link", { name: "Queue" }).click();
  await expect(page.getByRole("heading", { name: "Queue" })).toBeVisible();
  await sidebar.getByRole("link", { name: "Albums" }).click();

  // The one that failed is asked again and arrives; the one the server does not
  // hold is not asked again, because that answer has not changed.
  await expect(page.locator(".grid img.cover")).toHaveCount(1);
  await expect.poll(() => flaky).toBe(2);
  expect(absent).toBe(1);
});

/**
 * Starring a track on the shuffle page does not redraw the shuffle.
 *
 * A mutation invalidates everything it could have made wrong, because a song
 * appears in albums, genres, search, history, favourites, playlists and the
 * queue. The draw is the exception: it is the one question deliberately kept
 * out of the cache, so invalidating it asks the server for a *different*
 * sample — and the list goes out from under the hand that starred it, taking
 * the starred track with it.
 */
test("keeps the shuffle still when a track in it is favourited", async ({
  page,
}) => {
  let drawn = 0;
  await page.route("**/api/v2/songs/random*", async (route) => {
    drawn += 1;
    await route.fulfill({ json: [track] });
  });
  await page.route("**/api/v2/favorites/track/*", async (route) => {
    await route.fulfill({ status: 204, body: "" });
  });

  await page.goto("/random");
  await expect(page.getByText("Army of Me")).toBeVisible();
  expect(drawn).toBe(1);

  await page
    .getByRole("button", { name: "Add favourite: Army of Me" })
    .click();
  // The button renames itself once the star is on, so the state is read from
  // the label rather than from a locator that no longer matches.
  await expect(
    page.getByRole("button", { name: "Remove favourite: Army of Me" }),
  ).toBeVisible();

  // The same track, still there, and the server was not asked to draw again.
  await expect(page.getByText("Army of Me")).toBeVisible();
  expect(drawn).toBe(1);
});

/**
 * A loop already on screen survives a failed re-mint.
 *
 * A query keeps the answer it has when a later one fails, so `data` and `error`
 * can both be set at once — and saving a correction makes that reachable, since
 * it invalidates everything and this ticket is re-minted. Reading the error
 * first replaced a preview that still played, for a credential good for an
 * hour.
 */
test("keeps a canvas on screen when re-minting its ticket fails", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");
  canvases.set("song-9", [1, 2, 3]);

  await page.goto("/tracks/song-9/edit");
  const preview = page.locator("video.canvas-preview");
  await expect(preview).toBeVisible();

  // From here the ticket route is down, while the loop already on screen is
  // still playable.
  await page.route("**/api/v2/tracks/song-9/canvas-ticket", async (route) => {
    await route.fulfill({ status: 503, body: "" });
  });

  const title = page.getByLabel("Title", { exact: true });
  await title.fill("A title this track did not have");
  // Armed before the click, and awaited after it: the assertions below are
  // about what the panel shows once the re-mint has actually failed, and
  // waiting only on "Saved." would leave that to the order two promises happen
  // to settle in.
  const refused = page.waitForResponse(
    (response) =>
      response.url().endsWith("/canvas-ticket") && response.status() === 503,
  );
  await page.getByRole("button", { name: "Save" }).click();
  await refused;
  await expect(page.getByText("Saved.")).toBeVisible();

  await expect(preview).toBeVisible();
  await expect(
    page.getByText("Whether this track has a canvas could not be read."),
  ).toHaveCount(0);
});

/**
 * A correction reaches the listings that were holding the old value.
 *
 * Correcting a track can change its title, its artists, its genres, its year
 * and its numbering — so it can change what an album holds, how a listing
 * orders, which genre a track belongs to and what a search matches. While every
 * screen re-read on mount the question did not arise; against a cache, a
 * listing sits on the old answer for as long as it is held.
 *
 * **Reached by clicking throughout.** A `goto` to the editor would load a fresh
 * document, and a fresh document has an empty cache — the listing would be
 * re-asked whether or not anything was invalidated, and the test would be
 * measuring the reload. That is what defeated the first attempt at this.
 *
 * Asserted on the request, because the mock answers a fixture that does not
 * change: what marks the difference is that the question was asked again.
 */
test("re-asks a held listing after a correction is saved", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "about the sidebar");

  let asked = 0;
  await page.route("**/api/v2/albums*", async (route) => {
    asked += 1;
    await route.fulfill({ json: albums });
  });
  // The album holds the correctable track, so its editor is a link away rather
  // than an address away.
  await page.route("**/api/v2/albums/album-1", async (route) => {
    await route.fulfill({
      json: { ...albums[0], songs: [correctable.song] },
    });
  });

  await page.goto("/");
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(1);

  const sidebar = page.getByRole("navigation");
  await page.getByRole("link", { name: /Post/ }).first().click();
  await page
    .getByRole("link", { name: "Correct tags: Army Of Me" })
    .click();

  const title = page.getByLabel("Title", { exact: true });
  await expect(title).toHaveValue("Army Of Me");
  await title.fill("Army of Me");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Saved.")).toBeVisible();

  await sidebar.getByRole("link", { name: "Albums" }).click();
  await expect(page.getByText("Post", { exact: true })).toBeVisible();
  expect(asked).toBe(2);
});

/**
 * The icon is declared, and the declared icon is really there.
 *
 * Covered here rather than in the Rust suite, which runs against whatever
 * `build.rs` staged: a real client build when one exists, and a placeholder
 * holding nothing but `index.html` otherwise. The Rust CI never builds the
 * client, so asserting this there would pass on a developer's machine and fail
 * in CI for a reason that has nothing to do with the code. This project's
 * preview server holds the actual build.
 *
 * What the server does with `/favicon.ico` — a 404 rather than the shell, which
 * is what stopped a browser asking for it once a second — belongs to the
 * fallback and is pinned in `tests/service.rs`.
 */
test("declares an icon, and serves the one it declares", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");

  await page.goto("/");
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute(
    "href",
    "/favicon.svg",
  );

  const icon = await page.request.get("/favicon.svg");
  expect(icon.status()).toBe(200);
  expect(icon.headers()["content-type"]).toContain("image/svg+xml");
});

/**
 * The first-load screen is in the document the server sends, and it carries
 * the one vector file.
 *
 * Structural only: nothing here times anything. The floor and the fade are
 * pinned in `src/lib/splash.test.ts` against fake timers, because an assertion on
 * when the screen left would be an assertion about how fast the runner parsed
 * 276 kB of React.
 *
 * What this adds is the half a unit test cannot see: that the markup ships in
 * `index.html` rather than in the bundle — a splash inside the bundle could
 * only appear after the wait it exists to dress — and that it points at
 * `/logo.svg`, the same file the sidebar uses. A fourth hand-drawn copy of the
 * mark is the defect this screen's own issue was filed about.
 */
test("ships the first-load screen in the document, drawn from the one logo", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");

  // Read off the wire, before any script has run. Asking the rendered page
  // would be asking after the screen has already been taken away.
  const document_ = await page.request.get("/");
  const html = await document_.text();
  expect(html).toContain('id="splash"');
  expect(html).toContain('src="/logo.svg"');

  const logo = await page.request.get("/logo.svg");
  expect(logo.status()).toBe(200);
  expect(logo.headers()["content-type"]).toContain("image/svg+xml");
  // The desktop profile, so a divergence in the shared file fails here too:
  // five bars, symmetric, and a gradient rather than a flat fill.
  const svg = await logo.text();
  expect(svg.match(/<rect/g)).toHaveLength(5);
  expect(svg).toContain("linearGradient");

  await page.goto("/");
  // Gone from the document once the application is up — removed rather than
  // left transparent, because an element over the whole viewport still takes
  // a click at zero opacity.
  await expect(page.locator("#splash")).toHaveCount(0);
  await expect(page.locator("#main-content")).toBeVisible();

  // And on an address no route claims. This is the reason the dismissal sits
  // in the root route component rather than in the sign-in and shell
  // components: `notFoundComponent` belongs to the root and renders inside
  // neither of those, so a first visit to an unknown address would have kept
  // the mark until the ten-second ceiling.
  await page.goto("/no-such-screen");
  await expect(page.locator("#splash")).toHaveCount(0);
});

/**
 * The first-load screen leaves even when the client never arrives.
 *
 * `dismissSplash` runs from the bundle, so it cannot be what rescues a bundle
 * that fails to load: a reader would be left with a pulsing mark over the
 * whole viewport for as long as the tab stayed open — worse than a blank
 * page, which at least looks broken rather than looking like it is working.
 *
 * Eleven seconds of patience here, against a ten-second ceiling. It is the
 * slowest test in this file and it earns it: an untested fallback for a
 * failure nobody reproduces is the kind that turns out not to work on the one
 * day it matters.
 */
test("takes the first-load screen away even when the bundle never loads", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "one viewport is enough");

  // Every script the document asks for is refused, so React never mounts and
  // nothing in the bundle can clean up after it.
  await page.route("**/*.js", (route) => route.abort());
  await page.route("**/src/main.tsx", (route) => route.abort());

  await page.goto("/");
  // Up, and covering the page: this is the state the ceiling exists for.
  await expect(page.locator("#splash")).toBeVisible();

  // While it is up is the only moment the leaving state can be read on the
  // real element. Marked by hand rather than waited for: what is checked is
  // the rule, not the clock. Without `pointer-events: none` the layer keeps
  // taking clicks for the length of the fade — and for all of it under
  // `prefers-reduced-motion`, where it is already invisible.
  await page.locator("#splash").evaluate((element: HTMLElement) => {
    element.dataset.leaving = "true";
  });
  await expect(page.locator("#splash")).toHaveCSS("pointer-events", "none");
  await page.locator("#splash").evaluate((element: HTMLElement) => {
    delete element.dataset.leaving;
  });

  await expect(page.locator("#splash")).toHaveCount(0, { timeout: 11_000 });
  // What is revealed is an empty root — the page as it was before this screen
  // existed. The ceiling restores the old failure rather than inventing one.
  await expect(page.locator("#root")).toBeEmpty();
});

/**
 * The playing screen says what the file is.
 *
 * The facts, not the meter: the meter is five lit bars and `aria-hidden`,
 * because the band it summarises is printed in words right beside it and a
 * reader should hear that once rather than twice.
 *
 * Asserted on the rendered line rather than on the mock that fed it. The mock
 * carries these fields because `Song` declares them, so it agrees with the
 * declaration by construction — exactly the loop the lyrics test above was
 * written against, where a `snake_case` envelope the server never sent was
 * confirmed by a mock built from it. What the server really answers is
 * pinned in `tests/native_api.rs`, against the bytes of a WAV.
 */
test("says what the file is on the playing screen", async ({ page }) => {
  await page.goto("/playing");

  const facts = page.locator(".file-facts");
  await expect(facts).toBeVisible();
  // Read as one string: the order is part of what makes it legible at a
  // glance, and six separate assertions would pass on a line that listed
  // them backwards.
  await expect(facts).toHaveText(
    "Lossless · FLAC · 957 kbps · 44.1 kHz · 16 bit · 2 ch · 28 MB",
  );

  // Four of five steps lit: lossless, and not the hi-res band above it.
  await expect(page.locator(".quality-meter span.on")).toHaveCount(4);
  // And hidden from assistive technology, so the band is announced once.
  await expect(page.locator(".quality-meter")).toHaveAttribute(
    "aria-hidden",
    "true",
  );
});

/**
 * The clients holding a credential on this account, which nothing showed
 * before.
 *
 * The material existed and was reachable only by the instance administrator,
 * buried in a disclosure on a screen an ordinary account cannot open. So a
 * listener could not find out which devices were reading their library, let
 * alone stop one.
 *
 * A revoked credential is left out rather than struck through: this list
 * answers who **can** read the library, and one that cannot is not an answer
 * to that question. The administration screen keeps the full history.
 */
test("shows the clients that can read the library, and revokes one", async ({
  page,
}) => {
  await page.goto("/settings");

  // Located by its heading rather than by its position. `.last()` named this
  // panel until a second self-scoped panel was added below it, and a locator
  // that depends on the order of the page silently follows whatever lands at
  // the end.
  const clients = page
    .locator(".settings-panel")
    .filter({ has: page.getByRole("heading", { name: "Registered clients" }) });
  await expect(
    clients.getByRole("heading", { name: "Registered clients" }),
  ).toBeVisible();
  // One of the three: the second is revoked and the third is past its
  // expiry, and neither can read the library any more.
  await expect(clients.getByRole("listitem")).toHaveCount(1);
  await expect(clients.getByText("the kitchen speaker")).toBeVisible();
  await expect(clients.getByText("a laptop that is gone")).toHaveCount(0);
  await expect(
    clients.getByText("a tablet whose year ran out"),
  ).toHaveCount(0);

  // Registering one shows the secret, once, and the list follows.
  await clients.getByLabel("What it is for").fill("my phone");
  await clients.getByRole("button", { name: "Register" }).click();
  await expect(
    page.getByText("wfapi_shown_once_and_never_again"),
  ).toBeVisible();
  await expect(clients.getByRole("listitem")).toHaveCount(2);

  // And revoking stops one. The row goes, because the row is the answer to
  // "who can read my library".
  await clients
    .getByRole("button", { name: "Revoke: the kitchen speaker" })
    .click();
  await expect(clients.getByText("the kitchen speaker")).toHaveCount(0);
  await expect(clients.getByRole("listitem")).toHaveCount(1);

  // **And a second press while the registration is out sends nothing.** Two
  // registrations mint two credentials and only one secret reaches the
  // screen: the other is a credential on this library whose secret nobody
  // holds. This panel shipped without the guard; the one below it has the
  // same one now, for the same reason.
  let release = () => {};
  clientGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  clientWrites = [];
  const register = clients.getByRole("button", { name: "Register" });
  await clients.getByLabel("What it is for").fill("a slow tablet");
  await register.click();
  await expect.soft(register).toBeDisabled();
  await register.click({ force: true });
  await register.click({ force: true });
  release();
  await expect(register).toBeEnabled();
  expect(clientWrites).toEqual(["POST"]);
});

/**
 * The account changes its own Subsonic password, and switches it off.
 *
 * That password is typed into third-party clients, on phones, which is the
 * ordinary way an application password leaks. Until this panel its owner could
 * do neither — both server routes were administrative — so answering "my key
 * has leaked" meant finding an operator.
 */
test("changes the Subsonic password, then switches it off", async ({
  page,
}) => {
  await page.goto("/settings");

  const subsonic = page
    .locator(".settings-panel")
    .filter({ has: page.getByRole("heading", { name: "Subsonic password" }) });
  await expect(subsonic).toBeVisible();
  // Set and never changed, so one date and not two.
  await expect(subsonic.getByText(/^set /)).toBeVisible();
  await expect(subsonic.getByText(/last changed/)).toHaveCount(0);

  // **The key moves with the password**, and it is shown once. A leaked
  // `wfsk_` key authenticates on its own, so a rotation that kept it would
  // answer nothing.
  await subsonic.getByLabel("New password").fill("a long enough replacement");
  await subsonic.getByRole("button", { name: "Change it" }).click();
  await expect(page.getByText("wfsk_shown_once_only")).toBeVisible();
  await expect(subsonic.getByText(/last changed/)).toBeVisible();

  // **Asked twice.** The account cannot put this credential back without an
  // operator, so the first press only arms the second. `exact` because
  // "Yes, switch it off" contains "Switch it off", and the default substring
  // match would make every locator here ambiguous.
  const off = subsonic.getByRole("button", {
    name: "Switch it off",
    exact: true,
  });
  const reallyOff = subsonic.getByRole("button", {
    name: "Yes, switch it off",
  });
  await off.click();
  await expect(off).toHaveCount(0);
  // Backing out leaves the credential exactly as it was.
  await subsonic.getByRole("button", { name: "Cancel" }).click();
  await expect(off).toBeVisible();
  await expect(reallyOff).toHaveCount(0);
  await expect(subsonic.getByText(/^set /)).toBeVisible();

  // Switching it off leaves the panel saying the account holds none — and
  // offering nothing, because creating one opens the facade to an account and
  // that is the operator's decision.
  await off.click();
  await reallyOff.click();
  await expect(
    subsonic.getByText(
      "You have no Subsonic password, so no Subsonic client can read your library. Ask the administrator for one.",
    ),
  ).toBeVisible();
  await expect(subsonic.getByLabel("New password")).toHaveCount(0);
  await expect(
    subsonic.getByRole("button", { name: "Change it" }),
  ).toHaveCount(0);
});

/**
 * A short password is refused before it is sent.
 *
 * The server answers 422 under twelve characters, but that arrives after the
 * person has committed to the password they chose — and the panel would have
 * nothing to say about which of the two failures it was.
 */
test("refuses a Subsonic password under twelve characters", async ({
  page,
}) => {
  await page.goto("/settings");

  const subsonic = page
    .locator(".settings-panel")
    .filter({ has: page.getByRole("heading", { name: "Subsonic password" }) });
  await subsonic.getByLabel("New password").fill("abcdefghijk");
  await subsonic.getByRole("button", { name: "Change it" }).click();
  await expect(subsonic.getByText("Use at least 12 characters.")).toBeVisible();
  // Nothing was sent: the credential still reads as never changed.
  await expect(subsonic.getByText(/last changed/)).toHaveCount(0);
  await expect(page.getByText("wfsk_shown_once_only")).toHaveCount(0);
});

/**
 * A second press while the rotation is out sends nothing.
 *
 * **This one is about a secret, not about tidiness.** Each rotation mints a key
 * and stores its hash, so two in flight leave the second one's hash stored
 * while the panel may have settled on the first one's key — a `wfsk_` key
 * already dead, shown for a credential whose whole point is that it is shown
 * once. The person copies it into their clients and the working key is never
 * shown again.
 */
test("sends one rotation however many times the button is pressed", async ({
  page,
}) => {
  let release = () => {};
  subsonicGate = new Promise<void>((resolve) => {
    release = resolve;
  });

  await page.goto("/settings");
  const subsonic = page
    .locator(".settings-panel")
    .filter({ has: page.getByRole("heading", { name: "Subsonic password" }) });
  const change = subsonic.getByRole("button", { name: "Change it" });

  await subsonic.getByLabel("New password").fill("a long enough replacement");
  await change.click();
  // **Soft**, which is the only `expect.soft` in this file and earns it: the
  // disabled state is the mechanism and the request count below is the
  // consequence, and a hard failure here would stop the test before the
  // consequence was ever measured. Removing the guard has to show both.
  await expect.soft(change).toBeDisabled();
  await change.click({ force: true });
  await change.click({ force: true });

  release();
  await expect(page.getByText("wfsk_shown_once_only")).toBeVisible();
  await expect(change).toBeEnabled();
  expect(subsonicWrites).toEqual(["PUT"]);
});

/**
 * The words as the subject of the screen, and what timings buy.
 *
 * A sheet that carries them gets two things the strip cannot give: the sung
 * line is held in view, and every line is a button that seeks to it. A sheet
 * without them gets neither — and says so by having no buttons at all, rather
 * than promising a jump it cannot compute.
 *
 * No audio is served in these runs, so the head stays at zero and the first
 * line is the sung one. That is enough to tell a marked line from an unmarked
 * one; what it deliberately does not claim is that pressing a line moved the
 * head, which without a stream would be a measurement of nothing.
 */
test("gives the words a view of their own, with timings where there are any", async ({
  page,
}) => {
  lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: true,
      line: [
        { start: 0, value: "the lamps come on along the quay" },
        { start: 5_000, value: "and the water keeps the light" },
        { start: 12_500, value: "the lamps come on along the quay" },
        // An instrumental break: timed, and with no words to name it by.
        { start: 20_000, value: "" },
      ],
    },
  ]);

  await page.goto("/playing");
  await page.getByRole("link", { name: "Lyrics", exact: true }).click();

  const view = page.locator(".lyrics-view");
  await expect(view).toBeVisible();
  await expect(page).toHaveURL(/\?view=lyrics$/);
  // The strip is gone, not duplicated: one view at a time, or the words would
  // be on the screen twice and the sung line marked in two places.
  await expect(page.locator(".playing")).toHaveCount(0);

  // The head is at zero, so the first line is the one being sung — and a
  // refrain repeated word for word does not take the mark with it.
  const lines = view.getByRole("listitem");
  await expect(lines).toHaveCount(4);
  await expect(lines.nth(0)).toHaveAttribute("aria-current", "true");
  await expect(lines.nth(2)).not.toHaveAttribute("aria-current", "true");

  // **Every line is a button, named by its words and then the time.** The
  // words because the name must contain what is on screen — voice control
  // finds a control by what it shows — and the time because the words alone
  // say nothing about what pressing does. A blank line has no words to
  // repeat, so it keeps the time alone rather than a name that opens on a
  // comma.
  await expect(view.getByRole("button")).toHaveCount(4);
  await expect(
    view.getByRole("button", {
      name: "and the water keeps the light, jump to 0:05",
      exact: true,
    }),
  ).toBeVisible();
  // Visible, not merely present: a blank line held a collapsible space, so its
  // button had no height — reachable by the keyboard and drawn nowhere.
  await expect(
    view.getByRole("button", { name: "Jump to 0:20", exact: true }),
  ).toBeVisible();
  await expect(
    view.getByText("This sheet carries timings — press a line to jump to it."),
  ).toBeVisible();

  await expectNoWcagViolations(page);

  // **The switcher says where it is, once.** All three entries point at one
  // path, and the router counts a link carrying no search as matching every
  // search — so this read `['Detail', 'Lyrics']` before `activeOptions` was
  // made exact, and the screen claimed to be in two views at once. A count
  // rather than one attribute: the defect was an extra mark, which an
  // assertion on the right one passes straight through.
  const current = page.locator('.view-switcher a[aria-current="page"]');
  await expect(current).toHaveText("Lyrics");
  await page.goto("/playing");
  await expect(page.locator(".playing")).toBeVisible();
  await expect(current).toHaveText("Detail");

  // And a mode this build does not know is not a mode: the detail view
  // stands, rather than a blank screen or a crash.
  await page.goto("/playing?view=karaoke");
  await expect(page.locator(".playing")).toBeVisible();
  await expect(page.locator(".lyrics-view")).toHaveCount(0);
});

/**
 * A sheet that arrives after the head has already moved is still centred.
 *
 * The case a review asked about: open the lyrics view mid-track with nothing
 * cached, so the column mounts only once the sheet lands. The claim was that
 * the scroll effect would not re-run, because it watches `active` and not the
 * three-state reading beside it. It does run — `active` is derived from the
 * sheet, so it is `-1` until one arrives and cannot already name a line — and
 * this is that argument turned into a measurement rather than left as one.
 */
test("centres a sheet that lands after the head has moved", async ({
  page,
}) => {
  lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: true,
      line: Array.from({ length: 40 }, (_, index) => ({
        start: index * 5_000,
        value: `late line ${index}`,
      })),
    },
  ]);
  let release = () => {};
  lyricsGate = new Promise<void>((resolve) => {
    release = resolve;
  });

  try {
    await page.goto("/playing?view=lyrics");
    // Waiting, not empty: the view's frame is there and the column is not.
    await expect(page.locator(".lyrics-view")).toContainText("Loading");
    await expect(page.locator(".lyrics-view ol")).toHaveCount(0);

    // The head moves while there is no sheet to mark, so `active` stays -1
    // and the effect has nothing to do.
    for (let press = 0; press < 20; press += 1) {
      await page.keyboard.press("ArrowRight");
    }
  } finally {
    // **Released whatever happened.** An assertion failing above would leave
    // the route waiting on this promise for ever, and the context teardown
    // waiting on the route — so the test would not fail, it would hang. The
    // reset in `beforeEach` guards the tests after this one, not this one.
    //
    // The two gates added with the credential panels, and the canvas one
    // above them, carry the same hazard and are left as they are: wrapping
    // them means restructuring three tests this branch has no other reason to
    // touch.
    release();
  }
  const column = page.locator(".lyrics-view ol");
  await expect(column).toBeVisible();
  await expect(
    page.getByRole("listitem").filter({ hasText: "late line 20" }),
  ).toHaveAttribute("aria-current", "true");
  // The line the head is on is in view, which is the whole claim.
  await expect.poll(() => column.evaluate((list) => list.scrollTop)).toBeGreaterThan(0);
});

/**
 * A sheet without timings offers no seeking, and does not pretend to follow.
 *
 * The same view, the other half of the rule: `synced` is the server's statement
 * that the lines carry starts, and everything this view adds over the strip
 * hangs off it.
 */
test("offers no seeking on a sheet that carries no timings", async ({
  page,
}) => {
  lyricSheets.set("song-1", [
    {
      displayArtist: null,
      displayTitle: "Nocturne",
      lang: "eng",
      synced: false,
      line: [
        { value: "the lamps come on along the quay" },
        { value: "and the water keeps the light" },
      ],
    },
  ]);

  await page.goto("/playing?view=lyrics");

  const view = page.locator(".lyrics-view");
  await expect(view.getByRole("listitem")).toHaveCount(2);
  await expect(view.getByRole("button")).toHaveCount(0);
  await expect(
    view.getByText("This sheet carries timings — press a line to jump to it."),
  ).toHaveCount(0);
  // Nothing is marked: an unsynced sheet has no start to compare against, and
  // weighting a line would be a claim about where the track is.
  await expect(view.locator("[aria-current]")).toHaveCount(0);
});

/**
 * The immersive view, and the two things that make it a mode rather than a trap.
 *
 * It **carries transport controls**, because it covers the player bar: a view
 * that fills the screen with a cover and leaves no way to pause is a trap. And
 * it **lives in the location**, so Back leaves the view rather than the screen
 * — which is the press everybody makes on a phone, and the whole reason the
 * mode is a search parameter instead of a `useState`.
 */
test("opens the immersive view, and Back leaves the view and not the screen", async ({
  page,
}) => {
  await page.goto("/playing");
  await page.getByRole("link", { name: "Immersive" }).click();

  const immersive = page.locator("dialog.immersive");
  await expect(immersive).toBeVisible();
  await expect(page).toHaveURL(/\?view=immersive$/);
  await expect(immersive.locator(".immersive-cover")).toBeVisible();
  // The transport, so the mode can be sat in.
  for (const name of ["Previous track", "Play", "Next track"]) {
    await expect(immersive.getByRole("button", { name })).toBeVisible();
  }

  await expectNoWcagViolations(page);

  // **Escape, which is the platform's and not ours.** `<dialog>` answers it,
  // and the navigation hangs off the close rather than off the key, so the
  // button below and this press go through one closer.
  await page.keyboard.press("Escape");
  await expect(immersive).toHaveCount(0);
  await expect(page).toHaveURL(/\/playing$/);
  await expect(page.locator(".playing")).toBeVisible();

  // **And Back does not reopen what Escape just closed.** The close replaces
  // the immersive entry rather than pushing a detail one on top of it.
  // Pushing left `/playing → ?view=immersive → /playing` in the history, so
  // this press landed back on the immersive entry and opened the view again —
  // the opposite of the reason the mode is in the location.
  await page.goBack();
  await expect(page.locator("dialog.immersive")).toHaveCount(0);

  // And Back, from inside the view, leaves the view.
  await page.getByRole("link", { name: "Immersive" }).click();
  await expect(page.locator("dialog.immersive")).toBeVisible();
  await page.goBack();
  await expect(page.locator("dialog.immersive")).toHaveCount(0);
  await expect(page).toHaveURL(/\/playing$/);
  await expect(page.locator(".playing")).toBeVisible();

  // The close button takes the same route out.
  await page.getByRole("link", { name: "Immersive" }).click();
  await page
    .getByRole("button", { name: "Leave the immersive view" })
    .click();
  await expect(page.locator("dialog.immersive")).toHaveCount(0);
  await expect(page).toHaveURL(/\/playing$/);
});

/**
 * The sung line is held in view, and only the column moves to do it.
 *
 * `scrollIntoView` scrolls every scrollable ancestor it has, the window
 * included, so each new sung line dragged the whole page back to the words —
 * and somebody reading the saved positions underneath was hauled upwards every
 * few seconds. The column scrolls itself now.
 *
 * The head is moved with the keyboard rather than by pressing a line, because
 * Playwright scrolls an element into view before clicking it and that scroll
 * would be the one under test. Twelve saved positions are there to make the
 * page taller than the viewport: without something below the fold, "the window
 * did not scroll" is true however the scrolling is done.
 */
test("holds the sung line in view without dragging the page", async ({
  page,
}) => {
  lyricSheets.set("song-1", [
    {
      displayArtist: "Rue Delacour",
      displayTitle: "Nocturne",
      lang: "eng",
      synced: true,
      line: Array.from({ length: 40 }, (_, index) => ({
        start: index * 5_000,
        value: `line number ${index}`,
      })),
    },
  ]);
  bookmarks = Array.from({ length: 12 }, (_, index) => ({
    position_ms: index * 1_000,
    comment: null,
    created_at: 1,
    updated_at: 1,
    song: {
      id: `saved-${index}`,
      title: `Somewhere to pick up ${index}`,
      artist: "Rue Delacour",
    },
  }));

  await page.goto("/playing?view=lyrics");
  const column = page.locator(".lyrics-view ol");
  await expect(column).toBeVisible();
  // The list the screen renders under the track, drawn by a test at last.
  await expect(page.locator(".bookmark-list li")).toHaveCount(12);
  const scrollable = await page.evaluate(
    () => document.documentElement.scrollHeight > window.innerHeight,
  );
  expect(scrollable).toBe(true);

  // **Scrolled down to the saved positions first, which is the whole case.**
  // With the column fully on screen there is nothing for an ancestor scroll to
  // do, so the page stays put however the following is written — the complaint
  // was about a reader who had scrolled away from the words and kept being
  // hauled back to them.
  await page.evaluate(() => window.scrollTo(0, 400));
  const before = await page.evaluate(() => window.scrollY);
  expect(before).toBeGreaterThan(0);

  // Twenty presses of five seconds each: the player listens on the window, and
  // a modal dialog is the only thing that would be in the way.
  for (let press = 0; press < 20; press += 1) {
    await page.keyboard.press("ArrowRight");
  }

  // The head moved, which is also the proof that a line's own press would
  // seek: both go through `player.seek`, and the position is React state
  // rather than something read back off an element that is playing nothing.
  await expect(
    page.getByRole("listitem").filter({ hasText: "line number 20" }),
  ).toHaveAttribute("aria-current", "true");

  // Smooth scrolling settles over frames, so the column is polled rather than
  // read once.
  await expect
    .poll(() => column.evaluate((list) => list.scrollTop))
    .toBeGreaterThan(0);
  // And the page did not move under the reader.
  expect(await page.evaluate(() => window.scrollY)).toBe(before);
});

/**
 * The radio page, which manages a list the catalogue knows nothing about.
 *
 * Worth its own test for one reason: a station is written and then read back,
 * so every control here depends on the list the server holds rather than on
 * what the page remembers. The harness keeps the list for that reason too — a
 * mock answering a constant would let a delete that deletes nothing pass.
 */
test("keeps a list of radio stations, replacing rather than patching", async ({
  page,
}) => {
  await page.goto("/radio");
  await expect(
    page.getByRole("heading", { name: "Radio", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("No stations yet. Add one above."),
  ).toBeVisible();

  await page.getByLabel("Station name").fill("Radio Nova");
  await page.getByLabel("Stream address").fill("https://nova.example.org/live");
  await page.getByLabel("Station page (optional)").fill("https://nova.example.org/");
  await page.getByRole("button", { name: "Add station" }).click();

  const listed = page.locator(".resource-list li");
  await expect(listed).toHaveCount(1);
  await expect(listed.first()).toContainText("Radio Nova");
  await expect(listed.first()).toContainText("https://nova.example.org/live");
  await expect(
    page.getByRole("link", { name: "Station page" }),
  ).toHaveAttribute("href", "https://nova.example.org/");

  // An address the server would refuse is refused here first, with a reason,
  // and the button stays shut rather than sending a request to find out.
  await page.getByLabel("Station name").fill("Elsewhere");
  await page.getByLabel("Stream address").fill("mms://elsewhere.example.org/s");
  await expect(
    page.getByText("An address must start with http:// or https://."),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add station" }),
  ).toBeDisabled();

  await expectNoWcagViolations(page);

  // Editing loads the station into the same form, and says what saving means:
  // the page address is cleared because the form sends every field.
  await page.getByRole("button", { name: "Edit" }).click();
  await expect(page.getByLabel("Station name")).toHaveValue("Radio Nova");
  await expect(page.getByLabel("Station page (optional)")).toHaveValue(
    "https://nova.example.org/",
  );
  await expect(
    page.getByText(
      "Saving replaces every field, so an empty page address clears the stored one.",
    ),
  ).toBeVisible();
  await page.getByLabel("Station name").fill("Radio Nova Classics");
  await page.getByLabel("Station page (optional)").fill("");
  await page.getByRole("button", { name: "Save station" }).click();

  await expect(listed.first()).toContainText("Radio Nova Classics");
  // Cleared, which is the half a patch would have got wrong.
  await expect(page.getByRole("link", { name: "Station page" })).toHaveCount(0);

  await page.getByRole("button", { name: "Delete" }).click();
  await expect(listed).toHaveCount(0);
  await expect(
    page.getByText("No stations yet. Add one above."),
  ).toBeVisible();
});

/**
 * The form is closed for the length of a write, and the station playing is
 * not.
 *
 * **The held request below is what makes this test able to fail.** Every other
 * mock in this file answers instantly, and against an instant answer a guarded
 * form and an unguarded one look identical: the save is over before a second
 * click or a second edit could begin. A request held open until this test lets
 * it go leaves the window open for as long as the assertions need — where a
 * fixed delay would be a race the slowest runner loses (constat de revue,
 * pris; the two tests above it hold their requests the same way).
 */
test("closes the form while a station is saving, and leaves listening alone", async ({
  page,
}) => {
  await page.goto("/radio");
  await page.getByLabel("Station name").fill("Radio Nova");
  await page.getByLabel("Stream address").fill("https://nova.example.org/live");
  await page.getByRole("button", { name: "Add station" }).click();
  await expect(page.locator(".resource-list li")).toHaveCount(1);
  expect(radioWrites).toEqual(["POST"]);

  // Held from here on, so only the second save waits.
  let release = () => {};
  radioGate = new Promise<void>((resolve) => {
    release = resolve;
  });

  await page.getByLabel("Station name").fill("FIP");
  await page.getByLabel("Stream address").fill("https://fip.example.org/live");
  await page.getByRole("button", { name: "Add station" }).click();

  // Shut for the whole round trip: the button cannot be pressed again, and the
  // fields cannot be refilled under a save that is about to clear them.
  //
  // **Holding the request is what lets the first line below distinguish**, and
  // it did not when the mock merely answered slowly: an empty form disables
  // that button too, so the assertion retried until the save landed, cleared
  // the fields and disabled the button for the other reason. Held open, the
  // save cannot land, and removing the guard fails on this very line —
  // measured both ways.
  await expect(page.getByRole("button", { name: "Add station" })).toBeDisabled();
  await expect(page.getByLabel("Station name")).toBeDisabled();
  await expect(page.getByLabel("Stream address")).toBeDisabled();
  // And no edit can begin, which is what made a save clear a form somebody
  // else had just filled.
  await expect(page.getByRole("button", { name: "Edit" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Delete" })).toBeDisabled();
  // Listening is the deliberate exception: it writes nothing, and a reader
  // waiting on a save has no reason to lose the station they are hearing.
  await expect(page.getByRole("button", { name: "Listen" })).toBeEnabled();

  // The second click of an impatient double-click, dispatched for real rather
  // than waited for: `force` skips Playwright's own actionability check, so
  // what refuses it is the browser declining to activate a disabled button.
  // Without the guard this click sends a second POST and the account ends up
  // with two identical stations from one gesture — which is what the count
  // below would then show.
  await page
    .getByRole("button", { name: "Add station" })
    .click({ force: true });

  release();

  await expect(page.locator(".resource-list li")).toHaveCount(2);
  // One gesture, one station. Two POSTs in all, one per click.
  expect(radioWrites).toEqual(["POST", "POST"]);
  // Open again, and empty, once the save has landed.
  await expect(page.getByLabel("Station name")).toBeEnabled();
  await expect(page.getByLabel("Station name")).toHaveValue("");
});

/**
 * The disk tree (RFC-012 decision 1): one folder at a time, the folder in the
 * address, a trail back up, the received folder under its own name, and the
 * file names under each title. Read-only: no control here moves anything.
 */
test("walks a library's folders on disk and back up the trail", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  const folder = (name: string, path: string, direct: number, nested: number) => ({
    name,
    path,
    track_count: direct,
    nested_track_count: nested,
  });
  const listings: Record<string, unknown> = {
    "": {
      path: "",
      folder_total: 2,
      track_total: 0,
      folders: [
        folder(".waveflow-managed", ".waveflow-managed", 1, 1),
        folder("Björk", "Björk", 0, 2),
      ],
      tracks: [],
    },
    Björk: {
      path: "Björk",
      folder_total: 1,
      track_total: 0,
      folders: [folder("Vespertine", "Björk/Vespertine", 2, 2)],
      tracks: [],
    },
    "Björk/Vespertine": {
      path: "Björk/Vespertine",
      folder_total: 0,
      track_total: 2,
      folders: [],
      tracks: [
        { name: "01 Hidden Place.flac", song: song(1, "Hidden Place", 0, false) },
        { name: "02 Cocoon.flac", song: song(2, "Cocoon", 0, false) },
      ],
    },
    ".waveflow-managed": {
      path: ".waveflow-managed",
      folder_total: 0,
      track_total: 1,
      folders: [],
      tracks: [{ name: "9ad0f9e.flac", song: song(3, "Pagan Poetry", 0, false) }],
    },
  };
  const asked: string[] = [];
  await page.route("**/api/v2/libraries/library-1/tree?*", (route) => {
    const path = new URL(route.request().url()).searchParams.get("path") ?? "";
    asked.push(path);
    const listing = listings[path];
    return listing
      ? route.fulfill({ json: listing })
      : route.fulfill({ status: 404, json: { error: "not_found" } });
  });

  await page.goto("/folders");
  await expect(page.getByRole("heading", { name: "Folders" })).toBeVisible();
  await expect(page.getByText("Nothing here moves a file.", { exact: false })).toBeVisible();
  const folders = page.getByRole("list", { name: "Folders" });
  // The received folder by its label, and an artist folder by what is beneath.
  await expect(folders.getByRole("link", { name: /Received/ })).toBeVisible();
  await expect(folders.getByRole("link", { name: /Björk/ })).toContainText("2 tracks");
  await expect(page.getByRole("link", { name: "Upload music" })).toHaveAttribute(
    "href",
    "/upload",
  );

  await folders.getByRole("link", { name: /Björk/ }).click();
  await expect(page).toHaveURL(/path=Bj%C3%B6rk$/);
  await page.getByRole("list", { name: "Folders" }).getByRole("link", { name: /Vespertine/ }).click();
  await expect(page.getByRole("heading", { name: "Vespertine" })).toBeVisible();
  const trail = page.getByRole("navigation", { name: "Folder path" });
  await expect(trail.getByRole("link")).toHaveText(["Ma musique", "Björk", "Vespertine"]);
  await expect(trail.getByRole("link", { name: "Vespertine" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  // The file's name stands under each title.
  const rows = page.locator(".songs tbody tr");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("Hidden Place");
  await expect(rows.first()).toContainText("01 Hidden Place.flac");
  await expectNoWcagViolations(page);

  // Back up the trail to the root, then into the received files.
  await trail.getByRole("link", { name: "Ma musique" }).click();
  await expect(page.getByRole("heading", { name: "Folders" })).toBeVisible();
  await page.getByRole("list", { name: "Folders" }).getByRole("link", { name: /Received/ }).click();
  await expect(page.getByRole("heading", { name: "Received" })).toBeVisible();
  await expect(page.locator(".songs tbody tr")).toContainText("9ad0f9e.flac");
  expect([...new Set(asked)]).toEqual(["", "Björk", "Björk/Vespertine", ".waveflow-managed"]);
});

/** A listener walks the same tree and is offered no upload. */
test("offers a listener the folders without the upload", async ({ page }) => {
  libraries = [
    { ...library("library-1", "Ma musique"), role: "listener", accepts_uploads: true },
  ];
  await page.route("**/api/v2/libraries/library-1/tree?*", (route) =>
    route.fulfill({
      json: {
        path: "",
        folder_total: 1,
        track_total: 0,
        folders: [{ name: "Björk", path: "Björk", track_count: 0, nested_track_count: 2 }],
        tracks: [],
      },
    }),
  );
  await page.goto("/folders");
  await expect(
    page.getByRole("list", { name: "Folders" }).getByRole("link", { name: /Björk/ }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: "Upload music" })).toHaveCount(0);
});

/**
 * A stand-in for the filed-folder routes (RFC-012 decision 2), with state: the
 * screen re-reads after every write, and the point is that it shows what the
 * write did. Names collide case-insensitively, as on the server.
 */
async function serveFiledFolders(page: Page, songs: ReturnType<typeof song>[]) {
  const state = {
    folders: [] as Array<{ id: string; parent_id: string | null; name: string }>,
    placed: new Map<string, string>(),
    writes: [] as Array<{ method: string; path: string; body: unknown }>,
  };
  const tree = () => ({
    folders: state.folders.map((folder) => ({
      ...folder,
      track_count: [...state.placed.values()].filter((id) => id === folder.id)
        .length,
    })),
    unfiled_track_count: songs.filter((entry) => !state.placed.has(entry.id))
      .length,
  });
  await page.route("**/api/v2/libraries/library-1/folders", async (route) => {
    const request = route.request();
    if (request.method() === "POST") {
      const body = request.postDataJSON() as { name: string; parent_id: string | null };
      state.writes.push({ method: "POST", path: "folders", body });
      const name = body.name.trim();
      if (
        state.folders.some(
          (folder) =>
            folder.parent_id === body.parent_id &&
            folder.name.toLowerCase() === name.toLowerCase(),
        )
      ) {
        await route.fulfill({ status: 409, json: { error: "conflict" } });
        return;
      }
      const folder = { id: `folder-${state.folders.length + 1}`, parent_id: body.parent_id, name };
      state.folders.push(folder);
      await route.fulfill({ status: 201, json: { ...folder, track_count: 0 } });
      return;
    }
    await route.fulfill({ json: tree() });
  });
  await page.route("**/api/v2/libraries/library-1/folders/*/tracks?*", (route) => {
    const folder = new URL(route.request().url()).pathname.split("/").at(-2);
    return route.fulfill({
      json: songs.filter((entry) =>
        folder === "unfiled"
          ? !state.placed.has(entry.id)
          : state.placed.get(entry.id) === folder,
      ),
    });
  });
  await page.route("**/api/v2/libraries/library-1/placements", async (route) => {
    const body = route.request().postDataJSON() as {
      track_ids: string[];
      folder_id: string | null;
    };
    state.writes.push({ method: "PUT", path: "placements", body });
    for (const id of body.track_ids) {
      if (body.folder_id) state.placed.set(id, body.folder_id);
      else state.placed.delete(id);
    }
    await route.fulfill({ json: { placed: body.track_ids.length } });
  });
  await page.route("**/api/v2/folders/*", async (route) => {
    const request = route.request();
    const id = new URL(request.url()).pathname.split("/").at(-1) ?? "";
    if (request.method() === "DELETE") {
      state.writes.push({ method: "DELETE", path: id, body: null });
      const gone = new Set([id]);
      for (const folder of state.folders) {
        if (folder.parent_id && gone.has(folder.parent_id)) gone.add(folder.id);
      }
      let unfiled = 0;
      for (const [track, folder] of [...state.placed]) {
        if (gone.has(folder)) {
          state.placed.delete(track);
          unfiled += 1;
        }
      }
      state.folders = state.folders.filter((folder) => !gone.has(folder.id));
      await route.fulfill({ json: { unfiled } });
      return;
    }
    const patch = request.postDataJSON() as { name?: string; parent_id?: string | null };
    state.writes.push({ method: "PATCH", path: id, body: patch });
    state.folders = state.folders.map((folder) =>
      folder.id === id
        ? {
            ...folder,
            name: patch.name ?? folder.name,
            parent_id: patch.parent_id === undefined ? folder.parent_id : patch.parent_id,
          }
        : folder,
    );
    const folder = state.folders.find((entry) => entry.id === id);
    await route.fulfill({ json: { ...folder, track_count: 0 } });
  });
  return state;
}

/**
 * Filing, end to end on the screen: a folder created, a track filed from its
 * menu — the keyboard's way — another dragged onto a folder, the folder
 * renamed and deleted, and its tracks back among the unfiled. No control
 * here claims to touch a file, and none does: every write is to the folder
 * routes.
 */
test("files tracks into folders from the menu and by dragging", async ({
  page,
}, testInfo) => {
  const songs = [
    song(1, "Hidden Place", 0, false),
    song(2, "Cocoon", 0, false),
    song(3, "Pagan Poetry", 0, false),
  ];
  const state = await serveFiledFolders(page, songs);

  await page.goto("/folders?view=filed");
  await expect(page.getByRole("heading", { name: "Filed" })).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "Folder views" }).getByRole("link", {
      name: "Filed",
    }),
  ).toHaveAttribute("aria-current", "page");
  const cards = page.getByRole("list", { name: "Folders" });
  await expect(cards.getByRole("link", { name: /Unfiled/ })).toContainText("3 tracks");

  // A folder, and a second one refused for a name already taken.
  await page.getByRole("button", { name: "New folder" }).click();
  await page.getByLabel("Folder name").fill("Rock");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("status")).toHaveText("“Rock” created.");
  await page.getByRole("button", { name: "New folder" }).click();
  await page.getByLabel("Folder name").fill("ROCK");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(
    page.getByRole("alert").filter({ hasText: "A folder of that name is already here." }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();

  // From the menu: the unfiled tracks, the first one filed into Rock.
  await cards.getByRole("link", { name: /Unfiled/ }).click();
  await expect(page.getByRole("heading", { name: "Unfiled" })).toBeVisible();
  const rows = page.locator(".songs tbody tr");
  await expect(rows).toHaveCount(3);
  await rows.first().locator("button.song-title").focus();
  await page.keyboard.press("Shift+F10");
  await page.getByRole("menuitem", { name: "Move to folder…" }).click();
  const picker = page.getByRole("dialog", { name: "Move “Hidden Place” to a folder" });
  await expect(picker).toBeVisible();
  await expectNoWcagViolations(page);
  await picker.getByRole("button", { name: "Rock" }).click();
  await expect(picker).toBeHidden();
  await expect(rows).toHaveCount(2);
  expect(state.writes.at(-1)).toEqual({
    method: "PUT",
    path: "placements",
    body: { track_ids: ["song-1"], folder_id: "folder-1" },
  });

  // By dragging: a row onto the Rock card beside the unfiled tracks. A drag is
  // a mouse gesture; the phone has the menu above, already proven.
  if (testInfo.project.name === "desktop") {
    await rows
      .filter({ hasText: "Cocoon" })
      .dragTo(page.getByRole("list", { name: "File into" }).getByRole("link", { name: /Rock/ }));
    await expect(page.getByRole("status")).toHaveText("1 track filed in Rock.");
    await expect(rows).toHaveCount(1);
  } else {
    state.placed.set("song-2", "folder-1");
  }

  // Inside the folder: both, renamed, then deleted, and they come back unfiled.
  await page.goto("/folders?view=filed&folder=folder-1");
  await expect(page.getByRole("heading", { name: "Rock" })).toBeVisible();
  await expect(rows).toHaveCount(2);
  await page.getByRole("button", { name: "Rename" }).click();
  await page.getByLabel("New name").fill("Rock & Roll");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("heading", { name: "Rock & Roll" })).toBeVisible();
  page.once("dialog", (dialog) => void dialog.accept());
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByRole("heading", { name: "Filed" })).toBeVisible();
  await expect(page.getByRole("status")).toHaveText(
    "Folder deleted: 2 tracks are unfiled.",
  );
  await expect(cards.getByRole("link", { name: /Unfiled/ })).toContainText("3 tracks");
  await expectNoWcagViolations(page);
});

/** A listener walks the filed tree and is offered nothing that writes. */
test("shows a listener the filed tree with nothing to change", async ({ page }) => {
  libraries = [{ ...library("library-1", "Ma musique"), role: "listener" }];
  const songs = [song(1, "Hidden Place", 0, false)];
  const state = await serveFiledFolders(page, songs);
  state.folders.push({ id: "folder-1", parent_id: null, name: "Rock" });

  await page.goto("/folders?view=filed&folder=unfiled");
  await expect(page.locator(".songs tbody tr")).toHaveCount(1);
  for (const name of ["New folder", "Rename", "Delete", "Move…"]) {
    await expect(page.getByRole("button", { name })).toHaveCount(0);
  }
  // No destinations beside the unfiled tracks, and no drag on the rows.
  await expect(page.getByRole("list", { name: "File into" })).toHaveCount(0);
  await expect(page.locator(".songs tbody tr").first()).not.toHaveAttribute("draggable", "true");
  await page.locator(".songs tbody tr button.song-title").first().focus();
  await page.keyboard.press("Shift+F10");
  await expect(page.getByRole("menuitem", { name: "Properties" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Move to folder…" })).toHaveCount(0);
});

/**
 * Received first, filed second (RFC-012 decision 7): an upload started from a
 * folder commits as any other, then files the track it became.
 */
test("files a received track into the folder the upload started from", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  const state = await serveFiledFolders(page, []);
  state.folders.push({ id: "folder-1", parent_id: null, name: "Rock" });

  await page.goto("/upload?folder=folder-1");
  await expect(page.getByText("Received files go into the folder Rock.")).toBeVisible();
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "Army of Me.flac",
    mimeType: "audio/flac",
    buffer: Buffer.from(Array.from({ length: 20 }, (_, i) => i)),
  });
  await expect(page.getByText("Added to the library")).toBeVisible();
  expect(uploadCommits).toEqual(["upload-1"]);
  expect(state.writes).toEqual([
    {
      method: "PUT",
      path: "placements",
      body: { track_ids: ["song-uploaded"], folder_id: "folder-1" },
    },
  ]);
});

/**
 * Received is not undone by a filing that fails: the commit stands, and the
 * line says the track arrived unfiled rather than claiming it was filed.
 */
test("keeps a received track when filing it fails, and says so", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  const state = await serveFiledFolders(page, []);
  state.folders.push({ id: "folder-1", parent_id: null, name: "Rock" });
  await page.route("**/api/v2/libraries/library-1/placements", (route) =>
    route.fulfill({ status: 422, json: { error: "validation" } }),
  );

  await page.goto("/upload?folder=folder-1");
  await expect(page.getByText("Received files go into the folder Rock.")).toBeVisible();
  await page.getByLabel("Choose audio files").setInputFiles({
    name: "Army of Me.flac",
    mimeType: "audio/flac",
    buffer: Buffer.from(Array.from({ length: 20 }, (_, i) => i)),
  });
  await expect(page.getByText("Added to the library, but not filed")).toBeVisible();
  expect(uploadCommits).toEqual(["upload-1"]);
});

/**
 * No file is taken before the folder is known, and a folder that is gone is
 * said before anything is sent: either way a file would have arrived unfiled
 * under a page that promised a folder.
 */
test("waits for the folder before taking files, and says when it is gone", async ({
  page,
}) => {
  libraries = [{ ...library("library-1", "Ma musique"), accepts_uploads: true }];
  let release = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v2/libraries/library-1/folders", async (route) => {
    await held;
    await route.fulfill({ json: { folders: [], unfiled_track_count: 0 } });
  });
  try {
    await page.goto("/upload?folder=folder-gone");
    await expect(page.getByLabel("Choose audio files")).toBeDisabled();
  } finally {
    release();
  }
  await expect(page.getByLabel("Choose audio files")).toBeEnabled();
  await expect(page.getByRole("status")).toHaveText(
    "The folder this upload was opened for cannot be found: received files will stay unfiled.",
  );
});

/**
 * The root lists folders, not tracks. Coming back to it from the unfiled
 * tracks must not leave those tracks behind in its controls.
 */
test("offers no tracks at the root after visiting the unfiled ones", async ({
  page,
}) => {
  await serveFiledFolders(page, [song(1, "Hidden Place", 0, false)]);
  await page.goto("/folders?view=filed&folder=unfiled");
  await expect(page.locator(".songs tbody tr")).toHaveCount(1);
  const controls = page.locator(".page-controls");
  await expect(controls.getByRole("button", { name: /Play/ })).toBeVisible();
  await page
    .getByRole("navigation", { name: "Folder path" })
    .getByRole("link", { name: "Filed", exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "Filed" })).toBeVisible();
  await expect(controls.getByRole("button", { name: /Play/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Load more" })).toHaveCount(0);
});

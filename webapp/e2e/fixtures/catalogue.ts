// The fixed catalogue the mock answers from, and its answer factories.



export const session = {
  access_token: "e2e-access-token",
  user: { id: "user-1", username: "listener", role: "admin" },
  device_id: "device-1",
};

export const albums = [
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

export const track = {
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
export const history = Array.from({ length: 120 }, (_, index) => ({
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
export const library = (id: string, name: string) => ({
  id,
  name,
  visibility: "private" as const,
  role: "owner" as "owner" | "manager" | "listener",
  last_scan_started_at: null,
  last_scan_completed_at: null,
  accepts_uploads: false as boolean,
  accepts_canvas: false as boolean,
  follow_links: false as boolean,
});

/**
 * The tracks the scans can no longer find (RFC-021), as
 * `GET /libraries/{id}/unavailable-tracks` answers them. Mutable, because the
 * purge test removes them and the point is that the list follows.
 */
export const missingTrack = (index: number, title: string) => ({
  id: `missing-${index}`,
  relative_path: `Gone/${index} ${title}.flac`,
  title,
  artist: "Ada Reed",
  album: "Gone",
  duration_ms: 200_000,
  unavailable_since: 1_700_000_000_000 + index,
  playlist_entries: index,
  favorites: 1,
  plays: 3 * index,
  purged_at: null as number | null,
  deleted_after: null as number | null,
});

/** The grace window the mock server keeps a purge restorable for: a week. */
export const PURGE_GRACE_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * The clients this account has registered, as the self-scoped route answers
 * them. Mutable, because the test registers one and revokes one and the point
 * is that the list follows.
 */
export type Client = {
  id: string;
  name: string;
  scopes: string[];
  expires_at: number | null;
  created_at: number;
  last_used_at: number | null;
  revoked_at: number | null;
};

export function freshClients(): Client[] {
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

export const song = (
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
  // ReplayGain as tagged, different per track so a test can tell which one
  // the player levelled. The album's pair is shared, as on a real record.
  replay_gain_track_gain: -6 - index,
  replay_gain_track_peak: 0.9,
  replay_gain_album_gain: -8,
  replay_gain_album_peak: 0.95,
});

export const albumDetail = {
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
export const artistDetail = {
  id: "artist-1",
  name: "Björk",
  album_count: 2,
  artwork_hash: null,
  albums,
};

/**
 * A dry run's answer: three of five entries placed, two it could not.
 *
 * Two entries short on purpose — the panel's whole reason to exist is showing
 * what an import would lose before it happens, so the report has to carry
 * losses for the test to mean anything.
 */
export const importReport = {
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

/**
 * The hash the server would store a station's image under, made up from its
 * address: what the mock answers instead of fetching anything.
 */
export const imageHash = (address: string) =>
  `station-art-${address.replace(/[^a-z0-9]/gi, "")}`;

export const playlists = [
  {
    id: "playlist-1",
    owner_id: "user-1",
    owner_username: "listener",
    name: "Evening",
    comment: null,
    public: false,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    songs: [song(1, "Hidden Place", 5, true)],
  },
];

export const genres = [
  { name: "Art Pop", song_count: 412, album_count: 31 },
  { name: "Shoegaze", song_count: 233, album_count: 18 },
];

export const genreSongs = [
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
export function correctableTrack() {
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
export const fileArtists = [{ id: "artist-1", name: "Björk" }];

export type Offer = { full_hash: string; size_bytes: number; extension: string };

export const freshUploadSession = () => ({
  session_id: "upload-1",
  next_chunk: 0,
  received_bytes: 0,
  chunk_bytes: 8,
  expires_at: 0,
});

/**
 * External scrobbling — RFC-010.
 *
 * Two instances of one recipient, because that is the case the whole named
 * instances slice exists for and the one a screen keyed on the recipient alone
 * would collapse. Last.fm is declared and unavailable, which is its real shape
 * on a server whose operator registered no application.
 */
export type Recipient = "listenbrainz" | "maloja" | "lastfm";

/**
 * What `lastfm_unavailability` answers — a case, not a sentence.
 *
 * It used to be the server's own English prose, copied here and asserted
 * verbatim in `scrobbling.spec.ts`, so the test agreed with the server about a
 * string neither of them should have been sending to a client that ships in
 * two languages.
 */
const NO_PUBLIC_URL = "browser_journey_needs_https";

export const freshDestinations = () => [
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
export const linkState = (provider: Recipient, destination: string) => ({
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
export const freshUncertain = () => [
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

/**
 * `GET /api/v2/stats` for one period, shaped as the server shapes it: a day
 * per bucket for the two short periods, a month for the others, and the
 * busiest bucket last so the bars have something to measure against.
 */
export function statsFor(period: string) {
  const buckets = period === "7d" ? 7 : period === "30d" ? 30 : 12;
  const daily = period === "7d" || period === "30d";
  const timeline = Array.from({ length: buckets }, (_, index) => ({
    label: daily
      ? `2026-10-${String(index + 1).padStart(2, "0")}`
      : `2026-${String(index + 1).padStart(2, "0")}`,
    plays: index === buckets - 1 ? 4 : index % 3 === 0 ? 1 : 0,
    listening_ms: 0,
  }));
  const plays = timeline.reduce((sum, bucket) => sum + bucket.plays, 0);
  return {
    period,
    from: period === "all" ? null : 1_790_000_000_000,
    bucket: daily ? "day" : "month",
    totals: {
      plays,
      listening_ms: plays * 234_000,
      tracks: 2,
      albums: 1,
      artists: 1,
    },
    top_tracks: [
      { song: song(1, "Hidden Place", 0, false), plays: 3 },
      { song: song(2, "Cocoon", 0, false), plays: 1 },
    ],
    top_albums: [{ album: albums[1], plays: 4 }],
    top_artists: [
      { artist: { id: "artist-1", name: "Björk" }, artwork_hash: null, plays: 4 },
    ],
    top_genres: [{ name: "Art Pop", plays: 4 }],
    timeline,
    first_play: { song: song(2, "Cocoon", 0, false), played_at: 1_790_000_000_000 },
    last_play: { song: song(1, "Hidden Place", 0, false), played_at: 1_790_100_000_000 },
  };
}

// ── RFC-024: mixes, statistics and lyrics from LRCLIB ──

export const dailyMixes = () => [
  {
    id: "most-played",
    kind: "most_played",
    genre: null,
    decade: null,
    track_count: 2,
    artwork_hashes: [],
  },
  {
    id: "genre:art pop",
    kind: "genre",
    genre: "Art Pop",
    decade: null,
    track_count: 2,
    artwork_hashes: [],
  },
  {
    id: "decade:1990",
    kind: "decade",
    genre: null,
    decade: 1990,
    track_count: 2,
    artwork_hashes: [],
  },
];

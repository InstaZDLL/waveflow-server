import { call, collect, scoped } from "./client";

export type Album = {
  id: string;
  library_id: string;
  title: string;
  artist: string | null;
  artist_id: string | null;
  artwork_hash: string | null;
  year: number | null;
  starred_at: number | null;
  user_rating: number | null;
};

export type Song = {
  id: string;
  library_id: string;
  album_id: string | null;
  title: string;
  album: string | null;
  artist: string | null;
  artist_id: string | null;
  artwork_hash: string | null;
  duration_ms: number;
  track: number | null;
  disc: number | null;
  starred_at: number | null;
  user_rating: number | null;
  /**
   * What the file is, as the scan read it.
   *
   * Every one of these has been on the wire since `SongItem` gained it and
   * was simply not declared here. `the_technical_fields_reach_the_wire_and_match_the_file`
   * asserts them against the bytes of the file itself, which is the only
   * side that can: these declarations are claims about the server, and the
   * mocks in this client are built from them, so a mock can never contradict
   * one.
   *
   * Optional where a format has nothing to report — `bit_depth` on a lossy
   * codec, `bitrate` on a container the reader could not measure — rather
   * than zero, which would be a measurement.
   */
  codec: string | null;
  suffix: string;
  size: number;
  bitrate: number | null;
  sample_rate: number | null;
  channels: number | null;
  bit_depth: number | null;
};

export type AlbumDetail = Album & { songs: Song[] };

export type Artist = {
  id: string;
  name: string;
  album_count: number;
  artwork_hash: string | null;
};

/** Where one enriched field came from — RFC-011 decision 11. */
export type EnrichmentSource = {
  /** As `WAVEFLOW_METADATA_PROVIDERS` spells it: `lastfm`, `deezer`. */
  provider: string;
  /** When the provider gave this answer, in epoch milliseconds. */
  fetched_at: number;
};

/**
 * What the server learned about an artist from outside the library.
 *
 * `pending` means a provider has not answered yet, `none` that every one
 * answered with nothing, `disabled` that none is configured. The page asks
 * again while `pending`, a bounded number of times.
 */
export type ArtistEnrichment = {
  state: "disabled" | "pending" | "ready" | "none";
  bio: {
    summary: string | null;
    full: string | null;
    source: EnrichmentSource;
  } | null;
  image: { artwork_hash: string; source: EnrichmentSource } | null;
  similar: { artists: Artist[]; source: EnrichmentSource } | null;
};

export type ArtistDetail = Artist & {
  albums: Album[];
  /** Always present on the native page; optional so an older server parses. */
  enrichment?: ArtistEnrichment;
};

export type SearchResult = {
  artists: Artist[];
  albums: Album[];
  songs: Song[];
};

/**
 * The orders `GET /api/v2/albums` accepts, spelled as the server does — the
 * vocabulary is `AlbumOrder` in `src/services/mod.rs`, shared with the Subsonic
 * `type` parameter.
 *
 * Four of these also *filter*: `frequent`, `recent` and `starred` answer only
 * the albums that have a play count, a last play or a star, and `byYear` only
 * those carrying a year. The header count is read off the response for that
 * reason, rather than assumed to be the size of the catalogue.
 *
 * `random` is deliberately absent. It is `ORDER BY RANDOM()` under a `LIMIT`,
 * so each page of `collect` is an independent draw and the assembled list would
 * both repeat and omit albums past the first page.
 */
export type AlbumSort =
  | "alphabeticalByName"
  | "alphabeticalByArtist"
  | "newest"
  | "recent"
  | "frequent"
  | "starred"
  | "byYear";

/**
 * How a song listing is ordered.
 *
 * A subset of what the server accepts, as `AlbumSort` is: `highest`,
 * `random` and `byGenre` exist on the wire and have their own screens here,
 * so offering them in this menu would give two ways to ask one question.
 *
 * `alphabeticalByAlbum` is the one an album listing has no use for: sleeve
 * order across the catalogue, album then disc then track.
 */
export type SongSort =
  | "alphabeticalByName"
  | "alphabeticalByArtist"
  | "alphabeticalByAlbum"
  | "newest"
  | "recent"
  | "frequent"
  | "starred"
  | "byYear";

/**
 * How many songs one page of the listing carries.
 *
 * Exported because the page that reads it has to know when a short answer
 * means the end — a page of fewer than this is the last one.
 */
export const SONG_PAGE = 200;

/**
 * One page of the catalogue's songs, in the requested order.
 *
 * **Paged, where `listAlbums` and `listArtists` use `collect` and fetch
 * everything.** That difference is the whole reason this is not one more
 * one-liner beside them: a `Song` carries 44 fields, and a library has an
 * order of magnitude more tracks than albums. Collecting a twenty-thousand
 * track catalogue would pull tens of megabytes through a phone to draw the
 * first screenful, and would make the server's own pagination decorative.
 */
export const listSongs = (
  sort: SongSort,
  libraryId: string | undefined,
  offset: number,
  limit = SONG_PAGE,
  signal?: AbortSignal,
) =>
  call<Song[]>(
    `/api/v2/songs?${new URLSearchParams(
      scoped(libraryId, {
        sort,
        offset: String(offset),
        limit: String(limit),
      }),
    )}`,
    { signal },
  );

/** Albums in the requested order, scoped to `libraryId` when one is given. */
export const listAlbums = (sort?: AlbumSort, libraryId?: string) =>
  collect<Album>("/api/v2/albums", scoped(libraryId, sort ? { sort } : {}));

export const getAlbum = (id: string) =>
  call<AlbumDetail>(`/api/v2/albums/${id}`);

/** Artists, scoped to `libraryId` when one is given. */
export const listArtists = (libraryId?: string) =>
  collect<Artist>("/api/v2/artists", scoped(libraryId));

export const getArtist = (id: string) =>
  call<ArtistDetail>(`/api/v2/artists/${id}`);

/**
 * An artist’s most played tracks, for this account.
 *
 * Its own request rather than a field on {@link getArtist}: the artist page
 * already holds its albums from one cached answer, and this list changes every
 * time something is played while that one does not.
 *
 * The server counts from local listening history and leaves out what has never
 * been played, so a fresh account gets an empty list — which is the true
 * answer, not a missing feature.
 */
export const artistTopSongs = (id: string, limit = 10) =>
  call<Song[]>(
    `/api/v2/artists/${id}/top-songs?${new URLSearchParams({ limit: String(limit) })}`,
  );

/**
 * Search, scoped to `libraryId` when one is given.
 *
 * Takes a `signal` because the search box asks on every keystroke now. `call`
 * already spreads its `init` into `fetch`, so this needed no plumbing — the
 * parameter only makes the abandonment sayable from a query function.
 *
 * It saves the server work rather than protecting the screen: a stale answer
 * could not be shown anyway, because every needle is its own cache key and
 * what the page renders is the entry for the needle it is on.
 */
export const search = (
  query: string,
  libraryId?: string,
  signal?: AbortSignal,
) =>
  call<SearchResult>(
    `/api/v2/search?${new URLSearchParams(scoped(libraryId, { q: query }))}`,
    { signal },
  );

export const getTrack = (id: string) => call<Song>(`/api/v2/tracks/${id}`);

export type Genre = {
  name: string;
  song_count: number;
  album_count: number;
};

/** Genre summaries, scoped to `libraryId` when one is given. */
export const listGenres = (libraryId?: string) =>
  call<Genre[]>(`/api/v2/genres?${new URLSearchParams(scoped(libraryId))}`);

/** Songs of one genre, scoped to `libraryId` when one is given. */
export const listGenreSongs = (genre: string, libraryId?: string) =>
  collect<Song>("/api/v2/songs/by-genre", scoped(libraryId, { genre }));

/** Request a random selection of songs, optionally scoped by genre and library. */
export const listRandomSongs = (
  limit = 100,
  genre?: string,
  libraryId?: string,
) => {
  const query = new URLSearchParams(
    scoped(libraryId, { limit: String(limit) }),
  );
  if (genre) query.set("genre", genre);
  return call<Song[]>(`/api/v2/songs/random?${query}`);
};

export function formatDuration(ms: number): string {
  const total = Math.round(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

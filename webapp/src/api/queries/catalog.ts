// The catalogue: albums, songs, artists, genres, a random draw, search.

import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query";
import {
  type AlbumSort,
  type ArtistDetail,
  artistTopSongs,
  getAlbum,
  getArtist,
  listAlbums,
  listArtistAlbums,
  listArtists,
  listGenreSongs,
  listGenres,
  listRandomSongs,
  listSongs,
  SONG_PAGE,
  type SongSort,
  search,
} from "../index";

export const albumsQuery = (sort: AlbumSort, scope: string | undefined) =>
  queryOptions({
    queryKey: ["albums", sort, scope ?? null],
    queryFn: () => listAlbums(sort, scope),
  });

/**
 * The catalogue's songs, a page at a time.
 *
 * The only infinite query here, and the only one that needs to be: every
 * other listing is small enough to hold whole. A short page is the end —
 * the route has no total to send, and inventing one would mean a second
 * `COUNT(*)` over the same filter on every request.
 */
export const songsQuery = (sort: SongSort, scope: string | undefined) =>
  infiniteQueryOptions({
    queryKey: ["songs", sort, scope ?? null],
    // The signal is handed on so a page still in flight when the sort or the
    // library changes is abandoned instead of finishing into a key nobody
    // will read. A page is two hundred rows, which is the largest answer this
    // client asks for.
    queryFn: ({ pageParam, signal }) =>
      listSongs(sort, scope, pageParam, SONG_PAGE, signal),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, lastParam) =>
      last.length < SONG_PAGE ? undefined : lastParam + SONG_PAGE,
  });

export const albumQuery = (albumId: string) =>
  queryOptions({
    queryKey: ["album", albumId],
    queryFn: () => getAlbum(albumId),
  });

export const artistsQuery = (scope: string | undefined) =>
  queryOptions({
    queryKey: ["artists", scope ?? null],
    queryFn: () => listArtists(scope),
  });

/** How many times an artist page is asked in all while enrichment is pending. */
export const ENRICHMENT_ASKS = 6;
/** The pause between two of those asks. */
export const ENRICHMENT_RETRY_MS = 3_000;

/**
 * When to ask for an artist again, or `false` for never.
 *
 * The web client does not read the library event stream, so it learns that a
 * provider answered only by asking. It asks while the state is `pending`, up
 * to {@link ENRICHMENT_ASKS} answers in all, and then stops: a provider that
 * is unreachable stays `pending` on the server, and a page left open must
 * not ask forever. The count is the query's, so a later visit refetches once
 * on mount, as every page does, and does not start the series again.
 */
export function enrichmentRetryDelay(
  artist: ArtistDetail | undefined,
  answers: number,
): number | false {
  if (artist?.enrichment?.state !== "pending") return false;
  return answers < ENRICHMENT_ASKS ? ENRICHMENT_RETRY_MS : false;
}

export const artistQuery = (artistId: string) =>
  queryOptions({
    queryKey: ["artist", artistId],
    queryFn: () => getArtist(artistId),
    // Failed asks count too: an error leaves the last answer — still
    // `pending` — in place and does not move `dataUpdateCount`, so a server
    // that keeps failing would otherwise be asked every few seconds for ever.
    refetchInterval: (query) =>
      enrichmentRetryDelay(
        query.state.data,
        query.state.dataUpdateCount + query.state.errorUpdateCount,
      ),
  });

/** An artist's albums alone, for the album page's "more by". */
export const artistAlbumsQuery = (artistId: string) =>
  queryOptions({
    queryKey: ["artist-albums", artistId],
    queryFn: () => listArtistAlbums(artistId),
  });

export const artistTopSongsQuery = (artistId: string) =>
  queryOptions({
    queryKey: ["artist-top-songs", artistId],
    queryFn: () => artistTopSongs(artistId),
  });

export const genresQuery = (scope: string | undefined) =>
  queryOptions({
    queryKey: ["genres", scope ?? null],
    queryFn: () => listGenres(scope),
  });

export const genreSongsQuery = (genre: string, scope: string | undefined) =>
  queryOptions({
    queryKey: ["genre-songs", genre, scope ?? null],
    queryFn: () => listGenreSongs(genre, scope),
  });

/**
 * A fresh draw each time it is asked for.
 *
 * `staleTime: 0` on purpose, against the default: every other question here is
 * worth holding, and this one is worth exactly the opposite. Coming back to a
 * shuffle and seeing the previous shuffle would be a cache doing its job and
 * the page failing at its own.
 */
export const randomSongsQuery = (scope: string | undefined) =>
  queryOptions({
    queryKey: ["random-songs", scope ?? null],
    queryFn: () => listRandomSongs(100, undefined, scope),
    staleTime: 0,
    gcTime: 0,
  });

export const searchQuery = (needle: string, scope: string | undefined) =>
  queryOptions({
    // The scope belongs in the key: changing library while a result is on
    // screen has to re-ask, or the page keeps answering for the library it
    // left.
    queryKey: ["search", needle, scope ?? null],
    // The signal is the query's own: a needle superseded while its request is
    // still out is abandoned rather than left to finish into a cache entry
    // nobody will read. It costs the screen nothing — every needle is its own
    // key, so an answer that arrives late lands under the key it was asked
    // for and not over the one on screen — and it stops a search box from
    // spending one round trip per keystroke on the server.
    queryFn: ({ signal }) => search(needle, scope, signal),
    enabled: needle.length > 0,
  });

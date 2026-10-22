import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query";
import { playsByInstant, scrobbleRows, tracksToName } from "../lib/scrobbling";
import {
  type AlbumSort,
  type ArtistDetail,
  artistTopSongs,
  canvasUrl,
  FOLDER_TRACKS_PAGE,
  getAlbum,
  getArtist,
  getLyrics,
  getTrack,
  getTrackCredits,
  getTrackOverrides,
  getTrackProperties,
  listAlbums,
  listApiTokens,
  listArtistAlbums,
  listArtists,
  listBookmarks,
  listFavorites,
  listFolders,
  listFolderTracks,
  listGenreSongs,
  listGenres,
  listHistory,
  listLibraries,
  listLibraryMembers,
  listNowPlaying,
  listOwnApiTokens,
  listPlaylists,
  listRadioStations,
  listRandomSongs,
  listScrobbleDestinations,
  listScrobbleLinks,
  listShares,
  listSongs,
  listUncertainScrobbles,
  listUsers,
  readOwnSubsonicCredential,
  SONG_PAGE,
  type SongSort,
  search,
} from "./index";

/**
 * Every question this client asks the server, and the key each answer is held
 * under.
 *
 * Gathered here rather than written at each screen, because a key is shared
 * state: two screens that spell one differently keep two copies of the same
 * answer and disagree after a mutation, and two that spell different questions
 * the same way serve each other's data. Neither mistake shows up in a type.
 *
 * A key is the question, argument by argument — `["albums", sort, scope]` and
 * not `["albums"]` — so changing a sort or a library asks again instead of
 * returning what the other one answered.
 */

const HISTORY_PLAYS = 200;

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

export const playlistsQuery = () =>
  queryOptions({ queryKey: ["playlists"], queryFn: listPlaylists });

export const sharesQuery = () =>
  queryOptions({ queryKey: ["shares"], queryFn: listShares });

export const bookmarksQuery = () =>
  queryOptions({ queryKey: ["bookmarks"], queryFn: listBookmarks });

export const radioStationsQuery = () =>
  queryOptions({ queryKey: ["radio-stations"], queryFn: listRadioStations });

/**
 * One track as its properties panel shows it, with the corrections behind
 * its tags so each corrected value can say that it is one.
 *
 * Not `trackEditQuery`, though it reads the same two routes: that one is the
 * editor's and is invalidated by its saves; sharing a key would tie the two
 * screens' lifetimes together for no saving worth having.
 */
export const trackPropertiesQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["track-properties", trackId],
    queryFn: async () => {
      // The corrections only mark which tags were corrected. A failure to
      // read them costs those marks, not the panel.
      const [track, tracked] = await Promise.all([
        getTrackProperties(trackId),
        getTrackOverrides(trackId).catch(() => null),
      ]);
      return { track, tracked };
    },
  });

/**
 * A library's whole filed tree: bounded by the server's folder cap, so it
 * is held whole rather than paged.
 */
export const filedTreeQuery = (libraryId: string) =>
  queryOptions({
    queryKey: ["filed-tree", libraryId],
    queryFn: () => listFolders(libraryId),
  });

/**
 * The tracks filed in one folder, or the unfiled ones, a page at a time. A
 * short page is the end.
 */
export const folderTracksQuery = (libraryId: string, folder: string) =>
  infiniteQueryOptions({
    queryKey: ["folder-tracks", libraryId, folder],
    queryFn: ({ pageParam, signal }) =>
      listFolderTracks(
        libraryId,
        folder,
        pageParam,
        FOLDER_TRACKS_PAGE,
        signal,
      ),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, lastParam) =>
      last.length < FOLDER_TRACKS_PAGE
        ? undefined
        : lastParam + FOLDER_TRACKS_PAGE,
  });

export const lyricsQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["lyrics", trackId],
    queryFn: () => getLyrics(trackId),
  });

/**
 * Who is listening, re-asked on a timer.
 *
 * The panel used to hold the previous reading in a `useState` of its own,
 * because the hook beneath it blanked its value at the start of every poll and
 * flashed "nobody is listening" thirty seconds apart. A query keeps what it has
 * while it refetches, so that workaround is gone rather than moved.
 */
export const nowPlayingQuery = () =>
  queryOptions({
    queryKey: ["now-playing"],
    queryFn: listNowPlaying,
    refetchInterval: 30_000,
    staleTime: 0,
  });

/**
 * The favourites, resolved to the tracks themselves.
 *
 * `Promise.allSettled`, so one track that has since become unreadable costs its
 * own row and not the page.
 */
export const favoriteTracksQuery = () =>
  queryOptions({
    queryKey: ["favorite-tracks"],
    queryFn: async () => {
      const favorites = await listFavorites();
      const tracks = favorites.filter((item) => item.entity_type === "track");
      const resolved = await Promise.allSettled(
        tracks.map((item) => getTrack(item.entity_id)),
      );
      return resolved.flatMap((result) =>
        result.status === "fulfilled" ? [result.value] : [],
      );
    },
  });

/**
 * Recently played, one entry per track.
 *
 * The route answers plays, not songs, and the same track appears many times.
 * Keeping the first sighting of each id gives "recently played" in the order it
 * was last played, and asks for each track once.
 */
export const historyTracksQuery = (limit: number) =>
  queryOptions({
    queryKey: ["history-tracks", limit],
    queryFn: async () => {
      const plays = await listHistory(HISTORY_PLAYS);
      const seen = new Set<string>();
      const ordered = plays.filter((play) => {
        if (seen.has(play.track_id)) return false;
        seen.add(play.track_id);
        return true;
      });
      const resolved = await Promise.allSettled(
        ordered.slice(0, limit).map((play) => getTrack(play.track_id)),
      );
      return resolved.flatMap((result) =>
        result.status === "fulfilled" ? [result.value] : [],
      );
    },
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

export const apiTokensQuery = (username: string, open: boolean) =>
  queryOptions({
    queryKey: ["api-tokens", username],
    queryFn: () => listApiTokens(username),
    // Closed until asked for. This panel is rendered once per account, so
    // loading on mount meant one request per account every time the admin
    // screen opened — for a list almost nobody opens.
    enabled: open,
  });

/**
 * This account's own registered clients.
 *
 * Unconditional, unlike `apiTokensQuery`, which is gated on a disclosure
 * because the administration screen renders one per account. There is exactly
 * one of these and the page it sits on is about it.
 */
export const ownApiTokensQuery = () =>
  queryOptions({
    queryKey: ["own-api-tokens"],
    queryFn: listOwnApiTokens,
  });

/**
 * Whether this account holds a Subsonic credential.
 *
 * One of these, like the clients listing beside it, because the page it sits
 * on is about this account.
 */
export const ownSubsonicCredentialQuery = () =>
  queryOptions({
    queryKey: ["own-subsonic-credential"],
    queryFn: readOwnSubsonicCredential,
  });

export const libraryMembersQuery = (libraryId: string, open: boolean) =>
  queryOptions({
    queryKey: ["library-members", libraryId],
    queryFn: () => listLibraryMembers(libraryId),
    enabled: open,
  });

export const administrationQuery = () =>
  queryOptions({
    queryKey: ["administration"],
    queryFn: () => Promise.all([listLibraries(), listUsers()]),
  });

/**
 * A canvas ticket, held for exactly as long as it is being looked at.
 *
 * `staleTime` and `gcTime` at zero, against the defaults, because what this
 * answers is not data but a **credential with a deadline** — an AEAD-sealed
 * ticket the `<video>` plays from, since it can send no Authorization header.
 * A cache has no notion of that deadline and would go on serving the ticket
 * after it passed. The lifetime is `WAVEFLOW_STREAM_TICKET_TTL`, an hour by
 * default and an operator's to shorten, so "shorter than the cache" is a
 * configuration away rather than impossible. `player.tsx` checks the deadline
 * before playing a stream ticket for the same reason; here there is simply
 * nothing worth keeping.
 *
 * A track with no canvas answers `null`, which a query holds as an answer. The
 * hook this replaced could not tell that from "still waiting" and had to be
 * handed an object to unwrap.
 */
export const canvasQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["canvas", trackId],
    queryFn: () => canvasUrl(trackId),
    staleTime: 0,
    gcTime: 0,
  });

export const scrobbleRowsQuery = () =>
  queryOptions({
    queryKey: ["scrobble-rows"],
    queryFn: async () => {
      const [destinations, links] = await Promise.all([
        listScrobbleDestinations(),
        listScrobbleLinks(),
      ]);
      return scrobbleRows(destinations, links);
    },
  });

/**
 * The listens nobody has answered yet, with the tracks behind them named.
 *
 * The history read happens only when there is something to name: an empty list
 * must not cost one on every visit to the page.
 */
export const uncertainScrobblesQuery = () =>
  queryOptions({
    queryKey: ["uncertain-scrobbles"],
    queryFn: async () => {
      const entries = await listUncertainScrobbles();
      if (entries.length === 0) {
        return { entries, titles: new Map<string, string>() };
      }
      const plays = playsByInstant(await listHistory(HISTORY_PLAYS));
      const resolved = await Promise.allSettled(
        tracksToName(entries, plays).map((id) => getTrack(id)),
      );
      const byTrack = new Map<string, string>();
      for (const result of resolved) {
        if (result.status === "fulfilled") {
          byTrack.set(result.value.id, result.value.title);
        }
      }
      // Re-keyed by entry, not by track: the row shows the title beside the
      // listen it belongs to, and two entries can name the same track.
      const titles = new Map<string, string>();
      for (const entry of entries) {
        const track = plays.get(entry.played_at);
        const title = track ? byTrack.get(track) : undefined;
        if (title) titles.set(entry.id, title);
      }
      return { entries, titles };
    },
  });

/**
 * Everything the tag editor shows for one track: its effective values, and the
 * corrections behind them.
 *
 * Two reads rather than one, because provenance appears on this screen and
 * nowhere else — the catalogue shows the effective value everywhere.
 */
export const trackEditQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["track-edit", trackId],
    queryFn: async () => {
      const [song, tracked] = await Promise.all([
        getTrackCredits(trackId),
        getTrackOverrides(trackId),
      ]);
      return { song, tracked };
    },
  });

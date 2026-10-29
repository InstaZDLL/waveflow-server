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

export {
  administrationQuery,
  apiTokensQuery,
  libraryMembersQuery,
  librarySettingsQuery,
  ownApiTokensQuery,
  ownSubsonicCredentialQuery,
} from "./administration";
export {
  albumQuery,
  albumsQuery,
  artistAlbumsQuery,
  artistQuery,
  artistsQuery,
  artistTopSongsQuery,
  ENRICHMENT_ASKS,
  ENRICHMENT_RETRY_MS,
  enrichmentRetryDelay,
  genreSongsQuery,
  genresQuery,
  randomSongsQuery,
  searchQuery,
  songsQuery,
} from "./catalog";
export {
  bookmarksQuery,
  playlistsQuery,
  podcastChannelQuery,
  podcastsQuery,
  radioStationsQuery,
  sharesQuery,
  smartPlaylistQuery,
  smartPlaylistsQuery,
} from "./collections";
export {
  favoriteTracksQuery,
  historyTracksQuery,
  mixesQuery,
  mixQuery,
  nowPlayingQuery,
  scrobbleRowsQuery,
  statsQuery,
  uncertainScrobblesQuery,
} from "./listening";
export {
  attentionSummaryQuery,
  attentionTracksQuery,
  creditCandidatesQuery,
  creditDecisionsQuery,
  filedTreeQuery,
  folderTracksQuery,
  purgedTracksQuery,
  qualitySummaryQuery,
  qualityTracksQuery,
  trackQualityQuery,
  unavailableTracksQuery,
} from "./maintenance";
export { serverIdentityQuery } from "./server";
export {
  canvasQuery,
  lyricsQuery,
  trackEditQuery,
  trackPropertiesQuery,
} from "./tracks";

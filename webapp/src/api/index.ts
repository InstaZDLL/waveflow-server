/**
 * The client over /api/v2, one file per resource.
 *
 * Import from here rather than from a file beside it: this is the surface the
 * rest of the app sees. `client.ts` holds the session and the transport every
 * other file sends through, and what it shares with them alone stays off this
 * list.
 */

export type { ApiToken, SubsonicCredential } from "./account";
export {
  createOwnApiToken,
  listOwnApiTokens,
  readOwnSubsonicCredential,
  revokeOwnApiToken,
  revokeOwnSubsonicCredential,
  rotateOwnSubsonicCredential,
} from "./account";
export { artworkUrl, cachedArtworkUrl } from "./artwork";
export type { CanvasBlob } from "./canvas";
export { canvasUrl, placeCanvas, removeCanvas } from "./canvas";
export type {
  Album,
  AlbumDetail,
  AlbumSort,
  Artist,
  ArtistDetail,
  ArtistEnrichment,
  EnrichmentSource,
  Genre,
  SearchResult,
  Song,
  SongSort,
} from "./catalog";
export {
  artistTopSongs,
  formatDuration,
  getAlbum,
  getArtist,
  getTrack,
  listAlbums,
  listArtistAlbums,
  listArtists,
  listGenreSongs,
  listGenres,
  listRandomSongs,
  listSongs,
  SONG_PAGE,
  search,
} from "./catalog";
export type { SessionUser, WebSession } from "./client";
export {
  ApiError,
  bootstrapAdmin,
  currentUser,
  ensureSession,
  forgetOnSessionChange,
  hasSession,
  login,
  logout,
  setupRequired,
} from "./client";
export type { EntityKind, Favorite, Rating } from "./favorites";
export {
  listFavorites,
  listRatings,
  setFavorite,
  setRating,
} from "./favorites";
export type { Library, LibraryMember, ScanJob } from "./libraries";
export {
  addLibrary,
  drainEventStream,
  getScan,
  listLibraries,
  listLibraryMembers,
  removeLibraryMember,
  setLibraryMember,
  startScan,
  watchScan,
} from "./libraries";
export type { AuthorizeRequest } from "./oauth";
export { authorize, isAllowedRedirect, safeInternalPath } from "./oauth";
export type { Bookmark, NowPlaying, Play, Queue } from "./playback";
export {
  deleteBookmark,
  getQueue,
  listBookmarks,
  listHistory,
  listNowPlaying,
  saveQueue,
  setBookmark,
} from "./playback";
export type { Playlist, PlaylistImport, UnresolvedEntry } from "./playlists";
export {
  appendToPlaylist,
  createPlaylist,
  deletePlaylist,
  exportPlaylist,
  importPlaylist,
  listPlaylists,
} from "./playlists";
export type { RadioStation, RadioStationDraft } from "./radio";
export {
  createRadioStation,
  deleteRadioStation,
  listRadioStations,
  replaceRadioStation,
} from "./radio";
export type {
  KnownScrobbleUnavailable,
  ScrobbleDestination,
  ScrobbleLink,
  ScrobbleProvider,
  ScrobbleUnavailable,
  UncertainScrobble,
} from "./scrobbling";
export {
  authorizeLastFm,
  discardUncertainScrobble,
  linkScrobble,
  listScrobbleDestinations,
  listScrobbleLinks,
  listUncertainScrobbles,
  retryUncertainScrobble,
  scrobble,
  unlinkScrobble,
} from "./scrobbling";
export type { Share } from "./shares";
export { createShare, deleteShare, listShares } from "./shares";
export type { StreamUrl } from "./stream";
export { streamUrl } from "./stream";
export type {
  LyricsLine,
  LyricsList,
  SongCredits,
  StructuredLyrics,
  TrackCorrectionPatch,
  TrackOverrides,
  TrackOverrideValues,
  TrackProperties,
  TrackSourceTags,
} from "./tracks";
export {
  correctTrack,
  getLyrics,
  getTrackCredits,
  getTrackOverrides,
  getTrackProperties,
} from "./tracks";
export type {
  CommittedUpload,
  UploadDecision,
  UploadOffer,
  UploadSessionState,
  UploadVerdict,
} from "./uploads";
export {
  commitUpload,
  getUploadSession,
  negotiateUploads,
  putUploadChunk,
} from "./uploads";
export type { User } from "./users";
export {
  createApiToken,
  createUser,
  listApiTokens,
  listUsers,
  revokeApiToken,
  setSubsonicCredential,
  setUserDisabled,
} from "./users";

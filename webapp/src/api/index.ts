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
export type {
  AttentionCategory,
  AttentionCount,
  AttentionTrack,
  CreditCandidate,
  CreditDecision,
  CreditDecisionKind,
  CreditFragment,
  TrackCategory,
} from "./attention";
export {
  ATTENTION_TRACKS_PAGE,
  decideCredit,
  getAttentionSummary,
  listAttentionTracks,
  listCreditCandidates,
  listCreditDecisions,
  removeCreditDecision,
} from "./attention";
export type { CanvasBlob } from "./canvas";
export { canvasUrl, placeCanvas, removeCanvas } from "./canvas";
export type {
  Album,
  AlbumDetail,
  AlbumSort,
  Artist,
  ArtistDetail,
  ArtistEnrichment,
  ArtistImageCandidate,
  ArtistImageKind,
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
  getArtistImageCandidates,
  getTrack,
  listAlbums,
  listArtistAlbums,
  listArtists,
  listGenreSongs,
  listGenres,
  listRandomSongs,
  listSongs,
  pickDeezerPortrait,
  removeArtistImage,
  SONG_PAGE,
  search,
  setArtistImage,
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
export type { FiledFolder, FiledTree } from "./folders";
export {
  createFolder,
  deleteFolder,
  FOLDER_TRACKS_PAGE,
  folderOutline,
  folderTrail,
  isWithin,
  listFolders,
  listFolderTracks,
  MAX_PLACEMENTS,
  placeTracks,
  UNFILED,
  updateFolder,
} from "./folders";
export type {
  Library,
  LibraryMember,
  LibrarySettings,
  ScanJob,
  UnavailableTrack,
} from "./libraries";
export {
  addLibrary,
  deleteLibrary,
  drainEventStream,
  getLibrarySettings,
  getScan,
  listLibraries,
  listLibraryMembers,
  listUnavailableTracks,
  purgeUnavailableTracks,
  removeLibraryMember,
  setLibraryMember,
  startScan,
  UNAVAILABLE_PAGE,
  updateLibrary,
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
export type {
  AnalysisState,
  QualityAnalysis,
  QualitySummary,
  QualityTrack,
  QualityVerdict,
  TrackQuality,
  TrackQualityDetail,
} from "./quality";
export {
  getQualitySummary,
  getTrackQuality,
  listQualityTracks,
  QUALITY_TRACKS_PAGE,
  QUALITY_VERDICTS,
  requestQualityAnalysis,
  spectrogramUrl,
} from "./quality";
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
  LyricsFormat,
  LyricsLine,
  LyricsList,
  LyricsSource,
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
  deleteLyricsUpload,
  fetchLyricsFile,
  getLyrics,
  getTrackCredits,
  getTrackOverrides,
  getTrackProperties,
  uploadLyrics,
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

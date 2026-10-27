// Library upkeep: needs attention, quality, filed folders, missing files.

import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query";
import {
  ATTENTION_TRACKS_PAGE,
  FOLDER_TRACKS_PAGE,
  getAttentionSummary,
  getQualitySummary,
  getTrackQuality,
  listAttentionTracks,
  listCreditCandidates,
  listCreditDecisions,
  listFolders,
  listFolderTracks,
  listPurgedTracks,
  listQualityTracks,
  listUnavailableTracks,
  QUALITY_TRACKS_PAGE,
  type QualityVerdict,
  type TrackCategory,
  UNAVAILABLE_PAGE,
} from "../index";

/**
 * "Needs attention" (RFC-015): every query of the page is keyed under
 * `attention` and the library, so one invalidation after a decision, a
 * correction or a scan refreshes the whole page.
 */
export const attentionSummaryQuery = (libraryId: string) =>
  queryOptions({
    queryKey: ["attention", libraryId, "summary"],
    queryFn: () => getAttentionSummary(libraryId),
  });

export const creditCandidatesQuery = (libraryId: string) =>
  queryOptions({
    queryKey: ["attention", libraryId, "credits"],
    queryFn: () => listCreditCandidates(libraryId),
  });

export const creditDecisionsQuery = (libraryId: string) =>
  queryOptions({
    queryKey: ["attention", libraryId, "decisions"],
    queryFn: () => listCreditDecisions(libraryId),
  });

/** The tracks of one category, a page at a time. A short page is the end. */
export const attentionTracksQuery = (
  libraryId: string,
  category: TrackCategory,
) =>
  infiniteQueryOptions({
    queryKey: ["attention", libraryId, "tracks", category],
    queryFn: ({ pageParam, signal }) =>
      listAttentionTracks(
        libraryId,
        category,
        pageParam,
        ATTENTION_TRACKS_PAGE,
        signal,
      ),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, lastParam) =>
      last.length < ATTENTION_TRACKS_PAGE
        ? undefined
        : lastParam + ATTENTION_TRACKS_PAGE,
  });

/**
 * The quality page (RFC-016): keyed under `quality` and the library, so one
 * invalidation after a request for a new analysis refreshes the whole page.
 * While the analysis runs, the summary is asked again every ten seconds by the
 * page itself; the list follows when the counts move.
 */
export const qualitySummaryQuery = (libraryId: string) =>
  queryOptions({
    queryKey: ["quality", libraryId, "summary"],
    queryFn: () => getQualitySummary(libraryId),
  });

/** One filter of the list, a page at a time. A short page is the end. */
export const qualityTracksQuery = (
  libraryId: string,
  filter: { verdict?: QualityVerdict; codec?: string },
) =>
  infiniteQueryOptions({
    queryKey: [
      "quality",
      libraryId,
      "tracks",
      filter.verdict ?? "",
      filter.codec ?? "",
    ],
    queryFn: ({ pageParam, signal }) =>
      listQualityTracks(
        libraryId,
        filter,
        pageParam,
        QUALITY_TRACKS_PAGE,
        signal,
      ),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, lastParam) =>
      last.length < QUALITY_TRACKS_PAGE
        ? undefined
        : lastParam + QUALITY_TRACKS_PAGE,
  });

/** One track's verdict and spectrum, for its properties. */
export const trackQualityQuery = (trackId: string) =>
  queryOptions({
    queryKey: ["track-quality", trackId],
    queryFn: () => getTrackQuality(trackId),
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

/**
 * The files a library's scans can no longer find (RFC-021), a page at a time,
 * oldest first. Keyed under `missing` and the library, so one invalidation
 * after a purge refreshes the page.
 */
export const unavailableTracksQuery = (libraryId: string) =>
  infiniteQueryOptions({
    queryKey: ["missing", libraryId],
    queryFn: ({ pageParam, signal }) =>
      listUnavailableTracks(libraryId, pageParam, UNAVAILABLE_PAGE, signal),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, lastParam) =>
      last.tracks.length < UNAVAILABLE_PAGE
        ? undefined
        : lastParam + UNAVAILABLE_PAGE,
  });

export const purgedTracksQuery = (libraryId: string) =>
  infiniteQueryOptions({
    queryKey: ["missing", libraryId, "purged"],
    queryFn: ({ pageParam, signal }) =>
      listPurgedTracks(libraryId, pageParam, UNAVAILABLE_PAGE, signal),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, lastParam) =>
      last.tracks.length < UNAVAILABLE_PAGE
        ? undefined
        : lastParam + UNAVAILABLE_PAGE,
  });

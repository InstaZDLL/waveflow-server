import type { Song } from "./catalog";
import { call } from "./client";

/**
 * "Needs attention" (RFC-015): what a library's owners and managers are asked
 * to look at. The joined credits waiting for a split-or-keep decision, and the
 * desktop's track categories the server can compute and act on.
 */
export type AttentionCategory =
  | "split_credits"
  | "missing_artist"
  | "missing_album"
  | "missing_year"
  | "missing_track_number"
  | "missing_cover"
  | "album_year_conflict"
  | "compilation_no_album_artist"
  | "duplicate_track_number"
  | "track_number_gap"
  | "probable_duplicate"
  | "probable_fake_lossless";

/** A track category: every category but the credits. */
export type TrackCategory = Exclude<AttentionCategory, "split_credits">;

export type AttentionCount = { category: AttentionCategory; count: number };

/** One fragment of a joined credit, and the artist it would land on. */
export type CreditFragment = {
  name: string;
  /** Set when the library already has an artist of that name. */
  artist_id: string | null;
  album_count: number;
};

/** A joined credit with no decision yet. */
export type CreditCandidate = {
  artist_id: string;
  name: string;
  track_count: number;
  fragments: CreditFragment[];
};

export type CreditDecisionKind = "split" | "keep";

export type CreditDecision = {
  id: string;
  /** The credit as it read when the decision was taken. */
  credit: string;
  decision: CreditDecisionKind;
  /** What a split credits instead; absent for a keep. */
  parts?: string[];
  /** Who decided, by username; `null` once that account is gone. */
  decided_by: string | null;
  decided_at: number;
};

/** A track in one category; a probable duplicate names its group. */
export type AttentionTrack = Song & { duplicate_group?: number };

/** How many tracks one page of a category carries. */
export const ATTENTION_TRACKS_PAGE = 200;

export const getAttentionSummary = (libraryId: string) =>
  call<AttentionCount[]>(`/api/v2/libraries/${libraryId}/attention`);

export const listCreditCandidates = (libraryId: string) =>
  call<CreditCandidate[]>(`/api/v2/libraries/${libraryId}/attention/credits`);

export const listAttentionTracks = (
  libraryId: string,
  category: TrackCategory,
  offset: number,
  limit = ATTENTION_TRACKS_PAGE,
  signal?: AbortSignal,
) =>
  call<AttentionTrack[]>(
    `/api/v2/libraries/${libraryId}/attention/tracks?${new URLSearchParams({
      category,
      offset: String(offset),
      limit: String(limit),
    })}`,
    { signal },
  );

export const listCreditDecisions = (libraryId: string) =>
  call<CreditDecision[]>(`/api/v2/libraries/${libraryId}/credit-decisions`);

/**
 * Splits the credit an artist makes, or keeps it whole. The server applies it
 * at once and to every later scan; `409` when the credit is already decided,
 * `422` when it has nothing to split.
 */
export const decideCredit = (
  libraryId: string,
  artistId: string,
  decision: CreditDecisionKind,
) =>
  call<CreditDecision>(`/api/v2/libraries/${libraryId}/credit-decisions`, {
    method: "POST",
    body: JSON.stringify({ artist_id: artistId, decision }),
  });

/** Undoes a decision: the library reads as if it had never been taken. */
export const removeCreditDecision = (libraryId: string, decisionId: string) =>
  call<void>(`/api/v2/libraries/${libraryId}/credit-decisions/${decisionId}`, {
    method: "DELETE",
  });

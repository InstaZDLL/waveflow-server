import { call } from "./client";

/**
 * The most tracks one share holds: the server mints a pass per track when a
 * share is opened (`MAX_SHARE_TRACKS`), and refuses a longer list.
 */
export const MAX_SHARE_TRACKS = 400;

export type Share = {
  id: string;
  url?: string;
  description: string | null;
  expires_at: number | null;
  created_at: number;
  /** Openings of the share, not stream requests (RFC-020). */
  visit_count: number;
  /** How many openings the share allows; `null` for no cap. */
  max_visits: number | null;
  track_ids: string[];
};

export const listShares = () => call<Share[]>("/api/v2/shares");

export const createShare = (
  trackIds: string[],
  description: string,
  maxVisits: number | null = null,
) =>
  call<Share>("/api/v2/shares", {
    method: "POST",
    body: JSON.stringify({
      track_ids: trackIds,
      description,
      ...(maxVisits === null ? {} : { max_visits: maxVisits }),
    }),
  });

export const deleteShare = (id: string) =>
  call<void>(`/api/v2/shares/${id}`, { method: "DELETE" });

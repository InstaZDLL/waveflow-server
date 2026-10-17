import { call } from "./client";

export type Share = {
  id: string;
  url?: string;
  description: string | null;
  expires_at: number | null;
  created_at: number;
  visit_count: number;
  track_ids: string[];
};

export const listShares = () => call<Share[]>("/api/v2/shares");

export const createShare = (trackIds: string[], description: string) =>
  call<Share>("/api/v2/shares", {
    method: "POST",
    body: JSON.stringify({ track_ids: trackIds, description }),
  });

export const deleteShare = (id: string) =>
  call<void>(`/api/v2/shares/${id}`, { method: "DELETE" });

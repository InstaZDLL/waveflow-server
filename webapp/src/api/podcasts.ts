import { call } from "./client";

/**
 * Podcasts (RFC-025): one list per server. An administrator subscribes and
 * every account listens. Episodes are downloaded by the server, then played
 * from it like a track — `streamUrl(episode.id)` — and the position is kept
 * with `setBookmark(episode.id, …)`, as a track's is.
 */
export type PodcastChannel = {
  id: string;
  /** In full for an account that may manage podcasts; without its query string otherwise. */
  url: string;
  title: string;
  description: string | null;
  site_url: string | null;
  /** The feed's own cover address. The server does not fetch it. */
  image_url: string | null;
  status: "completed" | "error";
  /** Why the last fetch failed, as a word: `unreached`, `http_404`, `invalid_feed`… */
  error: string | null;
  created_at: number;
  refreshed_at: number | null;
  episode_count: number;
  /** On a read of one channel. */
  episodes?: PodcastEpisode[];
};

export type PodcastEpisodeStatus =
  | "new"
  | "downloading"
  | "completed"
  | "error"
  | "deleted";

export type PodcastEpisode = {
  id: string;
  channel_id: string;
  channel_title: string;
  title: string;
  description: string | null;
  published_at: number | null;
  duration_ms: number | null;
  status: PodcastEpisodeStatus;
  error: string | null;
  size: number | null;
  suffix: string | null;
  content_type: string | null;
  /** Where this account stopped, from its bookmark. */
  position_ms: number | null;
  created_at: number;
};

export type PodcastOverview = {
  /** `WAVEFLOW_PODCASTS`: off by default. */
  enabled: boolean;
  /** Whether this account may subscribe, refresh, download and delete. */
  can_manage: boolean;
  channels: PodcastChannel[];
};

export const getPodcastOverview = () =>
  call<PodcastOverview>("/api/v2/podcasts");

export const getPodcastChannel = (id: string) =>
  call<PodcastChannel>(`/api/v2/podcasts/${id}`);

/** The feed is fetched and read before anything is stored. */
export const subscribePodcast = (url: string) =>
  call<PodcastChannel>("/api/v2/podcasts", {
    method: "POST",
    body: JSON.stringify({ url }),
  });

export const unsubscribePodcast = (id: string) =>
  call<void>(`/api/v2/podcasts/${id}`, { method: "DELETE" });

export const refreshPodcasts = () =>
  call<void>("/api/v2/podcasts/refresh", { method: "POST" });

export const downloadPodcastEpisode = (id: string) =>
  call<void>(`/api/v2/podcast-episodes/${id}/download`, { method: "POST" });

export const deletePodcastEpisode = (id: string) =>
  call<void>(`/api/v2/podcast-episodes/${id}`, { method: "DELETE" });

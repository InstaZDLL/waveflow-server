import { ApiError, call } from "./client";
import type { StreamUrl } from "./stream";

/**
 * What a track says about its music video (RFC-029 decision 8): the container
 * and the size, and nothing that names the file.
 */
export type ClipSummary = {
  format: "mp4" | "m4v" | "mov" | "webm" | "mkv";
  byte_size: number;
};

/**
 * A URL a `<video>` can play for a track's music video, or `null` when the
 * server says it has none.
 *
 * Asked for each time the clip opens, and again when a ticket runs out while
 * the clip is paused: a ticket is a permission for an hour, not a property of
 * the track, so nothing is cached. A 404 is the server's "no clip" — the track
 * has none, it is not this account's, or its file is gone since the scan, and
 * the server does not tell those apart. Any other failure is thrown, because a
 * server that could not answer has not said there is no clip.
 */
export async function clipUrl(trackId: string): Promise<StreamUrl | null> {
  try {
    const ticket = await call<{ url: string; expires_at: number }>(
      `/api/v2/tracks/${trackId}/clip-ticket`,
      { method: "POST" },
    );
    return { url: ticket.url, expiresAt: ticket.expires_at };
  } catch (cause) {
    if (cause instanceof ApiError && cause.status === 404) return null;
    throw cause;
  }
}

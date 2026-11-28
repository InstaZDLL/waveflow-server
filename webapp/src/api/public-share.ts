import { publicCredentials, serverUrl } from "./base";
import type { Song } from "./catalog";
import { ApiError } from "./client";

/** A track as a share answers it: the song, and where to play it from. */
export type PublicShareTrack = Song & {
  /**
   * Carries the visit's pass (RFC-020 decision 4). Used as given: an address
   * built from the track id alone opens nothing.
   */
  streamUrl: string;
  /**
   * The track's cover, under the same pass (RFC-020): absent for a track that
   * has none. A plain address, so `<img src>` loads it with no session.
   */
  coverUrl?: string;
};

export type PublicShare = {
  id: string;
  description: string | null;
  expiresAt: number | null;
  visitCount: number;
  tracks: PublicShareTrack[];
};

/**
 * Opens a share, which counts one visit, and answers what it holds — or
 * `null` when the link opens nothing: unknown, expired, deleted, or past its
 * visit cap. The server says the same 404 for all four, and so does this.
 *
 * **Plain `fetch`, not the session client.** The visitor has no account, and
 * a signed-in owner opening their own link must not send a bearer the share
 * route neither needs nor reads. `accept` is what makes this address answer
 * JSON rather than the page that is asking (decision 3).
 */
export async function openPublicShare(
  token: string,
): Promise<PublicShare | null> {
  const response = await fetch(
    serverUrl(`/share/${encodeURIComponent(token)}`),
    {
      headers: { accept: "application/json" },
      credentials: publicCredentials(),
      cache: "no-store",
      referrerPolicy: "no-referrer",
    },
  );
  if (response.status === 404) return null;
  // The token is the credential, so it stays out of the message.
  if (!response.ok) throw new ApiError(response.status, "GET /share/{token}");
  const share = (await response.json()) as PublicShare;
  // With no public URL the server names a track's stream and cover as root
  // paths, which under Home Assistant's prefix (RFC-030) are asked for there;
  // a full address, with one, goes through unchanged.
  return {
    ...share,
    tracks: share.tracks.map((track) => ({
      ...track,
      streamUrl: serverUrl(track.streamUrl),
      ...(track.coverUrl ? { coverUrl: serverUrl(track.coverUrl) } : {}),
    })),
  };
}

import { serverUrl } from "./base";
import { call } from "./client";

/**
 * Exchanges the session for a URL an <audio> element can play. The element
 * cannot send an Authorization header, so the ticket in the path is the
 * credential; it authorises this one track and is re-checked on every range
 * request the browser makes while seeking.
 */
export type StreamUrl = { url: string; expiresAt: number };

export async function streamUrl(trackId: string): Promise<StreamUrl> {
  const ticket = await call<{ url: string; expires_at: number }>(
    `/api/v2/tracks/${trackId}/stream-ticket`,
    { method: "POST" },
  );
  // A root path, which this page may have to ask for under a prefix.
  return { url: serverUrl(ticket.url), expiresAt: ticket.expires_at };
}

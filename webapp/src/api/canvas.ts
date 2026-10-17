import {
  ApiError,
  call,
  canvaslessTracks,
  canvasTickets,
  endSession,
  forgetCanvas,
  parse,
  renewFor,
  session,
  sessionGeneration,
} from "./client";
import type { StreamUrl } from "./stream";

/** A loop as the store holds it, named by its content. */
export type CanvasBlob = {
  /** Where the bytes are, immutable under this URL. */
  url: string;
  hash: string;
  /** What the server read the container as, never what the file was called. */
  format: "mp4" | "webm";
  byte_size: number;
};

/**
 * Attaches a loop to a track, replacing whatever it carried. The file goes
 * whole, in one request: the route has its own body ceiling and a canvas weighs
 * a few hundred kilobytes, which is why none of the upload machinery applies.
 * Not through `call`, which labels every body as JSON.
 */
export async function placeCanvas(
  trackId: string,
  file: Blob,
  retry = true,
  /** Set only by the attempt a renewal has already paid for. */
  renewed = false,
): Promise<CanvasBlob> {
  const path = `/api/v2/tracks/${trackId}/canvas`;
  // Whatever comes back, what this track carries is no longer what was
  // remembered about it. Forgotten before the request rather than after its
  // success, because a refusal is not proof either: a 404 here can mean
  // another client moved the loop while this one was looking at it.
  forgetCanvas(trackId);
  const headers = new Headers({
    "content-type": file.type || "application/octet-stream",
  });
  if (session) headers.set("authorization", `Bearer ${session.access_token}`);
  // Whose session this attempt speaks for; see the same line in `call`.
  const generation = sessionGeneration;
  const response = await fetch(path, { method: "PUT", headers, body: file });
  if (response.status === 401 && retry && (await renewFor(generation))) {
    return placeCanvas(trackId, file, false, true);
  }
  // The same dead end `call` handles, on the one route that cannot go through
  // it — a body labelled as JSON would be refused here.
  if (response.status === 401 && renewed && generation === sessionGeneration) {
    endSession(session !== null);
  }
  if (!response.ok) throw new ApiError(response.status, `PUT ${path}`);
  const placed = await parse<CanvasBlob>(response);
  // Again, and not redundantly. The call above covers answers older than this
  // placement; this covers one that raced it — a ticket asked for after the
  // first forget and answered before the loop was committed gets a truthful
  // 404, passes the identity check because its entry is current, and files the
  // track as carrying nothing. The loop would then stay invisible until the
  // page was reloaded.
  //
  // `removeCanvas` needs no such pair: only absences are remembered, so a
  // ticket that succeeds mid-removal records nothing, and the next question
  // gets the 404 that has become true.
  forgetCanvas(trackId);
  return placed;
}

/**
 * Takes a track's loop away.
 *
 * Nothing is forgotten here, unlike a placement, and the asymmetry is the
 * point: only *absences* are remembered. A ticket already in flight either
 * succeeds — and a ticket is never cached — or meets the removal and answers
 * 404, which by then is true. Either way the next question is answered
 * correctly, so a forget would be a line no test could tell the absence of.
 */
export const removeCanvas = (trackId: string) =>
  call<void>(`/api/v2/tracks/${trackId}/canvas`, { method: "DELETE" });

/**
 * A URL a `<video>` can play for the loop a track carries, or `null` when it
 * carries none.
 *
 * Minting the ticket checks the link, so it is also how to ask: a track without
 * a canvas answers 404, and so does one the account cannot see — the server
 * does not tell those apart, and neither can this. Any other failure is thrown,
 * because a server that could not answer has not said there is no canvas.
 */
export function canvasUrl(trackId: string): Promise<StreamUrl | null> {
  if (canvaslessTracks.has(trackId)) return Promise.resolve(null);
  const inFlight = canvasTickets.get(trackId);
  if (inFlight) return inFlight;
  const pending: Promise<StreamUrl | null> = mintCanvasTicket(trackId)
    .catch((cause: unknown) => {
      if (cause instanceof ApiError && cause.status === 404) {
        // Only while this is still the request being waited on. Placing a loop
        // drops the entry, and so does a session change — either way this
        // answer is about a track, or an account, that has moved on since it
        // was asked.
        if (canvasTickets.get(trackId) === pending) {
          canvaslessTracks.add(trackId);
        }
        return null;
      }
      throw cause;
    })
    .finally(() => {
      // Never cached, only deduplicated: a live ticket expires, so the entry
      // exists for the moment the request is out and not a moment longer.
      if (canvasTickets.get(trackId) === pending) canvasTickets.delete(trackId);
    });
  canvasTickets.set(trackId, pending);
  return pending;
}

async function mintCanvasTicket(trackId: string): Promise<StreamUrl> {
  const ticket = await call<{ url: string; expires_at: number }>(
    `/api/v2/tracks/${trackId}/canvas-ticket`,
    { method: "POST" },
  );
  return { url: ticket.url, expiresAt: ticket.expires_at };
}

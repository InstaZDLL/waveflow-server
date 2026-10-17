import {
  ApiError,
  call,
  parse,
  renewFor,
  session,
  sessionGeneration,
} from "./client";

/** What the server decided about one offered file. */
export type UploadDecision =
  | "present"
  | "accepted"
  | "unsupported_format"
  | "too_large"
  | "quota_exceeded"
  | "library_closed"
  | "too_many_sessions";

/** An open transfer, as the server accounts for it. */
export type UploadSessionState = {
  session_id: string;
  /** The fragment the server wants next. */
  next_chunk: number;
  received_bytes: number;
  /** The size to send each fragment at, advertised rather than assumed. */
  chunk_bytes: number;
  expires_at: number;
};

export type UploadOffer = {
  full_hash: string;
  size_bytes: number;
  extension: string;
};

/** One verdict, carrying the hash it answers rather than a position. */
export type UploadVerdict = {
  full_hash: string;
  decision: UploadDecision;
  track_id?: string;
  session?: UploadSessionState;
};

export type CommittedUpload = { track_id: string; full_hash: string };

export const negotiateUploads = (libraryId: string, offers: UploadOffer[]) =>
  call<{ verdicts: UploadVerdict[] }>(
    `/api/v2/libraries/${libraryId}/uploads`,
    { method: "POST", body: JSON.stringify({ offers }) },
  ).then(({ verdicts }) => verdicts);

export const getUploadSession = (id: string) =>
  call<UploadSessionState>(`/api/v2/uploads/${id}`);

/**
 * One fragment, as raw bytes. Not through `call`, which labels every body as
 * JSON: this route reads `application/octet-stream`, and carries its own body
 * ceiling — the fragment size — rather than the router's 16 KiB.
 */
export async function putUploadChunk(
  id: string,
  index: number,
  bytes: Blob,
  retry = true,
): Promise<UploadSessionState> {
  const headers = new Headers({ "content-type": "application/octet-stream" });
  if (session) headers.set("authorization", `Bearer ${session.access_token}`);
  // Whose session this attempt speaks for; see `renewFor`.
  const generation = sessionGeneration;
  const response = await fetch(`/api/v2/uploads/${id}/chunks/${index}`, {
    method: "PUT",
    headers,
    body: bytes,
  });
  if (response.status === 401 && retry && (await renewFor(generation))) {
    return putUploadChunk(id, index, bytes, false);
  }
  if (!response.ok) {
    throw new ApiError(
      response.status,
      `PUT /api/v2/uploads/${id}/chunks/${index}`,
    );
  }
  return parse<UploadSessionState>(response);
}

export const commitUpload = (id: string) =>
  call<CommittedUpload>(`/api/v2/uploads/${id}/commit`, { method: "POST" });

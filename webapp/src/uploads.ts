import { blake3 } from "@noble/hashes/blake3.js";
import { bytesToHex } from "@noble/hashes/utils.js";

import type { UploadSessionState } from "./api";

/**
 * The extensions the server's scanner indexes — `AUDIO_EXTENSIONS` in
 * `waveflow-core`. Checked here so that a file the server would refuse is
 * refused before the browser spends the time to fingerprint it. The server
 * still decides: an extension proves nothing, and it reads the file at commit.
 */
export const UPLOAD_EXTENSIONS = [
  "mp3",
  "flac",
  "wav",
  "aiff",
  "aif",
  "ogg",
  "oga",
  "m4a",
  "mp4",
  "aac",
  "dsf",
  "dff",
] as const;

/** The extension a file will be offered with: lowercase, without its dot. */
export function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(dot + 1).toLowerCase();
}

export function isUploadable(name: string): boolean {
  return (UPLOAD_EXTENSIONS as readonly string[]).includes(extensionOf(name));
}

/**
 * The fingerprint the server recomputes at commit: BLAKE3, unkeyed, over the
 * whole file, in lowercase hexadecimal. Any other digest and every commit is
 * refused, since the declared hash must equal the one computed from the bytes.
 *
 * Streamed, because a file may be a gigabyte and must never be held whole.
 */
export async function hashChunks(
  chunks: AsyncIterable<Uint8Array>,
  onProgress?: (read: number) => void,
): Promise<string> {
  const hasher = blake3.create();
  let read = 0;
  for await (const chunk of chunks) {
    hasher.update(chunk);
    read += chunk.byteLength;
    onProgress?.(read);
  }
  return bytesToHex(hasher.digest());
}

/**
 * The byte range the session wants next, or `null` once every byte is in.
 *
 * Exactly `min(remaining, chunk_bytes)` from where the server says it stands:
 * the server refuses a fragment of any other size, because a short one
 * anywhere but the end would shift every later fragment to the wrong offset.
 */
export function nextRange(
  state: UploadSessionState,
  size: number,
): { index: number; start: number; end: number } | null {
  if (state.received_bytes >= size) return null;
  const start = state.received_bytes;
  return {
    index: state.next_chunk,
    start,
    end: Math.min(start + state.chunk_bytes, size),
  };
}

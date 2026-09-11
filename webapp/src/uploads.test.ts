import { describe, expect, it } from "vitest";

import type { UploadSessionState } from "./api";
import { extensionOf, hashChunks, isUploadable, nextRange } from "./uploads";

async function* chunksOf(...parts: Uint8Array[]): AsyncGenerator<Uint8Array> {
  for (const part of parts) yield part;
}

/**
 * The server recomputes BLAKE3 over the whole file and refuses a commit whose
 * declared hash differs. A browser digest that was anything else — another
 * algorithm, a keyed mode, uppercase hex — would fail every upload at the very
 * last step, after the whole transfer.
 */
describe("hashChunks", () => {
  it("is plain BLAKE3: the known digest of the empty input", async () => {
    expect(await hashChunks(chunksOf())).toBe(
      "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
    );
  });

  it("does not depend on where the stream was cut", async () => {
    const bytes = Uint8Array.from({ length: 5000 }, (_, i) => (i * 31) % 256);
    const whole = await hashChunks(chunksOf(bytes));
    const cut = await hashChunks(
      chunksOf(
        bytes.subarray(0, 1),
        bytes.subarray(1, 1024),
        bytes.subarray(1024),
      ),
    );
    expect(cut).toBe(whole);
    expect(whole).toMatch(/^[0-9a-f]{64}$/);
  });

  it("reports how much it has read", async () => {
    const read: number[] = [];
    await hashChunks(chunksOf(new Uint8Array(3), new Uint8Array(4)), (bytes) =>
      read.push(bytes),
    );
    expect(read).toEqual([3, 7]);
  });
});

describe("isUploadable", () => {
  it("accepts the extensions the scanner indexes, whatever their case", () => {
    expect(isUploadable("Army of Me.FLAC")).toBe(true);
    expect(isUploadable("backup.tar.mp3")).toBe(true);
    expect(extensionOf("Army of Me.FLAC")).toBe("flac");
  });

  it("refuses the rest before anything is hashed", () => {
    expect(isUploadable("notes.txt")).toBe(false);
    expect(isUploadable("no-extension")).toBe(false);
    // Deliberately absent from the scanner's list: most AIFC compressions do
    // not decode, so the file would be indexed and fail to play.
    expect(isUploadable("old.aifc")).toBe(false);
  });
});

describe("nextRange", () => {
  const state = (received: number, next: number): UploadSessionState => ({
    session_id: "session-1",
    next_chunk: next,
    received_bytes: received,
    chunk_bytes: 8,
    expires_at: 0,
  });

  it("asks for a whole fragment from where the server stands", () => {
    expect(nextRange(state(8, 1), 20)).toEqual({ index: 1, start: 8, end: 16 });
  });

  it("asks for exactly what remains at the end, and nothing past it", () => {
    expect(nextRange(state(16, 2), 20)).toEqual({
      index: 2,
      start: 16,
      end: 20,
    });
  });

  it("asks for nothing once every byte is in", () => {
    expect(nextRange(state(20, 3), 20)).toBeNull();
  });
});

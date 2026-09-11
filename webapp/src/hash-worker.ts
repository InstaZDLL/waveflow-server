import { hashChunks } from "./uploads";

/**
 * Fingerprints a file off the main thread. Hashing a gigabyte in the page would
 * freeze it for the whole read; here the page keeps drawing and only hears how
 * far the read has got.
 */
export type HashRequest = { id: number; file: File };

export type HashResponse =
  | { id: number; read: number }
  | { id: number; hash: string }
  | { id: number; error: string };

// The DOM library types `self` as a window. A dedicated worker's scope has the
// two members used here, so it is named by them rather than by adding the
// WebWorker library to the types of every file in the client.
const scope = self as unknown as {
  onmessage: ((event: MessageEvent<HashRequest>) => void) | null;
  postMessage: (message: HashResponse) => void;
};

/**
 * Progress is reported at most once per this many bytes. Per chunk would be
 * tens of thousands of messages for a large file, each one a render.
 */
const PROGRESS_STEP = 4 * 1024 * 1024;

async function* chunksOf(file: File): AsyncGenerator<Uint8Array> {
  const reader = file.stream().getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return;
    yield value;
  }
}

scope.onmessage = ({ data: { id, file } }) => {
  let reported = 0;
  hashChunks(chunksOf(file), (read) => {
    if (read - reported < PROGRESS_STEP) return;
    reported = read;
    scope.postMessage({ id, read });
  }).then(
    (hash) => scope.postMessage({ id, hash }),
    (error: unknown) =>
      scope.postMessage({
        id,
        error: error instanceof Error ? error.message : String(error),
      }),
  );
};

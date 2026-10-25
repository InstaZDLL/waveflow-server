import { call, renewFor, session, sessionGeneration } from "./client";

export type Library = {
  id: string;
  name: string;
  visibility: "private" | "shared";
  role: "owner" | "manager" | "listener";
  last_scan_started_at: number | null;
  last_scan_completed_at: number | null;
  /**
   * Whether the library takes files at all: the operator's decision, never a
   * member's. The role says who may upload; this says whether anyone can.
   */
  accepts_uploads: boolean;
  /**
   * Whether the library takes canvases: a door of its own, not the upload one.
   * A library closed to files may be open to loops, and the reverse.
   */
  accepts_canvas: boolean;
  /**
   * Whether the scan follows symbolic links that stay inside the library
   * (RFC-021). An administrator's setting; shown to every member because it
   * explains what the library holds.
   */
  follow_links: boolean;
};

export const listLibraries = () => call<Library[]>("/api/v2/libraries");

/** A library as an administrator manages it, its root on the server included. */
export type LibrarySettings = {
  id: string;
  name: string;
  path: string;
  follow_links: boolean;
};

export const getLibrarySettings = (libraryId: string) =>
  call<LibrarySettings>(`/api/v2/libraries/${libraryId}/settings`);

/**
 * Renames a library, moves it, or changes whether its scan follows links. A
 * name is its owner's to change; the path and the links setting are an
 * administrator's. A new path or links setting queues a scan.
 */
export const updateLibrary = (
  libraryId: string,
  update: { name?: string; path?: string; follow_links?: boolean },
) =>
  call<{ scan_id: string | null }>(`/api/v2/libraries/${libraryId}`, {
    method: "PATCH",
    body: JSON.stringify(update),
  });

/**
 * Removes a library from the catalogue, for every member, and nothing from the
 * disk. `confirm` must be its current name, or nothing is deleted.
 */
export const deleteLibrary = (libraryId: string, confirm: string) =>
  call<{ tracks: number; albums: number; artists: number }>(
    `/api/v2/libraries/${libraryId}`,
    { method: "DELETE", body: JSON.stringify({ confirm }) },
  );

/** A track whose file the scans can no longer find, with what a purge costs. */
export type UnavailableTrack = {
  id: string;
  relative_path: string;
  title: string;
  artist: string | null;
  album: string | null;
  duration_ms: number;
  unavailable_since: number;
  playlist_entries: number;
  favorites: number;
  plays: number;
};

export const UNAVAILABLE_PAGE = 100;

export const listUnavailableTracks = (
  libraryId: string,
  offset: number,
  limit: number,
  signal?: AbortSignal,
) =>
  call<{ total: number; tracks: UnavailableTrack[] }>(
    `/api/v2/libraries/${libraryId}/unavailable-tracks?offset=${offset}&limit=${limit}`,
    { signal },
  );

/** Purges named missing tracks, or all of them. Never neither. */
export const purgeUnavailableTracks = (
  libraryId: string,
  selection: { track_ids: string[] } | { all: true },
) =>
  call<{ purged: number }>(
    `/api/v2/libraries/${libraryId}/unavailable-tracks/purge`,
    { method: "POST", body: JSON.stringify(selection) },
  );

export const addLibrary = (
  name: string,
  path: string,
  visibility: "private" | "shared",
) =>
  call<{ library_id: string; scan_id: string }>("/api/v2/libraries", {
    method: "POST",
    body: JSON.stringify({ name, path, visibility }),
  });

export type ScanJob = {
  id: string;
  library_id: string;
  status: string;
  total_files: number;
  processed_files: number;
  added: number;
  updated: number;
  moved: number;
  skipped: number;
  unavailable: number;
  errors: number;
  /** Entries the scan left out: built-in exclusions and `.waveflowignore`. */
  excluded: number;
  /** Symbolic links the scan did not follow. */
  skipped_links: number;
  current_path: string | null;
  message: string | null;
};

/** Fetch the latest snapshot of a scan job. */
export const getScan = (scanId: string) =>
  call<ScanJob>(`/api/v2/scans/${scanId}`);

/**
 * Scan progress, read as it happens.
 *
 * `EventSource` cannot do this. The route authenticates on a bearer token —
 * `authenticated()` reads the `Authorization` header and there is no cookie
 * fallback — and `EventSource` sends no headers at all. That is the same wall
 * `<audio src>` hits, which stream tickets exist to get around; here the way
 * through is `fetch`, whose response body can be read as it arrives.
 *
 * Returns a function that stops the read. The server sends a `snapshot` event
 * first and then a `progress` event per step, and both carry a whole
 * `ScanJob`, so a caller only ever has to replace what it holds.
 */
export function watchScan(
  scanId: string,
  onJob: (job: ScanJob) => void,
  onFailure?: () => void,
): () => void {
  const controller = new AbortController();
  void (async () => {
    try {
      const open = async () => {
        const headers = new Headers({ accept: "text/event-stream" });
        if (session) {
          headers.set("authorization", `Bearer ${session.access_token}`);
        }
        return fetch(`/api/v2/scans/${scanId}/events`, {
          headers,
          signal: controller.signal,
        });
      };
      // This route is opened by hand rather than through `call`, so it has to
      // repeat what `call` does about an expired access token. Without it an
      // admin who left the tab open watched "waiting for the first reading"
      // for as long as the scan took, and then for ever.
      // Whose session this stream speaks for; see `renewFor`. This route is
      // the fifth of the kind and was not named in the issue.
      const generation = sessionGeneration;
      let response = await open();
      if (response.status === 401 && (await renewFor(generation))) {
        response = await open();
      }
      if (!response.ok || !response.body) {
        onFailure?.();
        return;
      }
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        const drained = drainEventStream(
          buffer + decoder.decode(value, { stream: true }),
        );
        buffer = drained.rest;
        for (const payload of drained.data) {
          try {
            onJob(JSON.parse(payload) as ScanJob);
          } catch {
            // A frame that is not a job is a keep-alive or a comment.
          }
        }
      }
    } catch {
      // Aborting is the ordinary way this ends — the panel is going away, and
      // there is nobody left to tell. Anything else is the stream failing, and
      // a network error is the likeliest way it does: without this the panel
      // waited on a first reading that no longer had a chance of arriving.
      if (!controller.signal.aborted) onFailure?.();
    }
  })();
  return () => controller.abort();
}

/**
 * Splits an accumulated `text/event-stream` buffer into the `data:` payloads of
 * its complete frames, and hands back whatever was left mid-frame.
 *
 * A read never lands on a frame boundary, so the tail has to survive until the
 * next chunk arrives; dropping it loses an event, and returning it as complete
 * feeds half a JSON object to the parser. Multi-line `data:` is concatenated,
 * which is what the specification says and what a long path would produce.
 */
export function drainEventStream(buffer: string): {
  data: string[];
  rest: string;
} {
  const data: string[] = [];
  let rest = buffer;
  let split = rest.indexOf("\n\n");
  while (split !== -1) {
    const frame = rest.slice(0, split);
    rest = rest.slice(split + 2);
    const payload = frame
      .split("\n")
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice(5).trimStart())
      // Newline, not empty: the specification joins consecutive `data:` lines
      // with a line feed, and a payload split across them is reassembled with
      // it. JSON treats the break as whitespace, so the parse is unaffected —
      // but silently dropping it would corrupt any payload that is not JSON.
      .join("\n");
    if (payload) data.push(payload);
    split = rest.indexOf("\n\n");
  }
  return { data, rest };
}

/** List every API token issued to an account. */
export type LibraryMember = {
  user_id: string;
  username: string;
  role: "owner" | "manager" | "listener";
  created_at: number;
};

/**
 * Who may see a library. The write side has existed since M4 with nothing to
 * read it back, so a screen could grant and revoke without showing who already
 * had access — and no client can work that out for itself, since an account's
 * own membership says nothing about anyone else's.
 */
export const listLibraryMembers = (libraryId: string) =>
  call<LibraryMember[]>(`/api/v2/libraries/${libraryId}/members`);

/** `owner` cannot be granted: the route refuses it. */
export const setLibraryMember = (
  libraryId: string,
  userId: string,
  role: "manager" | "listener",
) =>
  call<void>(`/api/v2/libraries/${libraryId}/members/${userId}`, {
    method: "PUT",
    body: JSON.stringify({ role }),
  });

export const removeLibraryMember = (libraryId: string, userId: string) =>
  call<void>(`/api/v2/libraries/${libraryId}/members/${userId}`, {
    method: "DELETE",
  });

export const startScan = (libraryId: string) =>
  call<{ scan_id: string }>(`/api/v2/libraries/${libraryId}/scans`, {
    method: "POST",
  });

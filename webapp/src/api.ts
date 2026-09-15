/**
 * Thin client over /api/v2.
 *
 * Access tokens are short-lived and live in memory only. The server keeps the
 * rotating refresh token in an HttpOnly, SameSite cookie; refresh and logout
 * additionally require a double-submit CSRF value.
 */

export type SessionUser = {
  id: string;
  username: string;
  role: "admin" | "user";
};

export type WebSession = {
  access_token: string;
  user: SessionUser;
  device_id: string;
};

let session: WebSession | null = null;

// M4 originally persisted both tokens. Remove those legacy entries on upgrade
// so an old rotating refresh token is not left readable by JavaScript.
try {
  localStorage.removeItem("waveflow.access");
  localStorage.removeItem("waveflow.refresh");
} catch {
  // Storage may be disabled; sessions do not depend on it anymore.
}

export type Album = {
  id: string;
  library_id: string;
  title: string;
  artist: string | null;
  artist_id: string | null;
  artwork_hash: string | null;
  year: number | null;
  starred_at: number | null;
  user_rating: number | null;
};

export type Song = {
  id: string;
  library_id: string;
  album_id: string | null;
  title: string;
  album: string | null;
  artist: string | null;
  artist_id: string | null;
  artwork_hash: string | null;
  duration_ms: number;
  track: number | null;
  disc: number | null;
  starred_at: number | null;
  user_rating: number | null;
};

export type AlbumDetail = Album & { songs: Song[] };

export type Artist = {
  id: string;
  name: string;
  album_count: number;
  artwork_hash: string | null;
};

export type ArtistDetail = Artist & { albums: Album[] };

export type SearchResult = {
  artists: Artist[];
  albums: Album[];
  songs: Song[];
};

export type Playlist = {
  id: string;
  name: string;
  comment: string | null;
  public: boolean;
  created_at: number;
  updated_at: number;
  songs: Song[];
};

export type Favorite = {
  entity_type: string;
  entity_id: string;
  starred_at: number;
};

export type Queue = {
  current: string | null;
  position_ms: number;
  changed_by: string | null;
  updated_at: number;
  songs: Song[];
};

export type Share = {
  id: string;
  url?: string;
  description: string | null;
  expires_at: number | null;
  created_at: number;
  visit_count: number;
  track_ids: string[];
};

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
};

export type User = {
  id: string;
  username: string;
  role: "admin" | "user";
  disabled: boolean;
  has_subsonic_credential: boolean;
  folder_ids: string[];
};

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

export function hasSession(): boolean {
  return session !== null;
}

export function currentUser(): SessionUser | null {
  return session?.user ?? null;
}

async function parse<T>(response: Response): Promise<T> {
  if (response.status === 204) return undefined as T;
  const text = await response.text();
  return text ? (JSON.parse(text) as T) : (undefined as T);
}

/**
 * In-flight refresh, shared by every caller.
 *
 * Refresh tokens rotate, so two concurrent 401s each starting their own refresh
 * would spend the same token twice: the second call presents one the server has
 * already retired and the whole session is dropped. Callers await one operation
 * instead.
 */
let pendingRefresh: {
  generation: number;
  result: Promise<boolean>;
} | null = null;
const artworkUrls = new Map<string, Promise<string | null>>();

/**
 * The same answers, once they have arrived, readable without awaiting.
 *
 * The map above holds promises, so nothing in it can be read during a render —
 * and a cover component therefore started every mount with no image and put the
 * grey placeholder on screen for a frame, even for a cover whose bytes were
 * already in memory. Leaving a page and coming back flashed the whole grid.
 */
const settledArtworkUrls = new Map<string, string | null>();

/** What is already held for this artwork, or `null` if nothing is yet. */
export function cachedArtworkUrl(id: string | null): string | null {
  return id ? (settledArtworkUrls.get(id) ?? null) : null;
}

function clearArtworkUrls(): void {
  for (const pending of artworkUrls.values()) {
    void pending.then((url) => url && URL.revokeObjectURL(url));
  }
  artworkUrls.clear();
  settledArtworkUrls.clear();
}

/**
 * Tracks the server has already said carry no loop.
 *
 * Minting a ticket is how the question is asked, and a track without a canvas
 * answers 404 to it. That answer is stable for as long as nothing here changes
 * the track's canvas — so asking again learns nothing, and asking again is
 * exactly what happened: the playing screen asks from an effect on every
 * mount, and the editor's panel holds a query deliberately kept uncached,
 * because a live ticket expires and must never be served from memory. Both
 * were right about the ticket and wrong about its absence, and a library of
 * ordinary tracks therefore produced a 404 per navigation, for ever.
 *
 * Kept here rather than in either caller, so one memory answers both. Emptied
 * when a session ends, with everything else that belongs to whoever was
 * signed in.
 *
 * What this deliberately does not see: a canvas placed by *another* client
 * during this session. Placing or removing one here forgets the track, so the
 * screen that did it reads back the truth; elsewhere, the page has to be
 * reloaded. That is the same bargain the artwork above already strikes, and it
 * is worth saying rather than discovering.
 */
const canvaslessTracks = new Set<string>();

/**
 * Tickets being minted right now, one entry per track.
 *
 * The same shape as `artworkUrls` above, and it earns its place twice over.
 * `StrictMode` invokes an effect twice on mount, so the playing screen asked
 * for every loop in duplicate throughout development; and the entry doubles as
 * the proof that an answer still belongs to the question. Recording an absence
 * happens only while the entry is still this request's — a session change
 * empties the map, and so does placing a loop — so an answer that outlived
 * either cannot be filed.
 *
 * That is the whole guard. It replaced a per-track counter that said the same
 * thing less directly, and it says the session half too: a 404 means "no loop
 * **or** none you may see", so keeping one account's refusal for the next
 * would hide a canvas the next account can read.
 */
const canvasTickets = new Map<string, Promise<StreamUrl | null>>();

/** Ask the server again about this track's loop, whatever it said before. */
function forgetCanvas(trackId: string): void {
  canvaslessTracks.delete(trackId);
  canvasTickets.delete(trackId);
}

/**
 * Things holding answers that belong to whoever is signed in.
 *
 * The object URLs above were the only one of these for a long time, and
 * clearing them was written inline at each of the three moments a session
 * begins or ends. There is a query cache now — albums, playlists, favourites,
 * an administrator's list of accounts — and it lives outside this module, so
 * it registers here instead of being reached for. Signing out and signing in
 * as somebody else on a shared browser must not show them the last person's
 * library, and a client-side navigation is all that happens between the two:
 * the document is never reloaded, so nothing is forgotten on its own.
 */
const sessionScoped: Array<() => void> = [];

export function forgetOnSessionChange(forget: () => void): void {
  sessionScoped.push(forget);
}

/**
 * Which session the answers below belong to.
 *
 * Bumped every time one begins or ends, so work already in flight can tell
 * whether it still speaks for the account that started it. A refresh takes a
 * round trip, and a sign-out during that round trip used to be undone by it:
 * `logout` set the session to null, the answer arrived afterwards, and the
 * assignment put the previous account's token straight back. The visitor was
 * on the sign-in screen and still authenticated.
 */
let sessionGeneration = 0;

function clearSessionState(): void {
  sessionGeneration += 1;
  clearArtworkUrls();
  canvaslessTracks.clear();
  canvasTickets.clear();
  for (const forget of sessionScoped) forget();
}

function refresh(): Promise<boolean> {
  // Shared only with callers of the same session. A renewal that outlived the
  // account it was started for now answers `false` on purpose, and handing that
  // answer to somebody who asked after a new session began would fail their
  // request for a reason that no longer applies.
  if (pendingRefresh && pendingRefresh.generation === sessionGeneration) {
    return pendingRefresh.result;
  }
  const attempt: { generation: number; result: Promise<boolean> } = {
    generation: sessionGeneration,
    result: performRefresh().finally(() => {
      // Only if nothing newer has taken its place: clearing unconditionally
      // would throw away a renewal that is still out.
      if (pendingRefresh === attempt) pendingRefresh = null;
    }),
  };
  pendingRefresh = attempt;
  return attempt.result;
}

async function performRefresh(): Promise<boolean> {
  const hadSession = session !== null;
  // Whose session this renewal speaks for. Read before the round trip and
  // checked after it: anything that began or ended a session in between makes
  // this answer somebody else's, and it is then worth nothing — neither its
  // token, which would undo a sign-out, nor its failure, which would sign out
  // whoever signed in while it was away.
  const generation = sessionGeneration;
  const stale = () => generation !== sessionGeneration;
  const csrf = cookieValue("waveflow-csrf");
  if (!csrf) {
    endSession(hadSession);
    return false;
  }
  try {
    const response = await fetch("/api/v2/web/auth/refresh", {
      method: "POST",
      headers: { "x-waveflow-csrf": csrf },
    });
    if (stale()) return false;
    if (!response.ok) {
      endSession(hadSession);
      return false;
    }
    const renewed = await parse<WebSession>(response);
    if (stale()) return false;
    session = renewed;
    return true;
  } catch {
    if (stale()) return false;
    endSession(hadSession);
    return false;
  }
}

/**
 * The session is over, and nothing on screen may go on acting as though it is
 * not.
 *
 * Reached from two places. A renewal that failed is the obvious one. The other
 * is a 401 that *survived* a renewal: `call` renews once and asks again, and
 * when the second answer is 401 too there is nothing left to try — the server
 * spends that status on `InvalidCredentials` and `InvalidRefreshToken` alone,
 * and refuses an authorisation with 403 or by blurring it into 404. It used to
 * throw and stop there, which is how a poll outlived its own session: a query
 * on `refetchInterval` keeps its schedule through an error, so `now-playing`
 * asked again every thirty seconds, indefinitely, while the screen sat on a
 * library it could no longer read and the visitor was never told.
 *
 * `hadSession` is read before the round trip by the renewal path, because a
 * sign-out during that round trip must not be reported as an expiry.
 */
function endSession(hadSession: boolean): void {
  session = null;
  // The redirect below reloads the document and would clear this anyway —
  // except when it does not fire, because the visitor is already on the
  // sign-in screen.
  clearSessionState();
  if (hadSession && window.location.pathname !== "/login") {
    window.location.assign("/login");
  }
}

export async function ensureSession(): Promise<boolean> {
  return hasSession() || refresh();
}

async function call<T>(
  path: string,
  init: RequestInit = {},
  retry = true,
  /** Set only by the attempt a renewal has already paid for. */
  renewed = false,
): Promise<T> {
  const headers = new Headers(init.headers);
  if (session) headers.set("authorization", `Bearer ${session.access_token}`);
  if (init.body) headers.set("content-type", "application/json");
  // Whose session this attempt speaks for, read before the round trip and
  // checked after it — the same guard `performRefresh` already keeps, for the
  // same reason. A signing-out and signing-in fit inside one round trip, and
  // ending a session on a refusal addressed to the account before it would put
  // whoever just arrived back on the sign-in screen.
  const generation = sessionGeneration;
  const response = await fetch(path, { ...init, headers });
  if (response.status === 401 && retry && (await refresh())) {
    return call<T>(path, init, false, true);
  }
  // Refused again, on the attempt that a renewal had already paid for. A
  // failed renewal has ended the session itself by now; this is the other
  // way out of the same dead end, and without it the caller only got a
  // rejected promise to ignore.
  //
  // Read from `renewed` and not from `retry`: they are not the same question.
  // `setupRequired` asks with the retry switched off because it runs before
  // anyone is signed in and has no session to renew — a 401 there is not a
  // session ending, and saying so would be wrong even where it is harmless.
  if (response.status === 401 && renewed && generation === sessionGeneration) {
    endSession(session !== null);
  }
  if (!response.ok) {
    throw new ApiError(response.status, `${init.method ?? "GET"} ${path}`);
  }
  return parse<T>(response);
}

export async function login(username: string, password: string): Promise<void> {
  const response = await fetch("/api/v2/web/auth/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      username,
      password,
      device_name: "WaveFlow Web",
    }),
  });
  if (!response.ok) {
    throw new ApiError(response.status, "login failed");
  }
  session = await parse<WebSession>(response);
  clearSessionState();
}

export const setupRequired = () =>
  call<{ required: boolean }>("/api/v2/setup", {}, false).then(
    (status) => status.required,
  );

export async function bootstrapAdmin(
  username: string,
  password: string,
): Promise<void> {
  const response = await fetch("/api/v2/setup", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username, password }),
  });
  if (!response.ok) {
    throw new ApiError(response.status, "setup failed");
  }
}

export async function logout(): Promise<void> {
  try {
    const csrf = cookieValue("waveflow-csrf");
    await call<void>("/api/v2/web/auth/logout", {
      method: "POST",
      headers: csrf ? { "x-waveflow-csrf": csrf } : undefined,
    });
  } finally {
    session = null;
    clearSessionState();
  }
}

function cookieValue(name: string): string | null {
  const prefix = `${name}=`;
  for (const part of document.cookie.split(";")) {
    const value = part.trim();
    if (value.startsWith(prefix)) return value.slice(prefix.length);
  }
  return null;
}

/** Walks the paged endpoint to completion; the server caps a page at 500. */
async function collect<T>(
  path: string,
  params: Record<string, string> = {},
): Promise<T[]> {
  const pageSize = 500;
  const all: T[] = [];
  for (let offset = 0; ; offset += pageSize) {
    const query = new URLSearchParams({
      ...params,
      limit: String(pageSize),
      offset: String(offset),
    });
    const page = await call<T[]>(`${path}?${query}`);
    all.push(...page);
    if (page.length < pageSize) return all;
  }
}

/**
 * The orders `GET /api/v2/albums` accepts, spelled as the server does — the
 * vocabulary is `AlbumOrder` in `src/services/mod.rs`, shared with the Subsonic
 * `type` parameter.
 *
 * Four of these also *filter*: `frequent`, `recent` and `starred` answer only
 * the albums that have a play count, a last play or a star, and `byYear` only
 * those carrying a year. The header count is read off the response for that
 * reason, rather than assumed to be the size of the catalogue.
 *
 * `random` is deliberately absent. It is `ORDER BY RANDOM()` under a `LIMIT`,
 * so each page of `collect` is an independent draw and the assembled list would
 * both repeat and omit albums past the first page.
 */
export type AlbumSort =
  | "alphabeticalByName"
  | "alphabeticalByArtist"
  | "newest"
  | "recent"
  | "frequent"
  | "starred"
  | "byYear";

/**
 * Catalogue calls take the active library, search included since 2026-09-14.
 *
 * It used to be the one that could not, and the screen said so. The reason
 * given was that scoping meant rewriting three FTS queries in a service the
 * frozen Subsonic façade shares — true of the queries, wrong about the
 * sharing: `search2`/`search3` reach a separate method with its own three,
 * and what the two surfaces share is the index and the projections.
 */
function scoped(
  libraryId: string | undefined,
  extra: Record<string, string> = {},
): Record<string, string> {
  return libraryId ? { ...extra, library_id: libraryId } : extra;
}

/** Albums in the requested order, scoped to `libraryId` when one is given. */
export const listAlbums = (sort?: AlbumSort, libraryId?: string) =>
  collect<Album>("/api/v2/albums", scoped(libraryId, sort ? { sort } : {}));
export const getAlbum = (id: string) =>
  call<AlbumDetail>(`/api/v2/albums/${id}`);
/** Artists, scoped to `libraryId` when one is given. */
export const listArtists = (libraryId?: string) =>
  collect<Artist>("/api/v2/artists", scoped(libraryId));
export const getArtist = (id: string) =>
  call<ArtistDetail>(`/api/v2/artists/${id}`);
/** Search, scoped to `libraryId` when one is given. */
export const search = (query: string, libraryId?: string) =>
  call<SearchResult>(
    `/api/v2/search?${new URLSearchParams(scoped(libraryId, { q: query }))}`,
  );
export const getTrack = (id: string) => call<Song>(`/api/v2/tracks/${id}`);

/**
 * A track with the credits the catalogue answers for it. The server sends these
 * on every `SongItem`; `Song` leaves them out because no listing reads them, and
 * only the tag editor needs them.
 */
export type SongCredits = Song & {
  year?: number | null;
  artists?: { id: string; name: string }[];
  genres?: string[];
};

export const getTrackCredits = (id: string) =>
  call<SongCredits>(`/api/v2/tracks/${id}`);

/**
 * What the file's tags say, as the last scan read them. Artists and genres are
 * absent on purpose: a correction to either replaces the rows the scan wrote,
 * so the file's own credits are no longer in the database to be shown.
 */
export type TrackSourceTags = {
  title: string;
  sort_title: string | null;
  year: number | null;
  track_number: number | null;
  disc_number: number | null;
  musicbrainz_recording_id: string | null;
  comment: string | null;
};

/** The stored correction, field by field. `null` is no correction there. */
export type TrackOverrideValues = {
  title: string | null;
  sort_title: string | null;
  year: number | null;
  track_number: number | null;
  disc_number: number | null;
  musicbrainz_recording_id: string | null;
  comment: string | null;
  artists: string[] | null;
  genres: string[] | null;
};

export type TrackOverrides = {
  source: TrackSourceTags;
  overrides: TrackOverrideValues;
};

/**
 * A partial patch in three states. A key **absent** from the object leaves that
 * correction as it is, `null` removes it, and a value sets it — so a key must
 * only be present when it is meant, and never as `undefined`: `JSON.stringify`
 * would drop it, which happens to read as absent, but by accident.
 */
export type TrackCorrectionPatch = {
  [Field in keyof TrackOverrideValues]?: TrackOverrideValues[Field];
};

export const getTrackOverrides = (id: string) =>
  call<TrackOverrides>(`/api/v2/tracks/${id}/overrides`);

export const correctTrack = (id: string, patch: TrackCorrectionPatch) =>
  call<SongCredits>(`/api/v2/tracks/${id}`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });

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
  const response = await fetch(`/api/v2/uploads/${id}/chunks/${index}`, {
    method: "PUT",
    headers,
    body: bytes,
  });
  if (response.status === 401 && retry && (await refresh())) {
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
  if (response.status === 401 && retry && (await refresh())) {
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

async function loadArtworkUrl(
  id: string,
  retry = true,
): Promise<string | null> {
  const headers = new Headers();
  if (session) headers.set("authorization", `Bearer ${session.access_token}`);
  const response = await fetch(`/api/v2/artwork/${encodeURIComponent(id)}`, {
    headers,
  });
  if (response.status === 401 && retry && (await refresh())) {
    return loadArtworkUrl(id, false);
  }
  // The same distinction `canvasUrl` makes above, and for the same reason: a
  // 404 is the server saying it holds no such artwork, which is an answer; any
  // other failure is a server that could not answer, which is not one. They
  // collapsed into the same `null` here and were then held for the session, so
  // a single blip left a cover grey until the tab was reloaded.
  //
  // An id here is a content hash — every caller passes `artwork_hash` — so a
  // 404 is about immutable content and is worth keeping. Re-asking it on every
  // mount would put back the request storm lazy loading exists to prevent.
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new ApiError(response.status, `GET /api/v2/artwork/${id}`);
  }
  return URL.createObjectURL(await response.blob());
}

/** Resolve authenticated artwork to a browser-safe object URL for <img>. */
export function artworkUrl(id: string | null): Promise<string | null> {
  if (!id) return Promise.resolve(null);
  const cached = artworkUrls.get(id);
  if (cached) return cached;
  const pending: Promise<string | null> = loadArtworkUrl(id).then(
    (url) => {
      // Only while this is still the request being waited on. A session change
      // empties both maps and revokes the object URLs, and a load started for
      // the account that left would otherwise land here afterwards — putting a
      // revoked URL back under a key the next account reads.
      if (artworkUrls.get(id) === pending) settledArtworkUrls.set(id, url);
      return url;
    },
    () => {
      // Not an answer, so nothing is remembered: the entry is dropped and the
      // next mount asks again. `null` is returned rather than rethrown because
      // a cover that could not be fetched is a placeholder, not a broken page.
      if (artworkUrls.get(id) === pending) artworkUrls.delete(id);
      return null;
    },
  );
  artworkUrls.set(id, pending);
  return pending;
}

export const listPlaylists = () => call<Playlist[]>("/api/v2/playlists");
export const createPlaylist = (name: string, trackIds: string[] = []) =>
  call<Playlist>("/api/v2/playlists", {
    method: "POST",
    body: JSON.stringify({ name, track_ids: trackIds }),
  });
export const deletePlaylist = (id: string) =>
  call<void>(`/api/v2/playlists/${id}`, { method: "DELETE" });
export const appendToPlaylist = (id: string, trackIds: string[]) =>
  call<Playlist>(`/api/v2/playlists/${id}`, {
    method: "PATCH",
    body: JSON.stringify({ add: trackIds }),
  });

export const listFavorites = () => call<Favorite[]>("/api/v2/favorites");
export const getQueue = () => call<Queue | null>("/api/v2/queue");
export const saveQueue = (
  songs: Song[],
  current: string | null,
  positionMs: number,
) =>
  call<void>("/api/v2/queue", {
    method: "PUT",
    body: JSON.stringify({
      track_ids: songs.map((song) => song.id),
      current,
      position_ms: positionMs,
      client: "WaveFlow Web",
    }),
  });

export const listShares = () => call<Share[]>("/api/v2/shares");
export const createShare = (trackIds: string[], description: string) =>
  call<Share>("/api/v2/shares", {
    method: "POST",
    body: JSON.stringify({ track_ids: trackIds, description }),
  });
export const deleteShare = (id: string) =>
  call<void>(`/api/v2/shares/${id}`, { method: "DELETE" });

export const listLibraries = () => call<Library[]>("/api/v2/libraries");
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
      let response = await open();
      if (response.status === 401 && (await refresh())) {
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

export type NowPlaying = {
  username: string;
  song: Song;
  started_at: number;
};

/** List the playback sessions currently reported by the server. */
export const listNowPlaying = () => call<NowPlaying[]>("/api/v2/now-playing");

export type ApiToken = {
  id: string;
  name: string;
  scopes: string[];
  expires_at: number | null;
  created_at: number;
  last_used_at: number | null;
  revoked_at: number | null;
};

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

export const listApiTokens = (username: string) =>
  call<ApiToken[]>(
    `/api/v2/admin/users/${encodeURIComponent(username)}/tokens`,
  );

/** The secret comes back once and is never recoverable: only its hash is kept. */
export const createApiToken = (username: string, name: string) =>
  call<ApiToken & { secret: string }>(
    `/api/v2/admin/users/${encodeURIComponent(username)}/tokens`,
    { method: "POST", body: JSON.stringify({ name, scopes: [] }) },
  );

/** Revoke an API token belonging to an account. */
export const revokeApiToken = (username: string, tokenId: string) =>
  call<void>(
    `/api/v2/admin/users/${encodeURIComponent(username)}/tokens/${tokenId}`,
    { method: "DELETE" },
  );

export const startScan = (libraryId: string) =>
  call<{ scan_id: string }>(`/api/v2/libraries/${libraryId}/scans`, {
    method: "POST",
  });

export const listUsers = () => call<User[]>("/api/v2/admin/users");
export const createUser = (
  username: string,
  webPassword: string,
  role: "admin" | "user",
) =>
  call<User>("/api/v2/admin/users", {
    method: "POST",
    body: JSON.stringify({ username, web_password: webPassword, role }),
  });
export const setUserDisabled = (username: string, disabled: boolean) =>
  call<User>(`/api/v2/admin/users/${encodeURIComponent(username)}`, {
    method: "PATCH",
    body: JSON.stringify({ disabled }),
  });
export const setSubsonicCredential = (username: string, password: string) =>
  call<{ api_key: string }>(
    `/api/v2/admin/users/${encodeURIComponent(username)}/subsonic-credential`,
    { method: "PUT", body: JSON.stringify({ password }) },
  );

export const setFavorite = (kind: string, id: string, on: boolean) =>
  call<void>(`/api/v2/favorites/${kind}/${id}`, {
    method: on ? "PUT" : "DELETE",
  });

/** The three things a star, a rating or a bookmark can hang off. */
export type EntityKind = "track" | "album" | "artist";

export type Rating = {
  entity_type: string;
  entity_id: string;
  rating: number;
  updated_at: number;
};

export const listRatings = () => call<Rating[]>("/api/v2/ratings");

/** 1 to 5 stars; 0 clears the rating, which is the server's own convention. */
export const setRating = (kind: EntityKind, id: string, rating: number) =>
  call<void>(`/api/v2/ratings/${kind}/${id}`, {
    method: "PUT",
    body: JSON.stringify({ rating }),
  });

export type LyricsLine = { start?: number; value: string };

export type StructuredLyrics = {
  displayArtist: string | null;
  displayTitle: string;
  lang: string;
  synced: boolean;
  line: LyricsLine[];
};

/**
 * What `GET /api/v2/tracks/{id}/lyrics` answers.
 *
 * `camelCase`, because `src/lyrics.rs` renames the whole struct that way — the
 * envelope as much as the `StructuredLyrics` it carries. This declared
 * `track_id`/`structured_lyrics` until 2026-09-15, so every read of it threw on
 * the first property and the playing page was unreachable. `StructuredLyrics`
 * above was already right: the inside was converted and the envelope forgotten.
 */
export type LyricsList = {
  trackId: string;
  structuredLyrics: StructuredLyrics[];
};

export const getLyrics = (trackId: string) =>
  call<LyricsList>(`/api/v2/tracks/${trackId}/lyrics`);

export type Bookmark = {
  position_ms: number;
  comment: string | null;
  created_at: number;
  updated_at: number;
  song: Song;
};

export const listBookmarks = () => call<Bookmark[]>("/api/v2/bookmarks");

/**
 * One bookmark per account and track, so this replaces rather than adds — the
 * route is `PUT` for that reason, and sending the same position twice leaves
 * the same single bookmark.
 */
export const setBookmark = (
  trackId: string,
  positionMs: number,
  comment?: string,
) =>
  call<void>(`/api/v2/bookmarks/${trackId}`, {
    method: "PUT",
    body: JSON.stringify({
      position_ms: Math.max(0, Math.round(positionMs)),
      comment: comment ?? null,
    }),
  });

export const deleteBookmark = (trackId: string) =>
  call<void>(`/api/v2/bookmarks/${trackId}`, { method: "DELETE" });

export type Genre = {
  name: string;
  song_count: number;
  album_count: number;
};

/** Genre summaries, scoped to `libraryId` when one is given. */
export const listGenres = (libraryId?: string) =>
  call<Genre[]>(`/api/v2/genres?${new URLSearchParams(scoped(libraryId))}`);

/** Songs of one genre, scoped to `libraryId` when one is given. */
export const listGenreSongs = (genre: string, libraryId?: string) =>
  collect<Song>("/api/v2/songs/by-genre", scoped(libraryId, { genre }));

/**
 * `GET /history` answers plays, not songs — `track_id`, `submission` and
 * `played_at` — so a screen wanting titles resolves them itself. The default
 * limit is the server's 200; the cap is `MAX_SYNC_LIMIT`.
 */
export type Play = {
  track_id: string;
  submission: boolean;
  played_at: number;
};

export const listHistory = (limit = 100) =>
  call<Play[]>(`/api/v2/history?limit=${limit}`);

/** Request a random selection of songs, optionally scoped by genre and library. */
export const listRandomSongs = (
  limit = 100,
  genre?: string,
  libraryId?: string,
) => {
  const query = new URLSearchParams(
    scoped(libraryId, { limit: String(limit) }),
  );
  if (genre) query.set("genre", genre);
  return call<Song[]>(`/api/v2/songs/random?${query}`);
};

export const scrobble = (trackId: string, submission: boolean) =>
  call<void>("/api/v2/scrobbles", {
    method: "POST",
    body: JSON.stringify({ track_id: trackId, submission }),
  });

/**
 * External scrobbling — RFC-010.
 *
 * The operator names instances and the account picks a name: no call here
 * sends a URL, and no answer carries one. `provider` is the recipient
 * (`listenbrainz`, `maloja`, `lastfm`) and `destination` is which of its
 * instances this server declares; the pair names every link and every route.
 */
export type ScrobbleProvider = "listenbrainz" | "maloja" | "lastfm";

/**
 * The cases this build has been taught to word.
 *
 * Exhaustive over what the server sends *today*, which is what a screen's
 * translation table must cover.
 */
export type KnownScrobbleUnavailable =
  | "no_application_configured"
  | "browser_journey_needs_https";

/**
 * Why a declared instance cannot be linked — a case, not a sentence.
 *
 * The server sent its own English prose here until 2026-09-15, which this
 * client printed verbatim: a French reader was told in English what to change
 * in a configuration file. The wording belongs to whoever is doing the
 * telling, so the wire carries only the case.
 *
 * Open on purpose. A server is free to be newer than the client reading it, so
 * a type naming only today's cases would be the same untruth this file has
 * just been cleared of: an assertion about the wire that the wire never made.
 * `string & {}` keeps the known cases in autocomplete while admitting the rest,
 * so handling an unrecognised one needs no cast to express.
 */
export type ScrobbleUnavailable = KnownScrobbleUnavailable | (string & {});

/** One instance a member may link, and whether this server can link it now. */
export type ScrobbleDestination = {
  provider: ScrobbleProvider;
  destination: string;
  available: boolean;
  /**
   * Why not, when it is not. Present only for an unavailable one, which is why
   * a screen must not read it as the reason a link failed.
   */
  unavailable?: ScrobbleUnavailable;
};

/**
 * What a link reports: a state and four counters, never an echo of a listen.
 *
 * `healthy` cannot mean "the token is still good" — a valid link with three
 * thousand listens waiting since this morning is broken in every sense that
 * matters, and that is what `degraded` says. `broken` is a refused credential,
 * or a destination this server can no longer reach under the name the link was
 * made against.
 */
export type ScrobbleLink = {
  provider: ScrobbleProvider;
  destination: string;
  health: "healthy" | "degraded" | "broken";
  pending: number;
  retrying: number;
  uncertain: number;
  oldest_pending_at: number | null;
  last_success_at: number | null;
  last_failure: string | null;
};

/**
 * One listen whose fate nobody knows, named so that it can be answered.
 *
 * It carries no title and no artists: what identifies it to a person is
 * `played_at`, which matches a listen they already hold from their history,
 * and `destination`, which says which instance may already have it.
 */
export type UncertainScrobble = {
  id: string;
  provider: ScrobbleProvider;
  destination: string;
  played_at: number;
  attempts: number;
  last_failure: string | null;
  updated_at: number;
};

export const listScrobbleDestinations = () =>
  call<ScrobbleDestination[]>("/api/v2/scrobble-destinations");

export const listScrobbleLinks = () =>
  call<ScrobbleLink[]>("/api/v2/scrobble-links");

/**
 * Authorises a destination with a secret the person holds.
 *
 * `PUT` because the instance names the resource: presenting a second token
 * leaves one link, not two. Listens already queued stay attached to the
 * authorisation they were queued under.
 */
export const linkScrobble = (
  provider: ScrobbleProvider,
  destination: string,
  secret: string,
) =>
  call<void>(`/api/v2/scrobble-links/${provider}/${destination}`, {
    method: "PUT",
    body: JSON.stringify({ secret }),
  });

/** Withdraws it. Succeeds whether or not anything was linked. */
export const unlinkScrobble = (
  provider: ScrobbleProvider,
  destination: string,
) =>
  call<void>(`/api/v2/scrobble-links/${provider}/${destination}`, {
    method: "DELETE",
  });

/**
 * Opens the Last.fm journey and answers where to send the browser.
 *
 * The response also sets an `HttpOnly` cookie scoped to the return path, and
 * the return is refused without it. This client is served from the API's own
 * origin, so `fetch`'s default `same-origin` credentials carry it; one served
 * from elsewhere would have to ask for `include`.
 *
 * The journey lasts twelve minutes, is good once, and has to finish in the
 * browser that opened it. Last.fm brings it back to a route that completes the
 * link itself and redirects to `/settings/scrobbling` carrying no token — so
 * nothing here handles the return.
 */
export const authorizeLastFm = (destination: string) =>
  call<{ authorize_url: string; expires_in: number }>(
    `/api/v2/scrobble-links/lastfm/authorize/${destination}`,
    { method: "POST" },
  );

export const listUncertainScrobbles = () =>
  call<UncertainScrobble[]>("/api/v2/scrobble-queue/uncertain");

/** Throws it away. The listen is never submitted and stops being asked about. */
export const discardUncertainScrobble = (entryId: string) =>
  call<void>(`/api/v2/scrobble-queue/uncertain/${entryId}`, {
    method: "DELETE",
  });

/**
 * Sends it again, accepting that the destination may already hold it.
 *
 * Granted once per entry: a second call answers 404, and so does an entry
 * belonging to somebody else. The ambiguous entry stays in the record exactly
 * as it happened — erasing it would falsify the only trace explaining why the
 * destination may hold the listen twice.
 */
export const retryUncertainScrobble = (entryId: string) =>
  call<{ id: string }>(`/api/v2/scrobble-queue/uncertain/${entryId}/retry`, {
    method: "POST",
  });

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
  return { url: ticket.url, expiresAt: ticket.expires_at };
}

export function formatDuration(ms: number): string {
  const total = Math.round(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export type AuthorizeRequest = {
  client_id: string;
  redirect_uri: string;
  code_challenge: string;
  code_challenge_method: string;
  state: string | null;
  device_name: string;
};

export const authorize = (request: AuthorizeRequest) =>
  call<{ redirect_to: string }>("/api/v2/oauth/authorize", {
    method: "POST",
    body: JSON.stringify(request),
  });

/**
 * Mirrors the server's `validate_redirect_uri` policy.
 *
 * The consent screen navigates to the client's redirect target itself when the
 * user cancels, and that target arrives in the query string. Without this check
 * the page is an open redirect: no authorisation code leaks, but a crafted link
 * borrows the server's origin to bounce a visitor anywhere. The server remains
 * the authority for approvals; this guards the navigation the client performs.
 */
export function isAllowedRedirect(redirectUri: string): boolean {
  let url: URL;
  try {
    url = new URL(redirectUri);
  } catch {
    return false;
  }
  if (url.hash) return false;
  switch (url.protocol) {
    case "http:":
      return ["127.0.0.1", "[::1]", "localhost"].includes(url.hostname);
    case "https:":
      return true;
    default:
      // A private-use scheme must be a reverse-domain name, never a bare word
      // another application could plausibly claim.
      return url.protocol.slice(0, -1).includes(".");
  }
}

/**
 * Narrows a remembered destination to a same-document path.
 *
 * The value is read back from storage and handed to `location.assign`, which
 * resolves far more than a path: `//host` is protocol-relative and leaves the
 * origin entirely, and a `javascript:` value would execute. Only a single
 * leading slash followed by a non-slash, non-backslash character is a path —
 * browsers normalise backslashes, so `/\host` escapes just like `//host`.
 */
export function safeInternalPath(value: string | null): string | null {
  if (!value || value.length < 1) return null;
  if (value[0] !== "/") return null;
  if (value[1] === "/" || value[1] === "\\") return null;
  return value;
}

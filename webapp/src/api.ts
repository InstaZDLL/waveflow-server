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
  title: string;
  album: string | null;
  artist: string | null;
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
  url: string;
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
let pendingRefresh: Promise<boolean> | null = null;

function refresh(): Promise<boolean> {
  if (!pendingRefresh) {
    pendingRefresh = performRefresh().finally(() => {
      pendingRefresh = null;
    });
  }
  return pendingRefresh;
}

async function performRefresh(): Promise<boolean> {
  const csrf = cookieValue("waveflow-csrf");
  if (!csrf) return false;
  const response = await fetch("/api/v2/web/auth/refresh", {
    method: "POST",
    headers: { "x-waveflow-csrf": csrf },
  });
  if (!response.ok) {
    session = null;
    return false;
  }
  session = await parse<WebSession>(response);
  return true;
}

export async function ensureSession(): Promise<boolean> {
  return hasSession() || refresh();
}

async function call<T>(
  path: string,
  init: RequestInit = {},
  retry = true,
): Promise<T> {
  const headers = new Headers(init.headers);
  if (session) headers.set("authorization", `Bearer ${session.access_token}`);
  if (init.body) headers.set("content-type", "application/json");
  const response = await fetch(path, { ...init, headers });
  if (response.status === 401 && retry && (await refresh())) {
    return call<T>(path, init, false);
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
async function collect<T>(path: string): Promise<T[]> {
  const pageSize = 500;
  const all: T[] = [];
  for (let offset = 0; ; offset += pageSize) {
    const page = await call<T[]>(`${path}?limit=${pageSize}&offset=${offset}`);
    all.push(...page);
    if (page.length < pageSize) return all;
  }
}

export const listAlbums = () => collect<Album>("/api/v2/albums");
export const getAlbum = (id: string) =>
  call<AlbumDetail>(`/api/v2/albums/${id}`);
export const listArtists = () => collect<Artist>("/api/v2/artists");
export const getArtist = (id: string) =>
  call<ArtistDetail>(`/api/v2/artists/${id}`);
export const search = (query: string) =>
  call<SearchResult>(`/api/v2/search?q=${encodeURIComponent(query)}`);
export const getTrack = (id: string) => call<Song>(`/api/v2/tracks/${id}`);

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

export const scrobble = (trackId: string, submission: boolean) =>
  call<void>("/api/v2/scrobbles", {
    method: "POST",
    body: JSON.stringify({ track_id: trackId, submission }),
  });

/**
 * Exchanges the session for a URL an <audio> element can play. The element
 * cannot send an Authorization header, so the ticket in the path is the
 * credential; it authorises this one track and is re-checked on every range
 * request the browser makes while seeking.
 */
export async function streamUrl(trackId: string): Promise<string> {
  const ticket = await call<{ url: string; expires_at: number }>(
    `/api/v2/tracks/${trackId}/stream-ticket`,
    { method: "POST" },
  );
  return ticket.url;
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

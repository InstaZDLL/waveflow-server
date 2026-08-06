/**
 * Thin client over /api/v2.
 *
 * Access tokens are short-lived, so every call retries once through /refresh
 * before surfacing a 401. Tokens live in localStorage: this is a SPA with no
 * server-rendered session, and the alternative — a cookie — would add ambient
 * authentication and a CSRF surface to an API that is otherwise header-only.
 */

const ACCESS_KEY = "waveflow.access";
const REFRESH_KEY = "waveflow.refresh";

export type Tokens = { access_token: string; refresh_token: string };

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

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

export function storedTokens(): Tokens | null {
  const access_token = localStorage.getItem(ACCESS_KEY);
  const refresh_token = localStorage.getItem(REFRESH_KEY);
  return access_token && refresh_token ? { access_token, refresh_token } : null;
}

function store(tokens: Tokens) {
  localStorage.setItem(ACCESS_KEY, tokens.access_token);
  localStorage.setItem(REFRESH_KEY, tokens.refresh_token);
}

export function clearTokens() {
  localStorage.removeItem(ACCESS_KEY);
  localStorage.removeItem(REFRESH_KEY);
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
  const tokens = storedTokens();
  if (!tokens) return false;
  const response = await fetch("/api/v2/auth/refresh", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ refresh_token: tokens.refresh_token }),
  });
  if (!response.ok) {
    clearTokens();
    return false;
  }
  store(await parse<Tokens>(response));
  return true;
}

async function call<T>(
  path: string,
  init: RequestInit = {},
  retry = true,
): Promise<T> {
  const tokens = storedTokens();
  const headers = new Headers(init.headers);
  if (tokens) headers.set("authorization", `Bearer ${tokens.access_token}`);
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

export async function login(
  username: string,
  password: string,
): Promise<void> {
  const response = await fetch("/api/v2/auth/login", {
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
  store(await parse<Tokens>(response));
}

export async function logout(): Promise<void> {
  try {
    await call<void>("/api/v2/auth/logout", { method: "POST" });
  } finally {
    clearTokens();
  }
}

/** Walks the paged endpoint to completion; the server caps a page at 500. */
async function collect<T>(path: string): Promise<T[]> {
  const pageSize = 500;
  const all: T[] = [];
  for (let offset = 0; ; offset += pageSize) {
    const page = await call<T[]>(
      `${path}?limit=${pageSize}&offset=${offset}`,
    );
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

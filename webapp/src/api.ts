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

async function refresh(): Promise<boolean> {
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

export const listAlbums = () => call<Album[]>("/api/v2/albums?limit=500");
export const getAlbum = (id: string) =>
  call<AlbumDetail>(`/api/v2/albums/${id}`);
export const listArtists = () => call<Artist[]>("/api/v2/artists?limit=500");
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

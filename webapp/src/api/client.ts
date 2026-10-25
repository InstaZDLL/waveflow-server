import type { StreamUrl } from "./stream";

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

export let session: WebSession | null = null;

// M4 originally persisted both tokens. Remove those legacy entries on upgrade
// so an old rotating refresh token is not left readable by JavaScript.
try {
  localStorage.removeItem("waveflow.access");
  localStorage.removeItem("waveflow.refresh");
} catch {
  // Storage may be disabled; sessions do not depend on it anymore.
}

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

export async function parse<T>(response: Response): Promise<T> {
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

export const artworkUrls = new Map<string, Promise<string | null>>();

/**
 * The same answers, once they have arrived, readable without awaiting.
 *
 * The map above holds promises, so nothing in it can be read during a render —
 * and a cover component therefore started every mount with no image and put the
 * grey placeholder on screen for a frame, even for a cover whose bytes were
 * already in memory. Leaving a page and coming back flashed the whole grid.
 */
export const settledArtworkUrls = new Map<string, string | null>();

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
export const canvaslessTracks = new Set<string>();

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
export const canvasTickets = new Map<string, Promise<StreamUrl | null>>();

/** Ask the server again about this track's loop, whatever it said before. */
export function forgetCanvas(trackId: string): void {
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
export let sessionGeneration = 0;

function clearSessionState(): void {
  sessionGeneration += 1;
  clearArtworkUrls();
  canvaslessTracks.clear();
  canvasTickets.clear();
  for (const forget of sessionScoped) forget();
}

/**
 * Whether a refusal addressed to this attempt still deserves a renewal.
 *
 * A sign-out and a sign-in fit inside one round trip, so a 401 raised for
 * account A can arrive after account B has signed in. Renewing then spends
 * **B's** rotating refresh token for a request A made, and the replay goes out
 * under **B's** access token, because a retry rebuilds its headers from the
 * current session. Nobody asked B for that request, and its answer is delivered
 * to a consumer that no longer exists.
 *
 * The generation is checked twice on purpose. Once before the renewal, and
 * again after it, because the renewal is itself a round trip that a sign-out
 * can happen inside — checking only on the way in would let the replay through.
 */
export async function renewFor(generation: number): Promise<boolean> {
  if (generation !== sessionGeneration) return false;
  if (!(await refresh())) return false;
  return generation === sessionGeneration;
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
export function endSession(hadSession: boolean): void {
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

export async function call<T>(
  path: string,
  init: RequestInit = {},
  retry = true,
  /** Set only by the attempt a renewal has already paid for. */
  renewed = false,
): Promise<T> {
  return parse<T>(await request(path, init, retry, renewed));
}

/**
 * One round trip, with the session renewal every caller depends on.
 *
 * Split out of {@link call} rather than copied: `/playlists/{id}/export`
 * answers a file and not JSON, and a second spelling of the renewal would be a
 * second chance to get the generation guard wrong.
 */
export async function request(
  path: string,
  init: RequestInit = {},
  retry = true,
  /** Set only by the attempt a renewal has already paid for. */
  renewed = false,
): Promise<Response> {
  const headers = new Headers(init.headers);
  if (session) headers.set("authorization", `Bearer ${session.access_token}`);
  // Only where the caller has not said otherwise: a playlist file is sent as
  // the bytes it is, and announcing it as JSON would be a lie the server would
  // have to ignore.
  if (init.body && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }
  // Whose session this attempt speaks for, read before the round trip and
  // checked after it — the same guard `performRefresh` already keeps, for the
  // same reason. A signing-out and signing-in fit inside one round trip, and
  // ending a session on a refusal addressed to the account before it would put
  // whoever just arrived back on the sign-in screen.
  const generation = sessionGeneration;
  const response = await fetch(path, { ...init, headers });
  if (response.status === 401 && retry && (await renewFor(generation))) {
    return request(path, init, false, true);
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
  return response;
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

/** The message a setup refused for its token is thrown with (RFC-019). */
export const SETUP_TOKEN_REJECTED = "setup_token_rejected";

/**
 * Creates the first administrator. `setupToken` is the one-time token the
 * server wrote to `setup-token` in its data directory, or the value of
 * `WAVEFLOW_SETUP_TOKEN` when the operator chose one.
 */
export async function bootstrapAdmin(
  username: string,
  password: string,
  setupToken: string,
): Promise<void> {
  const response = await fetch("/api/v2/setup", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      username,
      password,
      setup_token: setupToken.trim(),
    }),
  });
  if (!response.ok) {
    // The token's refusal has its own code, so the screen can point at the
    // field to correct rather than at the password rules.
    const code = await response
      .json()
      .then((body: { code?: unknown }) => body.code)
      .catch(() => undefined);
    throw new ApiError(
      response.status,
      code === SETUP_TOKEN_REJECTED ? SETUP_TOKEN_REJECTED : "setup failed",
    );
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
export async function collect<T>(
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
 * Catalogue calls take the active library, search included since 2026-09-14.
 *
 * It used to be the one that could not, and the screen said so. The reason
 * given was that scoping meant rewriting three FTS queries in a service the
 * frozen Subsonic façade shares — true of the queries, wrong about the
 * sharing: `search2`/`search3` reach a separate method with its own three,
 * and what the two surfaces share is the index and the projections.
 */
export function scoped(
  libraryId: string | undefined,
  extra: Record<string, string> = {},
): Record<string, string> {
  return libraryId ? { ...extra, library_id: libraryId } : extra;
}

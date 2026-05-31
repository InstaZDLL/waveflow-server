// Server-only fetcher for the waveflow-server Rust backend. Lives
// under `src/lib/server/` so it's never bundled into the browser
// chunk — `WAVEFLOW_SERVER_URL` stays on the Nitro side and the
// browser never sees the JWT.
//
// Architecture: TanStack server functions mint a JWT off the user's
// Better Auth session via `auth.api.getToken({ headers })`, then
// call `waveflowFetch` here with the resulting bearer. Going through
// a server function (instead of fetching from the browser directly)
// avoids CORS plumbing on waveflow-server and keeps every secret on
// the server side.
//
// Errors map to the shape the calling server function re-throws —
// TanStack Start serializes the error message back to the client.

const SERVER_URL_ENV = 'WAVEFLOW_SERVER_URL'

export interface WaveflowFetchInit {
  /** Verified JWT minted off the caller's Better Auth session. */
  token: string
  method?: 'GET' | 'POST' | 'PATCH' | 'DELETE'
  body?: unknown
}

export class WaveflowServerError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message)
    this.name = 'WaveflowServerError'
  }
}

/**
 * Call a waveflow-server endpoint with the supplied bearer token.
 * Path is appended to `WAVEFLOW_SERVER_URL` — pass it leading-slashed
 * (e.g. `/api/v1/profiles`). JSON bodies are stringified on the way
 * out; the response body is parsed as JSON when the server returns
 * one (else `undefined`).
 *
 * Failure modes:
 * - Missing `WAVEFLOW_SERVER_URL` env → throws (boot-time misconfig
 *   surfaces on first request rather than silently 500-ing).
 * - Network / DNS failure → fetch rejects, we rethrow unchanged.
 * - Non-2xx response → throws `WaveflowServerError` with status +
 *   the server's text body so the calling server fn can decide
 *   whether to re-throw as a 401 (user retries with refreshed
 *   token) or surface to the client.
 */
export async function waveflowFetch<T = unknown>(
  path: string,
  init: WaveflowFetchInit,
): Promise<T> {
  const base = process.env[SERVER_URL_ENV]
  if (!base) {
    throw new Error(`${SERVER_URL_ENV} is not set. Add it to .env.`)
  }

  const url = `${base.replace(/\/+$/, '')}${path}`
  const hasBody = init.body !== undefined
  const response = await fetch(url, {
    method: init.method ?? 'GET',
    headers: {
      Authorization: `Bearer ${init.token}`,
      ...(hasBody ? { 'Content-Type': 'application/json' } : {}),
    },
    body: hasBody ? JSON.stringify(init.body) : undefined,
  })

  if (!response.ok) {
    const text = await response.text().catch(() => '')
    throw new WaveflowServerError(
      response.status,
      text || `waveflow-server returned ${response.status}`,
    )
  }

  // 204 No Content + empty bodies — return `undefined` cast through
  // T. Callers that expect a body should narrow T accordingly.
  if (response.status === 204) {
    return undefined as T
  }
  const text = await response.text()
  if (!text) {
    return undefined as T
  }
  return JSON.parse(text) as T
}

// Server function for the streaming bridge. Mints a signed URL on
// the server side (Bearer-authed against waveflow-server's mint
// endpoint), then rewrites the relative `url` (`/api/v1/stream/...`)
// to an absolute one the browser's `<audio src>` can hit directly.
//
// The browser fetches the audio bytes from waveflow-server WITHOUT
// CORS preflight (media elements don't issue one unless the
// `crossorigin` attribute is set), so the audio plays even when the
// two hosts live on different ports during dev.

import { createServerFn } from '@tanstack/react-start'
import { waveflowFetch } from '@/lib/server/waveflow-server'
import { mintToken, withSafeErrors } from './_internal'

interface RawMintResponse {
  url: string
  expires_at: number
}

export interface StreamUrl {
  /** Absolute URL the browser drops into `<audio src>`. */
  url: string
  /** Unix epoch second past which the URL stops working. */
  expiresAt: number
}

export interface GetStreamUrlParams {
  profileId: number
  libraryId: number
  trackId: number
}

const SERVER_URL_ENV = 'WAVEFLOW_SERVER_URL'

/**
 * Mint a short-lived signed URL for a track the caller owns.
 * `waveflow-server` returns the URL as a server-relative path so
 * the same payload works under any deployment shape; the browser
 * needs an absolute URL, so we prepend `WAVEFLOW_SERVER_URL`.
 *
 * Errors:
 * - 404 from waveflow-server → "Not found." (handled by the shared
 *   envelope). The mint endpoint returns 404 both for "no such track"
 *   AND "track belongs to another user" — no-leak rule.
 * - 503 from waveflow-server → streaming disabled at the server end
 *   (`WAVEFLOW_MUSIC_ROOT` / `WAVEFLOW_STREAM_SECRET` unset).
 */
export const getStreamUrl = createServerFn({ method: 'GET' })
  .inputValidator((params: GetStreamUrlParams) => params)
  .handler(
    async ({ data }): Promise<StreamUrl> =>
      withSafeErrors('getStreamUrl', async () => {
        const base = process.env[SERVER_URL_ENV]
        if (!base) {
          throw new Error(`${SERVER_URL_ENV} is not set. Add it to .env.`)
        }

        const token = await mintToken()
        const minted = await waveflowFetch<RawMintResponse>(
          `/api/v1/profiles/${data.profileId}/libraries/${data.libraryId}/tracks/${data.trackId}/stream-url`,
          { token, method: 'POST' },
        )

        return {
          url: `${base.replace(/\/+$/, '')}${minted.url}`,
          expiresAt: minted.expires_at,
        }
      }),
  )

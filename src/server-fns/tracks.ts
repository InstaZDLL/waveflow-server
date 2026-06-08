// Server functions for the
// `/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks`
// surface. See `_internal.ts` for the shared JWT-mint + error
// envelope.

import { createServerFn } from '@tanstack/react-start'
import { waveflowFetch } from '@/lib/server/waveflow-server'
import { asPathId, mintToken, withSafeErrors } from './_internal'

export interface Track {
  id: number
  library_id: number
  /**
   * FK into the per-library `album` row, populated since Phase
   * 4.d.0.4. `null` for free-form tracks the sync apply pipeline
   * couldn't group (no album metadata in the source tag) — and
   * still `null` on every response from `listTracks` until the
   * `/tracks` collection's SELECT (lives in waveflow-core) is
   * bumped to project the column. The drill-down endpoints
   * (`/albums/{id}/tracks`, `/artists/{id}/tracks`) already
   * surface the real value.
   */
  album_id: number | null
  title: string
  file_path: string
  file_size: number
  duration_ms: number
  track_number: number | null
  disc_number: number | null
  year: number | null
  bitrate: number | null
  sample_rate: number | null
  channels: number | null
  bit_depth: number | null
  codec: string | null
  rating: number | null
}

export interface ListTracksParams {
  profileId: number
  libraryId: number
}

/**
 * List every track in the supplied library. Same tenancy guarantee
 * as [`listLibraries`] — a foreign id surfaces as "Not found."
 */
export const listTracks = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): ListTracksParams => {
    const raw = value as Partial<ListTracksParams> | undefined
    return {
      profileId: asPathId(raw?.profileId, 'profileId'),
      libraryId: asPathId(raw?.libraryId, 'libraryId'),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('listTracks', async () => {
      const token = await mintToken()
      return waveflowFetch<Track[]>(
        `/api/v1/profiles/${data.profileId}/libraries/${data.libraryId}/tracks`,
        { token },
      )
    }),
  )

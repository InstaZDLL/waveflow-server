// Server functions for the
// `/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks`
// surface. See `_internal.ts` for the shared JWT-mint + error
// envelope.

import { createServerFn } from '@tanstack/react-start'
import { waveflowFetch } from '@/lib/server/waveflow-server'
import { mintToken, withSafeErrors } from './_internal'

export interface Track {
  id: number
  library_id: number
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
  .inputValidator((params: ListTracksParams) => params)
  .handler(async ({ data }) =>
    withSafeErrors('listTracks', async () => {
      const token = await mintToken()
      return waveflowFetch<Track[]>(
        `/api/v1/profiles/${data.profileId}/libraries/${data.libraryId}/tracks`,
        { token },
      )
    }),
  )

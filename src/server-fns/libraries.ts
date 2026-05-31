// Server functions for the `/api/v1/profiles/{profile_id}/libraries`
// surface. See `_internal.ts` for the shared JWT-mint + error
// envelope.

import { createServerFn } from '@tanstack/react-start'
import { waveflowFetch } from '@/lib/server/waveflow-server'
import { mintToken, withSafeErrors } from './_internal'

export interface Library {
  id: number
  profile_id: number
  name: string
  description: string | null
  color_id: string
  icon_id: string
  track_count: number
  created_at: number
  last_used_at: number
}

/**
 * List every library owned by the calling user under `profileId`.
 * `profileId` is validated server-side by the `*_for_user` repository
 * call — a non-owner gets 404, surfaced as "Not found." in the UI.
 */
export const listLibraries = createServerFn({ method: 'GET' })
  .inputValidator((profileId: number) => profileId)
  .handler(async ({ data: profileId }) =>
    withSafeErrors('listLibraries', async () => {
      const token = await mintToken()
      return waveflowFetch<Library[]>(`/api/v1/profiles/${profileId}/libraries`, { token })
    }),
  )

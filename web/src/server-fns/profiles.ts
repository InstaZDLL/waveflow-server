// TanStack server functions for the `/api/v1/profiles` surface on
// waveflow-server. Each function lives on the Nitro side: it pulls
// the request headers, mints a JWT off the active Better Auth
// session, and forwards to waveflow-server with the bearer attached.
//
// Shared envelope + JWT minting live in `_internal.ts` so the rest
// of the file stays focused on resource-specific logic.

import { createServerFn } from '@tanstack/react-start'
import { waveflowFetch } from '@/lib/server/waveflow-server'
import { mintToken, withSafeErrors } from './_internal'

export interface Profile {
  id: number
  name: string
  color_id: string
  avatar_hash: string | null
  created_at: number
  last_used_at: number
}

export const listProfiles = createServerFn({ method: 'GET' }).handler(async () =>
  withSafeErrors('listProfiles', async () => {
    const token = await mintToken()
    return waveflowFetch<Profile[]>('/api/v1/profiles', { token })
  }),
)

// Server functions for the `/api/v1/profiles/{profile_id}/playlists`
// surface. See `_internal.ts` for the shared JWT-mint + error
// envelope. The server's wire format matches the desktop's
// `Playlist` DTO (minus the path-derived `profile_id` and the
// desktop-only `cover_path`); this file mirrors it 1:1 so a
// future sync with the desktop's playlist editor doesn't need a
// translation layer.

import { createServerFn } from '@tanstack/react-start'
import { waveflowFetch } from '@/lib/server/waveflow-server'
import { asPathId, mintToken, withSafeErrors } from './_internal'

export interface Playlist {
  id: number
  name: string
  description: string | null
  color_id: string
  icon_id: string
  /** `0` for user-curated playlists, `1` for smart-generated ones. */
  is_smart: number
  cover_hash: string | null
  /**
   * `1` when the cover is managed by the auto-regen pipeline, `0`
   * when the user uploaded their own image and the pipeline should
   * leave it alone.
   */
  cover_is_auto: number
  position: number
  created_at: number
  updated_at: number
  /** Denormalised, server-maintained as `playlist_track` rows change. */
  track_count: number
  total_duration_ms: number
  /** Raw JSON payload from `playlist.smart_rules`. `null` on custom playlists. */
  smart_rules: string | null
}

/**
 * List every playlist the calling user owns under `profileId`,
 * ordered `(position ASC, updated_at DESC)` per the server's
 * sidebar convention. A foreign `profileId` resolves to `[]` —
 * the server doesn't leak existence.
 */
export const listPlaylists = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown) => asPathId(value, 'profileId'))
  .handler(async ({ data: profileId }) =>
    withSafeErrors('listPlaylists', async () => {
      const token = await mintToken()
      return waveflowFetch<Playlist[]>(`/api/v1/profiles/${profileId}/playlists`, { token })
    }),
  )

export interface GetPlaylistParams {
  profileId: number
  playlistId: number
}

export interface CreatePlaylistParams {
  profileId: number
  name: string
  description?: string
  color_id?: string
  icon_id?: string
}

/**
 * Create a custom playlist under `profileId`. The server trims +
 * validates the name (empty / whitespace-only after trim → 400).
 * `color_id` / `icon_id` fall back to the server-side defaults
 * (`violet` / `music`) when omitted. Smart playlists aren't
 * writable through this route — the server hardcodes `is_smart=0`
 * on insert.
 */
export const createPlaylist = createServerFn({ method: 'POST' })
  .inputValidator((value: unknown): CreatePlaylistParams => {
    const raw = value as Partial<CreatePlaylistParams> | undefined
    const profileId = asPathId(raw?.profileId, 'profileId')
    const name = typeof raw?.name === 'string' ? raw.name.trim() : ''
    if (!name) throw new Error('Name is required.')
    if (name.length > 200) throw new Error('Name must be 200 characters or fewer.')
    // Description gets the same defensive symmetry as name —
    // client-side dialog gates at 1000 chars, but a future caller
    // (another server-fn consumer, a hand-rolled RPC call) would
    // otherwise only learn the cap from waveflow-server's 400.
    const description = typeof raw?.description === 'string' ? raw.description.trim() : undefined
    if (description !== undefined && description.length > 1000) {
      throw new Error('Description must be 1000 characters or fewer.')
    }
    // The remaining optional fields pass through as-is so the
    // server's defaults / validation own the contract.
    return {
      profileId,
      name,
      ...(description ? { description } : {}),
      ...(typeof raw?.color_id === 'string' ? { color_id: raw.color_id } : {}),
      ...(typeof raw?.icon_id === 'string' ? { icon_id: raw.icon_id } : {}),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('createPlaylist', async () => {
      const token = await mintToken()
      const { profileId, ...body } = data
      return waveflowFetch<Playlist>(`/api/v1/profiles/${profileId}/playlists`, {
        method: 'POST',
        token,
        body,
      })
    }),
  )

/**
 * Fetch a single playlist by id. 404s on a foreign / nonexistent
 * id (surfaced as "Not found." via the error envelope).
 */
export const getPlaylist = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): GetPlaylistParams => {
    const raw = value as Partial<GetPlaylistParams> | undefined
    return {
      profileId: asPathId(raw?.profileId, 'profileId'),
      playlistId: asPathId(raw?.playlistId, 'playlistId'),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('getPlaylist', async () => {
      const token = await mintToken()
      return waveflowFetch<Playlist>(
        `/api/v1/profiles/${data.profileId}/playlists/${data.playlistId}`,
        { token },
      )
    }),
  )

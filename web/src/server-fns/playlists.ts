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

export interface UpdatePlaylistParams {
  profileId: number
  playlistId: number
  name?: string
  description?: string
  color_id?: string
  icon_id?: string
}

/**
 * Partially update a playlist. Every field is optional; the server
 * COALESCEs absent fields against the current row so a PATCH with
 * only `name` set leaves the description untouched. `name` must be
 * non-empty / non-whitespace when present (rejected with 400 on
 * the server; mirrored client-side here for symmetry with
 * `createPlaylist`).
 *
 * Smart playlists aren't writable here either — the server's
 * repository call refuses to PATCH a row where `is_smart=1` so a
 * mistaken edit from the web UI surfaces as 404.
 */
export const updatePlaylist = createServerFn({ method: 'POST' })
  .inputValidator((value: unknown): UpdatePlaylistParams => {
    const raw = value as Partial<UpdatePlaylistParams> | undefined
    const profileId = asPathId(raw?.profileId, 'profileId')
    const playlistId = asPathId(raw?.playlistId, 'playlistId')
    const out: UpdatePlaylistParams = { profileId, playlistId }
    if (typeof raw?.name === 'string') {
      const trimmed = raw.name.trim()
      if (!trimmed) throw new Error('Name is required.')
      if (trimmed.length > 200) throw new Error('Name must be 200 characters or fewer.')
      out.name = trimmed
    }
    if (typeof raw?.description === 'string') {
      const trimmed = raw.description.trim()
      if (trimmed.length > 1000) {
        throw new Error('Description must be 1000 characters or fewer.')
      }
      // Empty-after-trim still goes through — that's how the user
      // signals "clear the description". The server treats it as
      // an explicit value (not Option::None) because the key is
      // present.
      out.description = trimmed
    }
    if (typeof raw?.color_id === 'string') out.color_id = raw.color_id
    if (typeof raw?.icon_id === 'string') out.icon_id = raw.icon_id
    return out
  })
  .handler(async ({ data }) =>
    withSafeErrors('updatePlaylist', async () => {
      const token = await mintToken()
      const { profileId, playlistId, ...body } = data
      return waveflowFetch<Playlist>(`/api/v1/profiles/${profileId}/playlists/${playlistId}`, {
        method: 'PATCH',
        token,
        body,
      })
    }),
  )

export interface DeletePlaylistParams {
  profileId: number
  playlistId: number
}

/**
 * Delete a playlist. 204 on success, 404 on a foreign / nonexistent
 * id (surfaced as "Not found." via the error envelope).
 */
export const deletePlaylist = createServerFn({ method: 'POST' })
  .inputValidator((value: unknown): DeletePlaylistParams => {
    const raw = value as Partial<DeletePlaylistParams> | undefined
    return {
      profileId: asPathId(raw?.profileId, 'profileId'),
      playlistId: asPathId(raw?.playlistId, 'playlistId'),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('deletePlaylist', async () => {
      const token = await mintToken()
      await waveflowFetch<void>(`/api/v1/profiles/${data.profileId}/playlists/${data.playlistId}`, {
        method: 'DELETE',
        token,
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

/**
 * One row of the playlist's track list. The server returns rows
 * in `(position ASC, track_id ASC)` order. Snapshot fields are
 * nullable on purpose: pre-1.j.b desktops emitted tracks ops
 * without them, and the owner is allowed to see the rows anyway
 * (the public share preview is the only surface that filters
 * NULL snapshots). The UI renders a placeholder for those rows
 * rather than hiding them.
 *
 * `track_id` is the SOURCE desktop's local i64 id — NOT a
 * server-canonical reference. Treat it as opaque row-key
 * material only; cross-device track resolution is a future
 * concern (phase 1.k server-side). The UI should NOT render it
 * to the user — it changes per-desktop and means nothing on
 * another device's view.
 *
 * `position` is 0-indexed on the wire (matches the desktop's
 * SQLite column). The web UI renders ordinals as `1..N` over
 * the rendered array order — see `TrackList` for the rationale.
 * Sparse positions (gaps from deletes) are tolerated by the
 * server's `ORDER BY` and don't surface to the user.
 */
export interface PlaylistTrack {
  track_id: number
  position: number
  added_at: number
  snapshot_title: string | null
  snapshot_artist: string | null
  snapshot_duration_ms: number | null
}

/**
 * Fetch the tracks of a playlist owned by the calling user. 404s
 * on a foreign / nonexistent playlist (surfaced as "Not found."
 * via the error envelope). Empty playlist returns `[]`, NOT 404
 * — the row exists, it just has no tracks.
 */
export const getPlaylistTracks = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): GetPlaylistParams => {
    const raw = value as Partial<GetPlaylistParams> | undefined
    return {
      profileId: asPathId(raw?.profileId, 'profileId'),
      playlistId: asPathId(raw?.playlistId, 'playlistId'),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('getPlaylistTracks', async () => {
      const token = await mintToken()
      return waveflowFetch<PlaylistTrack[]>(
        `/api/v1/profiles/${data.profileId}/playlists/${data.playlistId}/tracks`,
        { token },
      )
    }),
  )

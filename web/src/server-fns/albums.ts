// Server functions for the
// `/api/v1/profiles/{profile_id}/libraries/{library_id}/albums*`
// surface (waveflow-server phase 4.d.0.4). See `_internal.ts` for
// the shared JWT-mint + error envelope.
//
// Album rows are read-only on the wire — they materialise from the
// sync apply pipeline on the server, not from a user-facing CRUD
// form. The drill-down `/tracks` endpoint returns the same `Track`
// shape as `listTracks`, so the existing player flow plugs straight
// in.

import { createServerFn } from '@tanstack/react-start'
import { waveflowFetch } from '@/lib/server/waveflow-server'
import { asPathId, mintToken, withSafeErrors } from './_internal'
import type { Track } from './tracks'

export interface Album {
  id: number
  canonical_title: string
  /**
   * FK into `artist`. `null` for compilations (the apply pipeline
   * sets this to `null` when the source ships `is_compilation`
   * or `merge_implicit_compilations` collapses ≥ 3 distinct-artist
   * same-title rows into "Various Artists"). `album_artist_name`
   * tracks this — both are `null` together.
   */
  album_artist_id: number | null
  album_artist_name: string | null
  year: number | null
  /**
   * BLAKE3 hex of the album cover in the shared metadata cache.
   * `null` until the server-side cover-extraction pipeline ships
   * — the UI falls back to a neutral placeholder.
   */
  cover_hash: string | null
  is_compilation: boolean
  created_at: number
  updated_at: number
}

export interface ListAlbumsParams {
  profileId: number
  libraryId: number
}

/**
 * List every album in the supplied library, most-recently-updated
 * first. Tie-broken on `id ASC` so the order is deterministic when
 * a sync round upserts several rows at the same epoch millisecond.
 * 404 on a foreign / nonexistent library (surfaced as "Not found.").
 */
export const listAlbums = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): ListAlbumsParams => {
    const raw = value as Partial<ListAlbumsParams> | undefined
    return {
      profileId: asPathId(raw?.profileId, 'profileId'),
      libraryId: asPathId(raw?.libraryId, 'libraryId'),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('listAlbums', async () => {
      const token = await mintToken()
      return waveflowFetch<Album[]>(
        `/api/v1/profiles/${data.profileId}/libraries/${data.libraryId}/albums`,
        { token },
      )
    }),
  )

export interface GetAlbumTracksParams {
  profileId: number
  libraryId: number
  albumId: number
}

/**
 * Drill-down: list every track linked to `albumId`, ordered
 * `(disc_number, track_number, id)` so the standard sleeve order
 * falls out naturally. 404 on a foreign / nonexistent album;
 * empty owned album resolves to `[]`.
 */
export const getAlbumTracks = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): GetAlbumTracksParams => {
    const raw = value as Partial<GetAlbumTracksParams> | undefined
    return {
      profileId: asPathId(raw?.profileId, 'profileId'),
      libraryId: asPathId(raw?.libraryId, 'libraryId'),
      albumId: asPathId(raw?.albumId, 'albumId'),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('getAlbumTracks', async () => {
      const token = await mintToken()
      return waveflowFetch<Track[]>(
        `/api/v1/profiles/${data.profileId}/libraries/${data.libraryId}/albums/${data.albumId}/tracks`,
        { token },
      )
    }),
  )

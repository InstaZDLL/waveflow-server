// Server functions for the
// `/api/v1/profiles/{profile_id}/libraries/{library_id}/artists*`
// surface (waveflow-server phase 4.d.0.4). See `_internal.ts` for
// the shared JWT-mint + error envelope.
//
// Artist rows are read-only on the wire — they materialise from the
// sync apply pipeline on the server. The drill-down `/tracks`
// endpoint joins through `track_artist`, so a multi-artist track
// surfaces under every contributor.

import { createServerFn } from '@tanstack/react-start'
import { waveflowFetch } from '@/lib/server/waveflow-server'
import { asPathId, mintToken, withSafeErrors } from './_internal'
import type { Track } from './tracks'

export interface Artist {
  id: number
  name: string
  /**
   * BLAKE3 hex of the artist picture in the shared metadata cache.
   * `null` until the server-side artist-picture pipeline ships —
   * the UI falls back to a neutral placeholder.
   */
  picture_hash: string | null
  created_at: number
  updated_at: number
}

export interface ListArtistsParams {
  profileId: number
  libraryId: number
}

/**
 * List every artist in the supplied library, most-recently-updated
 * first. Same `id ASC` tiebreak as `listAlbums`. 404 on a foreign
 * / nonexistent library (surfaced as "Not found.").
 */
export const listArtists = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): ListArtistsParams => {
    const raw = value as Partial<ListArtistsParams> | undefined
    return {
      profileId: asPathId(raw?.profileId, 'profileId'),
      libraryId: asPathId(raw?.libraryId, 'libraryId'),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('listArtists', async () => {
      const token = await mintToken()
      return waveflowFetch<Artist[]>(
        `/api/v1/profiles/${data.profileId}/libraries/${data.libraryId}/artists`,
        { token },
      )
    }),
  )

export interface GetArtistTracksParams {
  profileId: number
  libraryId: number
  artistId: number
}

/**
 * Drill-down: every track contributed by `artistId`. Multi-artist
 * tracks surface under every contributor — the server joins
 * `track → track_artist → artist` and filters by `library_id` so
 * the result stays tenant-correct. 404 on a foreign / nonexistent
 * artist; empty owned artist resolves to `[]`.
 */
export const getArtistTracks = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): GetArtistTracksParams => {
    const raw = value as Partial<GetArtistTracksParams> | undefined
    return {
      profileId: asPathId(raw?.profileId, 'profileId'),
      libraryId: asPathId(raw?.libraryId, 'libraryId'),
      artistId: asPathId(raw?.artistId, 'artistId'),
    }
  })
  .handler(async ({ data }) =>
    withSafeErrors('getArtistTracks', async () => {
      const token = await mintToken()
      return waveflowFetch<Track[]>(
        `/api/v1/profiles/${data.profileId}/libraries/${data.libraryId}/artists/${data.artistId}/tracks`,
        { token },
      )
    }),
  )

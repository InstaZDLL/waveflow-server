import { Link, createFileRoute } from '@tanstack/react-router'

import { PlayableTrackList } from '@/components/PlayableTrackList'
import { getArtistTracks, listArtists, type Artist } from '@/server-fns/artists'
import type { Track } from '@/server-fns/tracks'

// Artist drill-down — header (resolved via the list endpoint) plus
// every track the artist contributed under this library, rendered
// through the shared `PlayableTrackList`. Multi-artist tracks
// surface under every contributor (the server joins
// `track → track_artist → artist`), so a duet shows up on both
// artists' pages.
export const Route = createFileRoute(
  '/_authed/profiles/$profileId/libraries/$libraryId/artists/$artistId',
)({
  loader: async ({ params }): Promise<LoaderData> => {
    const profileId = Number(params.profileId)
    const libraryId = Number(params.libraryId)
    const artistId = Number(params.artistId)
    if (![profileId, libraryId, artistId].every((id) => Number.isInteger(id) && id > 0)) {
      return { kind: 'error', message: 'Invalid profile, library, or artist id.' }
    }
    // Same shape as the album drill-down: tracks fetch is the
    // authoritative "is this URL valid" signal (it does its own
    // ownership check on the artist row), the list fetch is
    // best-effort metadata for the header. See the album-detail
    // loader header for the full rationale.
    let tracks: Track[]
    try {
      tracks = await getArtistTracks({ data: { profileId, libraryId, artistId } })
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load tracks.',
      }
    }
    const artistResult = await listArtists({ data: { profileId, libraryId } })
      .then(
        (artists): ArtistLookupResult => ({
          ok: true,
          artist: artists.find((a) => a.id === artistId) ?? null,
        }),
      )
      .catch(
        (err: unknown): ArtistLookupResult => ({
          ok: false,
          error: err instanceof Error ? err.message : 'Failed to resolve artist metadata.',
        }),
      )
    return {
      kind: 'ready',
      profileId,
      libraryId,
      artistId,
      artistResult,
      tracks,
    }
  },
  component: ArtistDetailView,
})

export type ArtistLookupResult = { ok: true; artist: Artist | null } | { ok: false; error: string }

export type LoaderData =
  | {
      kind: 'ready'
      profileId: number
      libraryId: number
      artistId: number
      artistResult: ArtistLookupResult
      tracks: Track[]
    }
  | { kind: 'error'; message: string }

export function ArtistDetailView() {
  const data = Route.useLoaderData()

  return (
    <main className="page-wrap px-4 py-12 pb-32">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        {data.kind === 'error' && (
          <p role="alert" className="text-base text-red-600 dark:text-red-400">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && (
          <>
            <ArtistHeader result={data.artistResult} />
            <PlayableTrackList
              profileId={data.profileId}
              libraryId={data.libraryId}
              tracks={data.tracks}
              emptyMessage="No tracks contributed by this artist yet."
              label="Artist tracks"
            />

            <p className="mt-6">
              <Link
                to="/profiles/$profileId/libraries/$libraryId/artists"
                params={{
                  profileId: String(data.profileId),
                  libraryId: String(data.libraryId),
                }}
                className="text-sm text-[var(--sea-ink-soft)] underline"
              >
                ← Back to artists
              </Link>
            </p>
          </>
        )}
      </section>
    </main>
  )
}

function ArtistHeader({ result }: { result: ArtistLookupResult }) {
  if (!result.ok || !result.artist) {
    return (
      <div className="mb-6 flex flex-col gap-2">
        <p className="island-kicker m-0">Artist</p>
        <h1 className="display-title text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Artist
        </h1>
        {!result.ok && (
          <p className="text-xs text-[var(--sea-ink-soft)]">
            Artist details unavailable: {result.error}
          </p>
        )}
      </div>
    )
  }
  return (
    <div className="mb-6 flex flex-col gap-2">
      <p className="island-kicker m-0">Artist</p>
      <h1 className="display-title text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
        {result.artist.name}
      </h1>
    </div>
  )
}

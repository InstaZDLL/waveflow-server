import { Link, createFileRoute } from '@tanstack/react-router'

import { PlayableTrackList } from '@/components/PlayableTrackList'
import { getAlbumTracks, listAlbums, type Album } from '@/server-fns/albums'
import type { Track } from '@/server-fns/tracks'

// Album drill-down — header (resolved via the list endpoint, since
// the server doesn't yet expose a per-id GET for albums) + the
// per-album track list rendered through the shared
// `PlayableTrackList` component. Auth gating inherited from
// `_authed`.
export const Route = createFileRoute(
  '/_authed/profiles/$profileId/libraries/$libraryId/albums/$albumId',
)({
  loader: async ({ params }): Promise<LoaderData> => {
    const profileId = Number(params.profileId)
    const libraryId = Number(params.libraryId)
    const albumId = Number(params.albumId)
    if (![profileId, libraryId, albumId].every((id) => Number.isInteger(id) && id > 0)) {
      return { kind: 'error', message: 'Invalid profile, library, or album id.' }
    }
    // The tracks fetch IS the page — without rows there's nothing
    // worth rendering. Its own ownership check also tells us
    // whether the album exists under this library, so we use it as
    // the authoritative "is this URL valid" signal. The list fetch
    // is purely for the header subtitle (`album_artist_name` etc.)
    // — its failures degrade gracefully into a neutral header.
    let tracks: Track[]
    try {
      tracks = await getAlbumTracks({ data: { profileId, libraryId, albumId } })
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load tracks.',
      }
    }
    // Header metadata — best-effort. The server has no per-id GET
    // for albums yet (4.d.0.4 only ships the list + drill-down),
    // so we resolve the row from the per-library list. If the list
    // fails (transient 500) or the album isn't in the returned set
    // (race window: a peer device deleted it between the two
    // requests, but the tracks fetch raced to OK first), the
    // header falls back to a neutral "Album" — the tracks below
    // are still playable so the page stays useful.
    const albumResult = await listAlbums({ data: { profileId, libraryId } })
      .then(
        (albums): AlbumLookupResult => ({
          ok: true,
          album: albums.find((a) => a.id === albumId) ?? null,
        }),
      )
      .catch(
        (err: unknown): AlbumLookupResult => ({
          ok: false,
          error: err instanceof Error ? err.message : 'Failed to resolve album metadata.',
        }),
      )
    return {
      kind: 'ready',
      profileId,
      libraryId,
      albumId,
      albumResult,
      tracks,
    }
  },
  component: AlbumDetailView,
})

export type AlbumLookupResult = { ok: true; album: Album | null } | { ok: false; error: string }

export type LoaderData =
  | {
      kind: 'ready'
      profileId: number
      libraryId: number
      albumId: number
      albumResult: AlbumLookupResult
      tracks: Track[]
    }
  | { kind: 'error'; message: string }

export function AlbumDetailView() {
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
            <AlbumHeader result={data.albumResult} />
            <PlayableTrackList
              profileId={data.profileId}
              libraryId={data.libraryId}
              tracks={data.tracks}
              emptyMessage="No tracks linked to this album yet."
              label="Album tracks"
            />

            <p className="mt-6">
              <Link
                to="/profiles/$profileId/libraries/$libraryId/albums"
                params={{
                  profileId: String(data.profileId),
                  libraryId: String(data.libraryId),
                }}
                className="text-sm text-[var(--sea-ink-soft)] underline"
              >
                ← Back to albums
              </Link>
            </p>
          </>
        )}
      </section>
    </main>
  )
}

function AlbumHeader({ result }: { result: AlbumLookupResult }) {
  // The list-fetch can fail OR succeed-but-miss (album not in the
  // returned set — happens if a peer device deletes it between the
  // two parallel fetches). Both fall through to the same neutral
  // header so the rest of the page (tracks!) still renders.
  if (!result.ok || !result.album) {
    return (
      <div className="mb-6 flex flex-col gap-2">
        <p className="island-kicker m-0">Album</p>
        <h1 className="display-title text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Album
        </h1>
        {!result.ok && (
          <p className="text-xs text-[var(--sea-ink-soft)]">
            Album details unavailable: {result.error}
          </p>
        )}
      </div>
    )
  }
  const album = result.album
  const subtitle = album.is_compilation
    ? 'Various Artists'
    : (album.album_artist_name ?? 'Unknown artist')
  return (
    <div className="mb-6 flex flex-col gap-2">
      <p className="island-kicker m-0">Album</p>
      <h1 className="display-title text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
        {album.canonical_title}
      </h1>
      <p className="text-sm text-[var(--sea-ink-soft)]">
        {subtitle}
        {album.year ? ` · ${album.year}` : ''}
        {album.is_compilation && (
          <span
            className="ml-2 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider"
            style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
          >
            Compilation
          </span>
        )}
      </p>
    </div>
  )
}

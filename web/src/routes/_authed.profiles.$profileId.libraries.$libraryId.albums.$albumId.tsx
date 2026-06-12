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
    //
    // The two run in PARALLEL so total wait = max(tracks, list)
    // rather than the sum. Header metadata can still come back
    // slowly on a large library (5k+ albums), so awaiting it
    // sequentially after the tracks would double the perceived
    // load time. Each promise carries its own inline `.catch` so
    // `Promise.all` itself cannot reject — we check the tracks
    // outcome explicitly after the join.
    type TracksOutcome = { ok: true; tracks: Track[] } | { ok: false; message: string }
    const [tracksResult, albumResult] = await Promise.all([
      getAlbumTracks({ data: { profileId, libraryId, albumId } })
        .then((t): TracksOutcome => ({ ok: true, tracks: t }))
        .catch(
          (err: unknown): TracksOutcome => ({
            ok: false,
            message: err instanceof Error ? err.message : 'Failed to load tracks.',
          }),
        ),
      listAlbums({ data: { profileId, libraryId } })
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
        ),
    ])
    if (!tracksResult.ok) {
      return { kind: 'error', message: tracksResult.message }
    }
    return {
      kind: 'ready',
      profileId,
      libraryId,
      albumId,
      albumResult,
      tracks: tracksResult.tracks,
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
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad">
        {data.kind === 'error' && (
          <p role="alert" className="error-card text-sm">
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
                className="back-link"
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
        <p className="section-eyebrow m-0">Album</p>
        <h1 className="display-title text-4xl font-bold text-[var(--sea-ink)]">Album</h1>
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
      <p className="section-eyebrow m-0">Album</p>
      <h1 className="display-title text-4xl font-bold text-[var(--sea-ink)]">
        {album.canonical_title}
      </h1>
      <p className="text-sm text-[var(--sea-ink-soft)]">
        {subtitle}
        {album.year ? ` · ${album.year}` : ''}
        {album.is_compilation && (
          <span
            className="ml-2 rounded-md px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider"
            style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
          >
            Compilation
          </span>
        )}
      </p>
    </div>
  )
}

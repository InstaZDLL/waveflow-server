import { Link, createFileRoute } from '@tanstack/react-router'
import { listAlbums, type Album } from '@/server-fns/albums'

// Album browse — list every album in the library so the user can
// drill into a per-album track view. Auth gating inherited from the
// `_authed` parent layout.
export const Route = createFileRoute('/_authed/profiles/$profileId/libraries/$libraryId/albums')({
  loader: async ({ params }): Promise<LoaderData> => {
    const profileId = Number(params.profileId)
    const libraryId = Number(params.libraryId)
    if (![profileId, libraryId].every((id) => Number.isInteger(id) && id > 0)) {
      return { kind: 'error', message: 'Invalid profile or library id.' }
    }
    try {
      const albums = await listAlbums({ data: { profileId, libraryId } })
      return { kind: 'ready', profileId, libraryId, albums }
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load albums.',
      }
    }
  },
  component: AlbumsView,
})

type LoaderData =
  | { kind: 'ready'; profileId: number; libraryId: number; albums: Album[] }
  | { kind: 'error'; message: string }

export function AlbumsView() {
  const data = Route.useLoaderData()

  return (
    <main className="page-wrap px-4 py-12 pb-32">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Library</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Albums
        </h1>

        {data.kind === 'error' && (
          <p role="alert" className="text-base text-red-600 dark:text-red-400">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && data.albums.length === 0 && (
          <p className="text-base text-[var(--sea-ink-soft)]">
            No albums in this library yet. The desktop app populates these as it scans your music.
          </p>
        )}

        {data.kind === 'ready' && data.albums.length > 0 && (
          <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {data.albums.map((album) => (
              <li key={album.id}>
                <Link
                  to="/profiles/$profileId/libraries/$libraryId/albums/$albumId"
                  params={{
                    profileId: String(data.profileId),
                    libraryId: String(data.libraryId),
                    albumId: String(album.id),
                  }}
                  className="block rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] p-4 no-underline transition hover:opacity-90"
                >
                  <p className="truncate text-base font-semibold text-[var(--sea-ink)]">
                    {album.canonical_title}
                  </p>
                  <p className="mt-1 truncate text-xs text-[var(--sea-ink-soft)]">
                    {/* Compilation rows have null album_artist_name —
                        render the conventional "Various Artists"
                        rather than leaking an empty subtitle. */}
                    {album.is_compilation
                      ? 'Various Artists'
                      : (album.album_artist_name ?? 'Unknown artist')}
                    {album.year ? ` · ${album.year}` : ''}
                  </p>
                </Link>
              </li>
            ))}
          </ul>
        )}

        {data.kind === 'ready' && (
          <p className="mt-6">
            <Link
              to="/profiles/$profileId/libraries/$libraryId"
              params={{
                profileId: String(data.profileId),
                libraryId: String(data.libraryId),
              }}
              className="text-sm text-[var(--sea-ink-soft)] underline"
            >
              ← Back to library
            </Link>
          </p>
        )}
      </section>
    </main>
  )
}

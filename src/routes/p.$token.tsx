import { Link, createFileRoute } from '@tanstack/react-router'
import {
  getPublicPlaylist,
  type PublicPlaylist,
  type PublicPlaylistResult,
} from '@/server-fns/share'

/**
 * Public preview of a shared playlist. Phase 1.g.2 of the WaveFlow
 * roadmap.
 *
 * The route is intentionally NOT nested under `_authed` — the
 * opaque token in the path is the only credential needed. The
 * server-fn `getPublicPlaylist` calls
 * `/api/v1/share/playlists/{token}` (unauthenticated) and returns a
 * discriminated union so the route can render distinct ok / 404 /
 * transient-error states.
 *
 * SSR + `head()` together produce real Open Graph + Twitter Card
 * meta tags for social previews — bots that don't run JavaScript
 * still see the playlist's title, description and cover hash.
 * Once Phase 1.g.2b ships server-side `playlist_track`
 * materialisation, the same page upgrades to render the track list
 * without a route change.
 */
export const Route = createFileRoute('/p/$token')({
  loader: async ({ params }): Promise<PublicPlaylistResult> => {
    return getPublicPlaylist({ data: params.token })
  },
  head: ({ loaderData }) => {
    if (!loaderData || loaderData.kind === 'not_found') {
      return {
        meta: [{ title: 'Playlist not found · WaveFlow' }],
      }
    }
    if (loaderData.kind === 'error') {
      // Transient upstream failure — distinct from 404 so the
      // social preview / browser tab matches what `ErrorPanel`
      // actually renders instead of falsely claiming the playlist
      // is gone.
      return {
        meta: [{ title: 'Server error · WaveFlow' }],
      }
    }
    const { playlist } = loaderData
    const title = `${playlist.name} · WaveFlow`
    const description =
      playlist.description ??
      `A shared playlist on WaveFlow${
        playlist.tracks.length > 0 ? ` — ${playlist.tracks.length} tracks` : ''
      }.`
    return {
      meta: [
        { title },
        { name: 'description', content: description },
        // Open Graph for Facebook / iMessage / Discord / Slack.
        { property: 'og:type', content: 'music.playlist' },
        { property: 'og:title', content: playlist.name },
        { property: 'og:description', content: description },
        { property: 'og:site_name', content: 'WaveFlow' },
        // Twitter / X cards. `summary_large_image` would also need a
        // hosted cover URL — wired in once the server-side artwork
        // pipeline is live (cover_hash is the BLAKE3 reference,
        // it's not a public URL yet).
        { name: 'twitter:card', content: 'summary' },
        { name: 'twitter:title', content: playlist.name },
        { name: 'twitter:description', content: description },
      ],
    }
  },
  component: PublicPlaylistView,
})

function PublicPlaylistView() {
  const data = Route.useLoaderData()

  if (data.kind === 'not_found') {
    return <NotFoundPanel />
  }
  if (data.kind === 'error') {
    return <ErrorPanel message={data.message} />
  }
  return <PlaylistPanel playlist={data.playlist} />
}

/**
 * Map a stored `color_id` to the tile background + foreground
 * classes. The desktop palette is the source of truth
 * ([`PLAYLIST_COLORS`](https://github.com/InstaZDLL/WaveFlow/blob/main/src/lib/playlistVisuals.ts));
 * mirrored here as a switch so Tailwind's static scanner sees
 * every concrete class name. Unknown ids fall back to violet —
 * same default the desktop applies when reading a `color_id` it
 * doesn't recognise.
 */
function colorTileClass(colorId: string): string {
  switch (colorId) {
    case 'emerald':
      return 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300'
    case 'sky':
      return 'bg-sky-100 text-sky-700 dark:bg-sky-950/60 dark:text-sky-300'
    case 'amber':
      return 'bg-amber-100 text-amber-700 dark:bg-amber-950/60 dark:text-amber-300'
    case 'rose':
      return 'bg-rose-100 text-rose-700 dark:bg-rose-950/60 dark:text-rose-300'
    case 'purple':
      return 'bg-purple-100 text-purple-700 dark:bg-purple-950/60 dark:text-purple-300'
    case 'pink':
      return 'bg-pink-100 text-pink-700 dark:bg-pink-950/60 dark:text-pink-300'
    case 'teal':
      return 'bg-teal-100 text-teal-700 dark:bg-teal-950/60 dark:text-teal-300'
    case 'orange':
      return 'bg-orange-100 text-orange-700 dark:bg-orange-950/60 dark:text-orange-300'
    case 'lime':
      return 'bg-lime-100 text-lime-700 dark:bg-lime-950/60 dark:text-lime-300'
    case 'violet':
    default:
      return 'bg-violet-100 text-violet-700 dark:bg-violet-950/60 dark:text-violet-300'
  }
}

function PlaylistPanel({ playlist }: { playlist: PublicPlaylist }) {
  // First code point of the playlist name — used as the cover
  // overlay until `cover_hash` is exposed as a public artwork URL
  // by the server-side artwork pipeline (a separate Phase 1.g.x).
  // `Array.from` iterates by code point (not UTF-16 code unit), so
  // a name like "🎵 Mix" still gives "🎵" instead of a lone
  // surrogate / replacement character.
  const initial = Array.from(playlist.name.trim())[0]?.toUpperCase() ?? '♪'
  const tileClass = colorTileClass(playlist.color_id)
  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Shared playlist</p>
        <div
          role="img"
          aria-label={`Cover for ${playlist.name}`}
          className={`mb-5 flex h-32 w-32 items-center justify-center rounded-2xl sm:h-40 sm:w-40 ${tileClass}`}
        >
          <span className="text-5xl font-bold leading-none sm:text-6xl">{initial}</span>
        </div>
        <h1 className="display-title mb-3 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          {playlist.name}
        </h1>
        {playlist.description && (
          <p className="mb-6 text-base text-[var(--sea-ink-soft)]">{playlist.description}</p>
        )}

        {playlist.tracks.length === 0 ? (
          <p className="mb-6 text-sm text-[var(--sea-ink-soft)]">
            Track list preview is not available yet. The playlist owner can still see the full
            content in WaveFlow Desktop.
          </p>
        ) : (
          <ol className="mb-6 list-decimal space-y-1 pl-6 text-sm text-[var(--sea-ink)]">
            {playlist.tracks.map((track, idx) => (
              <li key={idx}>
                {track.title}
                {track.artist && (
                  <span className="text-[var(--sea-ink-soft)]"> — {track.artist}</span>
                )}
              </li>
            ))}
          </ol>
        )}

        <p>
          <Link to="/" className="text-sm text-[var(--sea-ink-soft)] underline">
            ← What is WaveFlow?
          </Link>
        </p>
      </section>
    </main>
  )
}

function NotFoundPanel() {
  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Playlist not found</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          This share link is no longer active.
        </h1>
        <p className="mb-6 text-base text-[var(--sea-ink-soft)]">
          The playlist may have been made private, or the link was mistyped.
        </p>
        <p>
          <Link to="/" className="text-sm text-[var(--sea-ink-soft)] underline">
            ← Go to WaveFlow
          </Link>
        </p>
      </section>
    </main>
  )
}

function ErrorPanel({ message }: { message: string }) {
  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Couldn&apos;t load this playlist</p>
        <h1 className="display-title mb-4 text-2xl font-bold text-[var(--sea-ink)] sm:text-3xl">
          {message}
        </h1>
        <p>
          <Link to="/" className="text-sm text-[var(--sea-ink-soft)] underline">
            ← Go to WaveFlow
          </Link>
        </p>
      </section>
    </main>
  )
}

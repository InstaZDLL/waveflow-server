import { Link, createFileRoute } from '@tanstack/react-router'
import {
  getPublicPlaylist,
  type PublicPlaylist,
  type PublicPlaylistResult,
} from '@/server-fns/share'
import { formatDuration, formatTrackCountAndRuntime, getCanonicalOrigin } from '@/lib/share-format'

/**
 * Public preview of a shared playlist. Phase 1.g.2 of the WaveFlow
 * roadmap.
 *
 * The route is intentionally NOT nested under `_authed` — the
 * opaque token in the path is the only credential needed. The
 * server-fn `getPublicPlaylist` calls
 * `/api/v1/share/playlists/{token}` (unauthenticated) and returns a
 * discriminated union (`ok` / `error`) for the "reached upstream"
 * path; the 404 case bubbles out-of-band as a thrown `notFound()`
 * so the router emits a real HTTP 404 + renders
 * `notFoundComponent` instead of forcing the TanStack RPC client
 * to swallow a non-OK HTTP response.
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
    // The server-fn throws `notFound()` for 404 cases — that
    // propagates through the router and gets handled by
    // `notFoundComponent` below. Here we just surface the
    // ok / error union for the component to render.
    return getPublicPlaylist({ data: params.token })
  },
  notFoundComponent: NotFoundPanel,
  head: ({ loaderData, params }) => {
    if (!loaderData) {
      // Loader threw `notFound()` — `loaderData` is absent and
      // the router will render `notFoundComponent`. Mirror that
      // in the SSR title so crawlers / browser tab match.
      return {
        meta: [{ title: 'Playlist not found · WaveFlow' }],
      }
    }
    if (loaderData.kind === 'error') {
      // Transient upstream failure — distinct title so the
      // social preview / browser tab matches what `ErrorPanel`
      // actually renders.
      return {
        meta: [{ title: 'Server error · WaveFlow' }],
      }
    }
    const { playlist } = loaderData
    const title = `${playlist.name} · WaveFlow`
    // Same `X tracks · 32 min` helper the in-page header renders —
    // social previews stay consistent with the actual rendered page,
    // so a Discord embed reads the same way as the open tab.
    const description =
      playlist.description ??
      (playlist.tracks.length > 0
        ? `A shared playlist on WaveFlow — ${formatTrackCountAndRuntime(playlist.tracks)}.`
        : 'A shared playlist on WaveFlow.')
    // Canonical share URL (issue #21). Built from the deployment's
    // `BETTER_AUTH_URL` (already the source of truth for the web
    // origin — set both in dev .env and in prod / preview deploys)
    // with a `waveflow.app` fallback so a misconfigured deploy
    // still hands social scrapers SOMETHING resolvable. `head()`
    // runs SSR-side at the moment a crawler hits the page, so
    // `process.env` is available; the value is read inline rather
    // than captured in a const because `head()` re-runs per request
    // and a closure-captured value would freeze on the first SSR.
    const canonicalUrl = `${getCanonicalOrigin()}/p/${params.token}`
    return {
      meta: [
        { title },
        { name: 'description', content: description },
        // Open Graph for Facebook / iMessage / Discord / Slack.
        { property: 'og:type', content: 'music.playlist' },
        { property: 'og:title', content: playlist.name },
        { property: 'og:description', content: description },
        { property: 'og:site_name', content: 'WaveFlow' },
        // og:url pins the canonical share URL so scrapers stop
        // confusing the share path with whatever referrer they
        // landed on. Mirror on twitter:url for the Twitter / X
        // card surface.
        { property: 'og:url', content: canonicalUrl },
        // Twitter / X cards. `summary_large_image` would also need a
        // hosted cover URL — wired in once the server-side artwork
        // pipeline is live (cover_hash is the BLAKE3 reference,
        // it's not a public URL yet).
        { name: 'twitter:card', content: 'summary' },
        { name: 'twitter:title', content: playlist.name },
        { name: 'twitter:description', content: description },
        { name: 'twitter:url', content: canonicalUrl },
      ],
      links: [
        // Canonical link tag — the HTML-standard equivalent of
        // og:url, picked up by Google's index + Facebook's debugger.
        { rel: 'canonical', href: canonicalUrl },
      ],
    }
  },
  component: PublicPlaylistView,
})

function PublicPlaylistView() {
  const data = Route.useLoaderData()
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
  const hasTracks = playlist.tracks.length > 0
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
          <p className="mb-4 text-base text-[var(--sea-ink-soft)]">{playlist.description}</p>
        )}
        {hasTracks && (
          <p className="mb-6 text-sm text-[var(--sea-ink-soft)]">
            {formatTrackCountAndRuntime(playlist.tracks)}
          </p>
        )}

        {!hasTracks ? (
          <p className="mb-6 text-sm text-[var(--sea-ink-soft)]">
            Track list preview is not available yet. The playlist owner can still see the full
            content in WaveFlow Desktop.
          </p>
        ) : (
          // Ordered list styled with flex rows + a fixed-width
          // position column + `tabular-nums` on the duration cell, so
          // the duration column aligns across rows the way a `<table>`
          // would — without losing the semantic "ordered sequence"
          // affordance that screen readers announce on `<ol>` /
          // `<li>`. `tabular-nums` keeps `5:21` and `12:03` the same
          // pixel width on a font that defaults to proportional
          // digits, so the right-aligned column doesn't jitter as the
          // eye scans down. The visible position number lives in its
          // own `<span>` (rather than relying on the browser's
          // automatic `<ol>` markers) because flex layout lets us pin
          // it to a fixed `w-6` cell instead of inheriting the marker
          // box's variable width.
          <ol
            aria-label="Tracks in this playlist"
            className="mb-6 divide-y divide-[var(--sea-ink-soft)]/15 text-sm"
          >
            {playlist.tracks.map((track, idx) => {
              const duration = formatDuration(track.duration_ms)
              return (
                <li key={idx} className="flex items-baseline gap-3 py-2 text-[var(--sea-ink)]">
                  <span className="w-6 shrink-0 text-right tabular-nums text-[var(--sea-ink-soft)]">
                    {idx + 1}
                  </span>
                  <span className="min-w-0 flex-1 truncate">
                    {track.title}
                    {track.artist && (
                      <span className="text-[var(--sea-ink-soft)]"> — {track.artist}</span>
                    )}
                  </span>
                  {duration && (
                    <span className="shrink-0 tabular-nums text-[var(--sea-ink-soft)]">
                      {duration}
                    </span>
                  )}
                </li>
              )
            })}
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

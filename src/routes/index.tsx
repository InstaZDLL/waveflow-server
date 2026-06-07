import { createFileRoute, Link } from '@tanstack/react-router'
import WaveflowLogo from '@/components/WaveflowLogo'
import { getCurrentSession } from '@/server-fns/session'

export const Route = createFileRoute('/')({
  // SSR-resolve the session so the hero CTA renders the right
  // copy on the first paint. The client-only `useSession()` hook
  // returns `null` during hydration and would otherwise paint
  // the signed-out CTA, then swap to "Open my library" for a
  // signed-in user — a visible flash + a benign hydration
  // mismatch warning. The loader resolves server-side, so the
  // markup the browser receives is already correct.
  loader: async (): Promise<{ isSignedIn: boolean }> => {
    const session = await getCurrentSession()
    return { isSignedIn: !!session }
  },
  component: App,
})

function App() {
  const { isSignedIn } = Route.useLoaderData()

  return (
    <main className="page-wrap px-4 pb-8 pt-14">
      <section className="island-shell rise-in relative overflow-hidden rounded-[2rem] px-6 py-10 sm:px-10 sm:py-14">
        <div
          className="pointer-events-none absolute -left-20 -top-24 h-56 w-56 rounded-full"
          style={{
            background:
              'radial-gradient(circle, color-mix(in oklab, var(--accent-500) 32%, transparent), transparent 66%)',
          }}
        />
        <div
          className="pointer-events-none absolute -bottom-20 -right-20 h-56 w-56 rounded-full"
          style={{
            background:
              'radial-gradient(circle, color-mix(in oklab, var(--accent-700) 18%, transparent), transparent 66%)',
          }}
        />
        <div className="mb-3 flex items-center gap-3">
          <span style={{ color: 'var(--accent-600)' }}>
            <WaveflowLogo size={48} label={null} />
          </span>
          <p className="island-kicker m-0">WaveFlow</p>
        </div>
        <h1 className="display-title mb-5 max-w-3xl text-4xl leading-[1.02] font-bold tracking-tight text-[var(--sea-ink)] sm:text-6xl">
          Your music, your server, every device.
        </h1>
        <p className="mb-8 max-w-2xl text-base text-[var(--sea-ink-soft)] sm:text-lg">
          A self-hostable music library that syncs between the WaveFlow desktop app and the web.
          Your files stay on your server. No ads, no tracking, no recommendations engine looking
          over your shoulder.
        </p>
        <div className="flex flex-wrap gap-3">
          {isSignedIn ? (
            <Link
              to="/profiles"
              className="rounded-full px-5 py-2.5 text-sm font-semibold text-white no-underline transition hover:-translate-y-0.5"
              style={{ backgroundColor: 'var(--accent-600)' }}
            >
              Open my library
            </Link>
          ) : (
            <Link
              to="/sign-up"
              className="rounded-full px-5 py-2.5 text-sm font-semibold text-white no-underline transition hover:-translate-y-0.5"
              style={{ backgroundColor: 'var(--accent-600)' }}
            >
              Create your account
            </Link>
          )}
          <a
            href="https://github.com/InstaZDLL/WaveFlow/releases/latest"
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-full border border-[rgba(23,58,64,0.2)] bg-white/50 px-5 py-2.5 text-sm font-semibold text-[var(--sea-ink)] no-underline transition hover:-translate-y-0.5 hover:border-[rgba(23,58,64,0.35)]"
          >
            Get the desktop app
          </a>
        </div>
      </section>

      <section className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {[
          ['Local-first', 'Music files live on your server. Stream over LAN or open the internet.'],
          [
            'Multi-device sync',
            'Library, playlists, ratings, listening history — synced across desktops in real time.',
          ],
          [
            'Share playlists',
            'Send a public link. Recipients see the tracklist without an account.',
          ],
          [
            'Privacy',
            'No telemetry, no ads. Your listening behaviour never leaves your own machines.',
          ],
        ].map(([title, desc], index) => (
          <article
            key={title}
            className="island-shell feature-card rise-in rounded-2xl p-5"
            style={{ animationDelay: `${index * 90 + 80}ms` }}
          >
            <h2 className="mb-2 text-base font-semibold text-[var(--sea-ink)]">{title}</h2>
            <p className="m-0 text-sm text-[var(--sea-ink-soft)]">{desc}</p>
          </article>
        ))}
      </section>

      <section className="island-shell mt-8 rounded-2xl p-6">
        <p className="island-kicker mb-2">Self-hosted, open source</p>
        <ul className="m-0 list-disc space-y-2 pl-5 text-sm text-[var(--sea-ink-soft)]">
          <li>
            The web client is{' '}
            <a
              href="https://github.com/InstaZDLL/waveflow-web"
              target="_blank"
              rel="noopener noreferrer"
              className="font-semibold text-[var(--sea-ink)] underline"
            >
              AGPL-3.0
            </a>
            ; the desktop app is{' '}
            <a
              href="https://github.com/InstaZDLL/WaveFlow"
              target="_blank"
              rel="noopener noreferrer"
              className="font-semibold text-[var(--sea-ink)] underline"
            >
              GPL-3.0
            </a>
            .
          </li>
          <li>
            Backend is{' '}
            <a
              href="https://github.com/InstaZDLL/waveflow-server"
              target="_blank"
              rel="noopener noreferrer"
              className="font-semibold text-[var(--sea-ink)] underline"
            >
              waveflow-server
            </a>{' '}
            — a single-binary Rust server. Drop it on a Pi, a NAS, or a VPS.
          </li>
          <li>
            Theme the whole app from{' '}
            <Link to="/settings" className="font-semibold text-[var(--sea-ink)] underline">
              Settings → Appearance
            </Link>{' '}
            — 14 OKLCH presets, light or dark.
          </li>
        </ul>
      </section>
    </main>
  )
}

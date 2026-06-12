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
    <main className="page-wrap app-main px-4">
      <section className="panel rise-in relative grid min-h-[72dvh] overflow-hidden rounded-[1.5rem] lg:grid-cols-[1.08fr_0.92fr]">
        <div className="relative z-10 flex flex-col justify-between gap-12 p-6 sm:p-10 lg:p-12">
          <div>
            <div className="mb-7 flex items-center gap-3">
              <span className="art-tile h-12 w-12" style={{ color: 'var(--accent-700)' }}>
                <WaveflowLogo size={30} label={null} />
              </span>
              <p className="section-eyebrow m-0">WaveFlow</p>
            </div>
            <h1 className="section-title max-w-4xl">Your music library, served from home.</h1>
            <p className="mt-6 max-w-[62ch] text-base leading-8 text-[var(--sea-ink-soft)] sm:text-lg">
              Run the Rust server where your files already live. Browse, stream, sync playlists,
              and share selected mixes from any browser without handing your listening habits to a
              recommendation feed.
            </p>
          </div>

          <div className="flex flex-wrap gap-3">
            {isSignedIn ? (
              <Link to="/profiles" className="button button-primary">
                Open library
              </Link>
            ) : (
              <Link to="/sign-up" className="button button-primary">
                Create account
              </Link>
            )}
            <a
              href="https://github.com/InstaZDLL/WaveFlow/releases/latest"
              target="_blank"
              rel="noopener noreferrer"
              className="button button-ghost"
            >
              Desktop app
            </a>
          </div>
        </div>

        <div className="relative min-h-[24rem] border-t border-[var(--line)] p-5 lg:border-l lg:border-t-0">
          <div className="absolute inset-0 bg-[radial-gradient(circle_at_50%_12%,color-mix(in_oklab,var(--accent-500)_22%,transparent),transparent_34%)]" />
          <div className="relative grid h-full grid-rows-[1fr_auto] gap-5">
            <div className="art-tile relative overflow-hidden rounded-[1.25rem] p-6">
              <div className="absolute inset-x-8 top-10 h-28 rounded-full border border-[var(--line)]" />
              <div className="absolute inset-x-14 top-20 h-28 rounded-full border border-[var(--line)]" />
              <div className="relative mt-auto w-full">
                <div className="mb-5 flex items-end gap-2">
                  {[44, 76, 118, 68, 96, 52, 128, 82].map((height, index) => (
                    <span
                      key={index}
                      className="block flex-1 rounded-t-md"
                      style={{
                        height,
                        background:
                          index % 2 === 0
                            ? 'var(--sea-ink)'
                            : 'color-mix(in oklab, var(--accent-600) 74%, var(--sea-ink))',
                        opacity: 0.82,
                      }}
                    />
                  ))}
                </div>
                <div className="rounded-xl border border-[var(--line)] bg-[var(--surface-strong)] p-4">
                  <p className="m-0 text-sm font-bold text-[var(--sea-ink)]">
                    Basement server / Living room browser
                  </p>
                  <p className="mt-1 text-xs text-[var(--sea-ink-soft)]">
                    FLAC streams, playlist sync, public previews
                  </p>
                </div>
              </div>
            </div>
            <dl className="grid grid-cols-3 gap-2 text-center">
              {[
                ['60s', 'signed URLs'],
                ['0', 'ad pixels'],
                ['AGPL', 'web + server'],
              ].map(([value, label]) => (
                <div key={label} className="quiet-panel p-3">
                  <dt className="text-lg font-bold tabular-nums text-[var(--sea-ink)]">{value}</dt>
                  <dd className="mt-1 text-[0.68rem] font-semibold tracking-[0.07em] text-[var(--sea-ink-soft)] uppercase">
                    {label}
                  </dd>
                </div>
              ))}
            </dl>
          </div>
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
            className="feature-card rise-in rounded-2xl p-5"
            style={{ animationDelay: `${index * 90 + 80}ms` }}
          >
            <h2 className="mb-2 text-base font-semibold text-[var(--sea-ink)]">{title}</h2>
            <p className="m-0 text-sm text-[var(--sea-ink-soft)]">{desc}</p>
          </article>
        ))}
      </section>

      <section className="panel panel-pad mt-8">
        <p className="section-eyebrow mb-2">Self-hosted, open source</p>
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

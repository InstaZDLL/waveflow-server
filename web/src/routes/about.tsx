import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/about')({
  component: About,
})

function About() {
  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad">
        <p className="section-eyebrow mb-2">About</p>
        <h1 className="display-title mb-3 text-5xl font-bold text-(--sea-ink)">
          A music library you actually own.
        </h1>
        <p className="m-0 max-w-3xl text-base leading-8 text-(--sea-ink-soft)">
          WaveFlow is a local-first music player. Your files live on a Rust-powered server you run
          yourself — on a Raspberry Pi, an old laptop, or a VPS — and the desktop app + web client
          stream from it. Playlists, ratings, and listening history sync between every device signed
          into the same server.
        </p>
      </section>

      <section className="panel panel-pad mt-6">
        <p className="section-eyebrow mb-4">What you get</p>
        <ul className="m-0 grid list-none gap-3 p-0 text-sm text-(--sea-ink-soft) sm:grid-cols-2">
          <li>
            <article className="quiet-panel h-full p-4">
              <strong className="text-(--sea-ink)">Desktop player</strong>
              <p className="mt-2">
                Tauri + Rust audio engine with lossless playback, gapless, ReplayGain, EQ,
                crossfade, DSD support, and optional WASAPI exclusive on Windows.
              </p>
            </article>
          </li>
          <li>
            <article className="quiet-panel h-full p-4">
              <strong className="text-(--sea-ink)">Web client</strong>
              <p className="mt-2">
                Browse your library, edit playlists, and manage your profile from any browser.
              </p>
            </article>
          </li>
          <li>
            <article className="quiet-panel h-full p-4">
              <strong className="text-(--sea-ink)">Self-host backend</strong>
              <p className="mt-2">
                A single Rust binary serves streaming, sync, OAuth, and the API surface.
              </p>
            </article>
          </li>
          <li>
            <article className="quiet-panel h-full p-4">
              <strong className="text-(--sea-ink)">Multi-profile</strong>
              <p className="mt-2">
                One server can host multiple users, each with their own library and sync stream.
              </p>
            </article>
          </li>
          <li>
            <article className="quiet-panel h-full p-4 sm:col-span-2">
              <strong className="text-(--sea-ink)">Open source</strong>
              <p className="mt-2">
                Web client AGPL-3.0, desktop GPL-3.0, server AGPL-3.0. Modify, fork, and
                redistribute within the licence terms.
              </p>
            </article>
          </li>
        </ul>
      </section>

      <section className="panel panel-pad mt-6">
        <p className="section-eyebrow mb-2">Where things stand</p>
        <p className="m-0 text-sm leading-7 text-(--sea-ink-soft)">
          The web client is in active development — sign-in, profile + library browsing, basic
          playback, and public playlist sharing all work today. The full PlayerBar, Now Playing
          overlay, and playlist editing land in the next sprints. Track the roadmap in{' '}
          <a
            href="https://github.com/InstaZDLL/WaveFlow/tree/main/docs/rfcs"
            target="_blank"
            rel="noreferrer"
            className="font-semibold text-(--sea-ink) underline"
          >
            the project RFCs
          </a>
          .
        </p>
      </section>
    </main>
  )
}

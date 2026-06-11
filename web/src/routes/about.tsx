import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/about')({
  component: About,
})

function About() {
  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">About</p>
        <h1 className="display-title mb-3 text-4xl font-bold text-[var(--sea-ink)] sm:text-5xl">
          A music library you actually own.
        </h1>
        <p className="m-0 max-w-3xl text-base leading-8 text-[var(--sea-ink-soft)]">
          WaveFlow is a local-first music player. Your files live on a Rust-powered server you run
          yourself — on a Raspberry Pi, an old laptop, or a VPS — and the desktop app + web client
          stream from it. Playlists, ratings, and listening history sync between every device signed
          into the same server.
        </p>
      </section>

      <section className="island-shell mt-6 rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">What you get</p>
        <ul className="m-0 list-disc space-y-2 pl-5 text-sm text-[var(--sea-ink-soft)]">
          <li>
            <strong className="text-[var(--sea-ink)]">Desktop player</strong> — Tauri + Rust audio
            engine. Lossless playback, gapless, ReplayGain, 6-band EQ, crossfade, DSD support,
            optional WASAPI exclusive on Windows. Spotify / Apple Music inspired UI.
          </li>
          <li>
            <strong className="text-[var(--sea-ink)]">Web client</strong> — this app. Browse your
            library, edit playlists, manage your profile from any browser.
          </li>
          <li>
            <strong className="text-[var(--sea-ink)]">Self-host backend</strong> — a single Rust
            binary serves the streaming HTTP API, sync, OAuth, and (soon) a community metadata pool.
          </li>
          <li>
            <strong className="text-[var(--sea-ink)]">Multi-profile</strong> — one server hosts
            multiple users; each gets their own library + sync stream. Family / roommates friendly.
          </li>
          <li>
            <strong className="text-[var(--sea-ink)]">Open source</strong> — web client AGPL-3.0,
            desktop GPL-3.0, server AGPL-3.0. Modify, fork, redistribute — within the licence&apos;s
            network-clause terms for AGPL pieces.
          </li>
        </ul>
      </section>

      <section className="island-shell mt-6 rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Where things stand</p>
        <p className="m-0 text-sm leading-7 text-[var(--sea-ink-soft)]">
          The web client is in active development — sign-in, profile + library browsing, basic
          playback, and public playlist sharing all work today. The full PlayerBar, Now Playing
          overlay, and playlist editing land in the next sprints. Track the roadmap in{' '}
          <a
            href="https://github.com/InstaZDLL/WaveFlow/tree/main/docs/rfcs"
            target="_blank"
            rel="noreferrer"
            className="font-semibold text-[var(--sea-ink)] underline"
          >
            the project RFCs
          </a>
          .
        </p>
      </section>
    </main>
  )
}

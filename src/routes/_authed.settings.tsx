import { createFileRoute } from '@tanstack/react-router'

import { ThemePicker } from '@/components/ThemePicker'

export const Route = createFileRoute('/_authed/settings')({
  component: SettingsPage,
})

// Exported for direct unit-render — sidesteps the file-route + router
// shell so a vitest spec can mount the picker in isolation.
export function SettingsPage() {
  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell mx-auto max-w-3xl rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Settings</p>
        <h1 className="display-title mb-6 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Appearance
        </h1>
        <p className="mb-6 text-sm text-[var(--sea-ink-soft)]">
          Pick a theme. Your choice is stored as a cookie so the next page render already paints the
          right palette — no flash of the brand colour while React hydrates.
        </p>
        <ThemePicker />
      </section>
    </main>
  )
}

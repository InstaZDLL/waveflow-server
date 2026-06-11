import { HeadContent, Scripts, createRootRoute } from '@tanstack/react-router'
import { TanStackRouterDevtoolsPanel } from '@tanstack/react-router-devtools'
import { TanStackDevtools } from '@tanstack/react-devtools'
import { findTheme } from '@waveflow/design-tokens'
import Footer from '../components/Footer'
import Header from '../components/Header'
import { PlayerBar } from '../components/PlayerBar'
import { ThemeProvider } from '../components/ThemeProvider'
import { ThemeStyle } from '../components/ThemeStyle'
import { PlayerProvider } from '../lib/player-context'
import { getStoredThemeId } from '../server-fns/theme'

import appCss from '../styles.css?url'

// Legacy light/dark bootstrap — preserved as a transitional shim so
// the existing `dark:` Tailwind variants stay in sync with the
// data-theme attribute. The DS-driven theme system reads the cookie
// instead of localStorage and applies the full accent palette on
// top; the two coexist so this PR doesn't have to also gut every
// `dark:bg-black/30` in the codebase.
const THEME_INIT_SCRIPT = `(function(){try{var stored=window.localStorage.getItem('theme');var mode=(stored==='light'||stored==='dark'||stored==='auto')?stored:'auto';var prefersDark=window.matchMedia('(prefers-color-scheme: dark)').matches;var resolved=mode==='auto'?(prefersDark?'dark':'light'):mode;var root=document.documentElement;root.classList.remove('light','dark');root.classList.add(resolved);if(mode==='auto'){root.removeAttribute('data-theme')}else{root.setAttribute('data-theme',mode)}root.style.colorScheme=resolved;}catch(e){}})();`

export const Route = createRootRoute({
  // SSR loader: resolve the stored theme cookie BEFORE the shell
  // renders, so the inline `<style>:root { --accent-* }</style>`
  // injection paints with the right palette on the first byte.
  // `findTheme` validates the id; an unknown cookie value falls
  // back to the default so a tampered cookie can never raise to
  // the React tree.
  loader: async () => {
    const themeId = await getStoredThemeId()
    return { themeId: findTheme(themeId).id }
  },
  head: () => ({
    meta: [
      {
        charSet: 'utf-8',
      },
      {
        name: 'viewport',
        content: 'width=device-width, initial-scale=1',
      },
      {
        title: 'WaveFlow — your music, your server, every device',
      },
      {
        name: 'description',
        content:
          'Self-hostable music library that syncs between the WaveFlow desktop app and the web. Your files stay on your server.',
      },
    ],
    links: [
      {
        rel: 'stylesheet',
        href: appCss,
      },
    ],
  }),
  shellComponent: RootDocument,
})

function RootDocument({ children }: { children: React.ReactNode }) {
  const { themeId } = Route.useLoaderData()
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: THEME_INIT_SCRIPT }} />
        <ThemeStyle themeId={themeId} />
        <HeadContent />
      </head>
      <body className="font-sans antialiased [overflow-wrap:anywhere] selection:bg-[rgba(79,184,178,0.24)]">
        <ThemeProvider initialThemeId={themeId}>
          <PlayerProvider>
            <Header />
            {children}
            <Footer />
            <PlayerBar />
          </PlayerProvider>
        </ThemeProvider>
        <TanStackDevtools
          config={{
            position: 'bottom-right',
          }}
          plugins={[
            {
              name: 'Tanstack Router',
              render: <TanStackRouterDevtoolsPanel />,
            },
          ]}
        />
        <Scripts />
      </body>
    </html>
  )
}

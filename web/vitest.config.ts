// Vitest config — pulls in the same Vite plugin chain as the runtime
// build, plus jsdom for component tests. The default Vitest config
// would try to load `react/index.js` as ESM and trip the CommonJS
// interop guard (`ReferenceError: module is not defined`); routing
// through the React plugin lets Vitest see the JSX modules in their
// transformed form.
//
// `passWithNoTests: false` makes Vitest fail when no tests match the
// `include` globs. The previous `true` was useful during the
// bootstrap window where the suite was empty, but now that 230+
// tests are landing it's a footgun — a typo in the include pattern
// (or an accidental file move that breaks discovery) would pass CI
// silently. CI failing loud is the right signal.

import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'
import viteReact from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [viteReact()],
  // Mirror the `@/*` -> `src/*` mapping declared in `tsconfig.json`.
  // The runtime build picks this up via `resolve: { tsconfigPaths: true }`
  // in `vite.config.ts`; Vitest doesn't load that config, so we wire
  // the alias here too.
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    passWithNoTests: false,
    // Pick up tests from the web app AND every workspace package.
    // Workspace packages (`packages/*`) ship their own suites and
    // run against the same jsdom env so a DOM helper like
    // `applyTheme` doesn't need a per-package vitest config.
    include: ['src/**/*.test.{ts,tsx}', 'packages/*/src/**/*.test.{ts,tsx}'],
    // Vitest 4's per-test timeout default is 5s; the React Testing
    // Library waits routinely push that on a cold runner. Bump to
    // 10s to match the prior scaffold convention.
    testTimeout: 10_000,
  },
})

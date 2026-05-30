// Vitest config — pulls in the same Vite plugin chain as the runtime
// build, plus jsdom for component tests. The default Vitest config
// would try to load `react/index.js` as ESM and trip the CommonJS
// interop guard (`ReferenceError: module is not defined`); routing
// through the React plugin lets Vitest see the JSX modules in their
// transformed form.
//
// `passWithNoTests` keeps the CI green during the bootstrap window
// where we haven't authored any tests yet — once `src/**/*.test.tsx`
// files start landing this flag becomes a no-op.

import { defineConfig } from 'vitest/config'
import viteReact from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [viteReact()],
  test: {
    environment: 'jsdom',
    globals: true,
    passWithNoTests: true,
    // Vitest 4's per-test timeout default is 5s; the React Testing
    // Library waits routinely push that on a cold runner. Bump to
    // 10s to match the prior scaffold convention.
    testTimeout: 10_000,
  },
})

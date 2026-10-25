import { defineConfig, devices } from "@playwright/test";

// Another port when several checkouts run their suites at once: a preview
// already listening is reused, and it may be serving another tree's build.
const port = Number(process.env.WAVEFLOW_E2E_PORT ?? 4173);

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: "retain-on-failure",
    // The production build registers a service worker (RFC-022, offline
    // listening). Blocked here so that every mock answers what the page asks,
    // as it did before the worker existed; the worker's own rules are tested
    // by `src/lib/service-worker.test.ts`, and the offline page falls back
    // to the cache without it.
    serviceWorkers: "block",
  },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "mobile", use: { ...devices["Pixel 7"] } },
  ],
  webServer: {
    command: `bun run preview --host 127.0.0.1 --port ${port}`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: !process.env.CI,
  },
});

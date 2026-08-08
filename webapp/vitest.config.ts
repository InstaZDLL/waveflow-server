import { defineConfig } from "vitest/config";

// The design-token helpers write onto document.documentElement, so the suite
// needs a DOM. passWithNoTests stays false: a broken include glob must fail
// loudly rather than report an empty run as success.
export default defineConfig({
  test: {
    environment: "jsdom",
    passWithNoTests: false,
    include: ["src/**/*.test.ts"],
  },
});

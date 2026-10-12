import { defineConfig } from "vitest/config";

// The design-token helpers write onto document.documentElement, so the suite
// needs a DOM. passWithNoTests stays false: a broken include glob must fail
// loudly rather than report an empty run as success.
export default defineConfig({
  test: {
    environment: "jsdom",
    passWithNoTests: false,
    include: ["src/**/*.test.ts"],
    // Vitest hands a CSS import back as an empty string by default, which is
    // sound for a component test and wrong for a test *about* the stylesheet:
    // the scale check read "" and passed over nothing. The sheet needs no
    // dependency to be read this way — `?raw` is Vite's own — but it does need
    // the default turned off.
    css: true,
  },
});

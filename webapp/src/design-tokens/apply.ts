// `applyTheme` — write a theme preset's CSS variables onto the
// document root + flip the legacy `dark` class. Idempotent. Pure
// DOM, no React imports, so the function works equally well from a
// bootstrap script in `index.html` (avoiding the dark-default white
// flash) and from a React effect after hydration.
//
// Ported from the desktop's `applyTheme` in `src/lib/themes.ts`.
// Web ambient defaults differ from desktop ones — the web app has
// its own background canvas — but a theme preset overrides them
// anyway, so the divergence only matters for the no-ambient
// `default` / `default-dark` pair.

import type { ThemePreset } from "./themes";

const DEFAULT_SURFACE_DARK = "#121212";
const DEFAULT_SURFACE_DARK_ELEVATED = "#181818";
const DEFAULT_SURFACE_LIGHT = "#ffffff";
const DEFAULT_SURFACE_LIGHT_ELEVATED = "#fafafa";

/**
 * Apply a preset to the document root. Re-applies every token even
 * when the preset doesn't override one — switching FROM a custom
 * theme BACK to the default needs to clear the lingering override,
 * which a `setProperty` of the default does.
 *
 * SSR-safe: no-op when `document` is undefined (no `globalThis`
 * cast needed — TS narrows `document` for us through `typeof`).
 */
export function applyTheme(theme: ThemePreset): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  for (const [shade, value] of Object.entries(theme.accent)) {
    root.style.setProperty(`--accent-${shade}`, value);
  }
  root.style.setProperty(
    "--ambient-bg",
    theme.ambient ?? (theme.mode === "dark" ? "#121212" : "#ffffff"),
  );
  root.style.setProperty(
    "--color-surface-dark",
    theme.surfaceDark ?? DEFAULT_SURFACE_DARK,
  );
  root.style.setProperty(
    "--color-surface-dark-elevated",
    theme.surfaceDarkElevated ?? DEFAULT_SURFACE_DARK_ELEVATED,
  );
  root.style.setProperty(
    "--color-surface-light",
    theme.surfaceLight ?? DEFAULT_SURFACE_LIGHT,
  );
  root.style.setProperty(
    "--color-surface-light-elevated",
    theme.surfaceLightElevated ?? DEFAULT_SURFACE_LIGHT_ELEVATED,
  );
  root.setAttribute("data-theme", theme.id);
  if (theme.mode === "dark") {
    root.classList.add("dark");
  } else {
    root.classList.remove("dark");
  }
}

/**
 * Serialize a preset to a `<style>` rule body — useful for SSR-side
 * theme injection (write the resolved theme into the document
 * `<head>` so the first paint already has the right tint, no FOUC).
 * Consumers wrap the return in `:root { ... }` themselves; the
 * helper stays scope-free so a `[data-theme="..."]` selector can
 * also reuse it.
 */
export function themeCssDeclarations(theme: ThemePreset): string {
  const lines: string[] = [];
  for (const [shade, value] of Object.entries(theme.accent)) {
    lines.push(`--accent-${shade}: ${value};`);
  }
  lines.push(
    `--ambient-bg: ${theme.ambient ?? (theme.mode === "dark" ? "#121212" : "#ffffff")};`,
  );
  lines.push(
    `--color-surface-dark: ${theme.surfaceDark ?? DEFAULT_SURFACE_DARK};`,
  );
  lines.push(
    `--color-surface-dark-elevated: ${theme.surfaceDarkElevated ?? DEFAULT_SURFACE_DARK_ELEVATED};`,
  );
  lines.push(
    `--color-surface-light: ${theme.surfaceLight ?? DEFAULT_SURFACE_LIGHT};`,
  );
  lines.push(
    `--color-surface-light-elevated: ${theme.surfaceLightElevated ?? DEFAULT_SURFACE_LIGHT_ELEVATED};`,
  );
  return lines.join("\n  ");
}

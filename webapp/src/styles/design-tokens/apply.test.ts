// `applyTheme` is a DOM-side effect — the test runs under jsdom so
// `document.documentElement` is real. `themeCssDeclarations` is a
// pure string builder; we exercise both here so a future change to
// the surface tokens diff fails loudly in CI.

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { applyTheme, themeCssDeclarations, themeSurfaces } from "./apply";
import { findTheme, THEME_PRESETS } from "./themes";

describe("applyTheme", () => {
  beforeEach(() => {
    // Reset the root so a prior test's accent leak doesn't surface
    // as a false pass here.
    const root = document.documentElement;
    root.removeAttribute("style");
    root.removeAttribute("data-theme");
    root.classList.remove("dark");
  });

  afterEach(() => {
    document.documentElement.removeAttribute("style");
    document.documentElement.removeAttribute("data-theme");
    document.documentElement.classList.remove("dark");
  });

  it("writes the full 11-shade accent scale onto the root", () => {
    applyTheme(findTheme("lavender"));
    const root = document.documentElement;
    expect(root.style.getPropertyValue("--accent-50")).toMatch(/^oklch\(/);
    expect(root.style.getPropertyValue("--accent-500")).toMatch(/^oklch\(/);
    expect(root.style.getPropertyValue("--accent-950")).toMatch(/^oklch\(/);
  });

  it("sets data-theme + the dark class for dark presets", () => {
    applyTheme(findTheme("lavender"));
    expect(document.documentElement.getAttribute("data-theme")).toBe(
      "lavender",
    );
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("removes the dark class when switching to a light preset", () => {
    applyTheme(findTheme("lavender"));
    applyTheme(findTheme("lavender-light"));
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(document.documentElement.getAttribute("data-theme")).toBe(
      "lavender-light",
    );
  });

  it("clears overrides when going back to the default preset", () => {
    // Lavender overrides --color-surface-dark; flipping back to the
    // unstyled default must reset it, not leave the violet behind.
    applyTheme(findTheme("lavender"));
    expect(
      document.documentElement.style.getPropertyValue("--color-surface-dark"),
    ).toBe("#15101e");
    applyTheme(findTheme("default-dark"));
    expect(
      document.documentElement.style.getPropertyValue("--color-surface-dark"),
    ).toBe("#121212");
  });
});

describe("themeCssDeclarations", () => {
  it("emits accent shades + surface tokens for every preset", () => {
    for (const preset of THEME_PRESETS) {
      const css = themeCssDeclarations(preset);
      expect(css).toContain("--accent-500: oklch(");
      expect(css).toContain("--color-surface-dark:");
      expect(css).toContain("--color-surface-light:");
      expect(css).toContain("--ambient-bg:");
    }
  });

  it("falls back to defaults when the preset leaves a surface unset", () => {
    const def = findTheme("default-dark");
    const css = themeCssDeclarations(def);
    expect(css).toContain("--color-surface-dark: #121212;");
    expect(css).toContain("--color-surface-dark-elevated: #181818;");
    expect(css).toContain("--color-surface-light: #ffffff;");
  });
});

describe("themeSurfaces", () => {
  afterEach(() => {
    document.documentElement.removeAttribute("style");
  });

  it("names the surfaces applyTheme writes for the preset's own mode", () => {
    // The picker's miniatures draw from these; a card that disagreed with
    // the theme it previews would show a colour the theme never paints.
    const root = document.documentElement;
    for (const preset of THEME_PRESETS) {
      applyTheme(preset);
      const prefix = preset.mode === "dark" ? "dark" : "light";
      const surfaces = themeSurfaces(preset);
      expect(surfaces.ground).toBe(root.style.getPropertyValue("--ambient-bg"));
      expect(surfaces.surface).toBe(
        root.style.getPropertyValue(`--color-surface-${prefix}`),
      );
      expect(surfaces.raised).toBe(
        root.style.getPropertyValue(`--color-surface-${prefix}-elevated`),
      );
    }
  });
});

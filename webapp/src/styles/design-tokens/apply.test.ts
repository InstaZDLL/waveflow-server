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

  it("is what applyTheme hands the dark material for a dark preset", () => {
    // The picker's miniatures draw from these and the page's surfaces read
    // them: a card that disagreed would show a colour the page never paints.
    const root = document.documentElement;
    for (const preset of THEME_PRESETS.filter((t) => t.mode === "dark")) {
      applyTheme(preset);
      const surfaces = themeSurfaces(preset);
      expect(root.style.getPropertyValue("--theme-ground")).toBe(
        surfaces.ground,
      );
      expect(root.style.getPropertyValue("--theme-surface")).toBe(
        surfaces.surface,
      );
      expect(root.style.getPropertyValue("--theme-raised")).toBe(
        surfaces.raised,
      );
    }
  });

  it("takes each dark preset's own surfaces, lifting the panel a step", () => {
    const midnight = themeSurfaces(findTheme("midnight"));
    expect(midnight.ground).toBe("#0b1020");
    expect(midnight.surface).toBe("color-mix(in srgb, #0b1020 97%, white)");
    expect(midnight.raised).toBe("#141a2e");
    // OLED keeps its true black ground.
    expect(themeSurfaces(findTheme("oled")).ground).toBe("#000000");
  });

  it("keeps the material's own near-black for a preset that names none", () => {
    expect(themeSurfaces(findTheme("default-dark"))).toEqual({
      ground: "#0b0b0e",
      surface: "#121216",
      raised: "#17171d",
    });
  });

  it("clears the dark surfaces when a light preset follows a dark one", () => {
    const root = document.documentElement;
    applyTheme(findTheme("midnight"));
    applyTheme(findTheme("midnight-light"));
    for (const property of [
      "--theme-ground",
      "--theme-surface",
      "--theme-raised",
    ])
      expect(root.style.getPropertyValue(property)).toBe("");
  });

  it("names a light preset's raw tokens, as applyTheme writes them", () => {
    const root = document.documentElement;
    for (const preset of THEME_PRESETS.filter((t) => t.mode === "light")) {
      applyTheme(preset);
      const surfaces = themeSurfaces(preset);
      expect(surfaces.ground).toBe(root.style.getPropertyValue("--ambient-bg"));
      expect(surfaces.surface).toBe(
        root.style.getPropertyValue("--color-surface-light"),
      );
      expect(surfaces.raised).toBe(
        root.style.getPropertyValue("--color-surface-light-elevated"),
      );
    }
  });
});

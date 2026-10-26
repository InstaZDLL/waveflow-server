import postcss from "postcss";
import { describe, expect, it } from "vitest";

import { fluid, refresh, styles } from "../sheets";
import { applyTheme } from "./apply";
import {
  composite,
  contrastRatio,
  mixSrgb,
  parseColor,
  type Rgba,
  toHex,
} from "./contrast";
import { THEME_PRESETS, type ThemePreset } from "./themes";

/** The sheets in the order `main.tsx` imports them: later wins a tie. */
const SHEETS = [styles, refresh, fluid];

type Declaration = {
  name: string;
  value: string;
  weight: number;
  order: number;
};

/**
 * Every custom property a top-level `:root…` rule declares, with the state
 * the rule asks of the root: `.dark`, `:not(.dark)`, `[data-theme="…"]`.
 * Only those rules define tokens; a rule with a descendant (`:root .a`) or
 * one inside `@media` is not a token block and is skipped.
 */
const ROOT_SELECTOR =
  /^:root((?:\.dark|:not\(\.dark\)|\[data-theme="[^"]+"\])*)$/;

function tokenDeclarations(): Array<
  Declaration & { dark: boolean | null; theme: string | null }
> {
  const found: Array<
    Declaration & { dark: boolean | null; theme: string | null }
  > = [];
  let order = 0;
  for (const sheet of SHEETS) {
    const root = postcss.parse(sheet);
    root.each((node) => {
      if (node.type !== "rule") return;
      for (const selector of node.selectors) {
        const match = selector.trim().match(ROOT_SELECTOR);
        if (!match) continue;
        const qualifiers = match[1] ?? "";
        const dark = qualifiers.includes(":not(.dark)")
          ? false
          : qualifiers.includes(".dark")
            ? true
            : null;
        const theme = qualifiers.match(/\[data-theme="([^"]+)"\]/)?.[1] ?? null;
        const weight =
          1 + (qualifiers.match(/\.dark|\[data-theme/g)?.length ?? 0);
        node.walkDecls((decl) => {
          if (!decl.prop.startsWith("--")) return;
          order += 1;
          found.push({
            name: decl.prop,
            value: decl.value,
            weight,
            order,
            dark,
            theme,
          });
        });
      }
    });
  }
  return found;
}

/**
 * The custom properties in force on the root for one preset: the sheets'
 * token blocks that match its mode and id, by specificity then source order,
 * and over them the inline values `applyTheme` itself writes — read back from
 * the element, so the mapping from preset fields to variables is the real one
 * and not a copy of it.
 */
function tokensFor(preset: ThemePreset): Map<string, string> {
  const dark = preset.mode === "dark";
  const applicable = tokenDeclarations()
    .filter(
      (decl) =>
        (decl.dark === null || decl.dark === dark) &&
        (decl.theme === null || decl.theme === preset.id),
    )
    .sort((a, b) => a.weight - b.weight || a.order - b.order);
  const tokens = new Map<string, string>();
  for (const decl of applicable) tokens.set(decl.name, decl.value);
  const root = document.documentElement;
  root.removeAttribute("style");
  applyTheme(preset);
  for (let index = 0; index < root.style.length; index += 1) {
    const name = root.style.item(index);
    if (name.startsWith("--")) {
      tokens.set(name, root.style.getPropertyValue(name).trim());
    }
  }
  root.removeAttribute("style");
  return tokens;
}

/** Split on commas that are not inside parentheses. */
function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (char === "(") depth += 1;
    else if (char === ")") depth -= 1;
    else if (char === "," && depth === 0) {
      parts.push(text.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(text.slice(start).trim());
  return parts;
}

/** The text between a function's opening parenthesis and its matching close. */
function inside(text: string, name: string): string {
  if (!text.startsWith(`${name}(`) || !text.endsWith(")")) {
    throw new Error(`expected ${name}(…), got "${text}"`);
  }
  return text.slice(name.length + 1, -1);
}

/**
 * A colour value as the stylesheets write them: a literal, a `var()`, or a
 * `color-mix(in srgb, …)` of either. Anything else throws, so a sheet that
 * starts spelling a token some new way fails this test instead of being
 * measured as something it is not.
 */
function evaluate(
  value: string,
  tokens: Map<string, string>,
  seen = new Set<string>(),
): Rgba {
  const text = value.replace(/\s+/g, " ").trim();
  if (text.startsWith("var(")) {
    const [name, fallback] = splitTopLevel(inside(text, "var"));
    if (!name) throw new Error(`empty var() in "${value}"`);
    if (seen.has(name)) throw new Error(`cycle through ${name}`);
    const next = tokens.get(name) ?? fallback;
    if (next === undefined) throw new Error(`undefined token ${name}`);
    return evaluate(next, tokens, new Set([...seen, name]));
  }
  if (text.startsWith("color-mix(")) {
    const [space, first, second] = splitTopLevel(inside(text, "color-mix"));
    if (space !== "in srgb" || !first || !second) {
      throw new Error(`unsupported mix "${value}"`);
    }
    const operand = (part: string): [Rgba, number | null] => {
      const percent = part.match(/^(.*\S)\s+([\d.]+)%$/);
      return percent?.[1]
        ? [evaluate(percent[1], tokens, seen), Number(percent[2])]
        : [evaluate(part, tokens, seen), null];
    };
    const [a, pa] = operand(first);
    const [b, pb] = operand(second);
    return mixSrgb(a, pa, b, pb);
  }
  return parseColor(text);
}

/**
 * The strongest share of `--fe-glow` and of `--fe-glow-deep` any background
 * in `fluid.css` spends, as `color-mix(in srgb, var(--fe-glow) N%,
 * transparent)`: the pools behind the page, the heads' light, the login
 * story's, the player's top edge. Read from the sheet, so a background that
 * spends the light harder is measured at once. Shadows are left out: a glow
 * cast under a cover is not a ground any text sits on.
 */
function strongestSpend(name: "--fe-glow" | "--fe-glow-deep"): number {
  const pattern = new RegExp(
    `color-mix\\( ?in srgb, var\\(${name}\\) ([\\d.]+)%, transparent ?\\)`,
    "g",
  );
  const spends: number[] = [];
  postcss.parse(fluid).walkDecls(/^background/, (decl) => {
    const value = decl.value.replace(/\s+/g, " ");
    for (const match of value.matchAll(pattern)) spends.push(Number(match[1]));
  });
  if (spends.length === 0) throw new Error(`fluid.css never spends ${name}`);
  return Math.max(...spends);
}

/**
 * The page under the cover's light at its strongest: each pool's colour at
 * the largest share any rule spends it, painted over the page. Text sits
 * there — a head's kicker and count are drawn over its own pool of light.
 *
 * Which light: the theme's own stand-in, in both materials, and in the light
 * material also the darkest light a cover can bring, taken as black. A
 * record's light replaces the stand-in as soon as one plays, and a dark cover
 * is what pulls a pale page towards the dark ink. (The dark material's worst
 * cover is a bright one; that bound is not measured here.)
 */
function glowGrounds(
  preset: ThemePreset,
  tokens: Map<string, string>,
  page: Rgba,
): Record<string, Rgba> {
  const lights: Array<[string, Map<string, string>]> = [
    ["the theme's light", tokens],
  ];
  if (preset.mode === "light") {
    const darkest = new Map(tokens);
    darkest.set("--glow", "#000");
    darkest.set("--glow-deep", "#000");
    lights.push(["the darkest cover's light", darkest]);
  }
  const out: Record<string, Rgba> = {};
  for (const [light, values] of lights) {
    for (const name of ["--fe-glow", "--fe-glow-deep"] as const) {
      const share = strongestSpend(name);
      out[`${light} (${name} at ${share}%)`] = composite(
        evaluate(
          `color-mix(in srgb, var(${name}) ${share}%, transparent)`,
          values,
        ),
        page,
      );
    }
  }
  return out;
}

/** WCAG 1.4.3: text at the sizes the interface sets it. */
const TEXT = 4.5;
/** WCAG 1.4.11: a mark that carries state — a focus ring, an edge, a border. */
const INDICATOR = 3;

type Measure = { pair: string; ratio: number; minimum: number };

/**
 * Every pairing the material makes, measured for one preset.
 *
 * The grounds are the ones text actually sits on in the Fluid Editorial
 * material (`fluid.css`, the last sheet, which redefines every colour token
 * the earlier ones set): the page, a panel, a raised panel, the deepest of
 * the translucent tones a hovered or open item lays over the page — and the
 * page under the cover's light at its strongest (see `glowGrounds`).
 *
 * Then the pairs. The three inks hold AA on every ground — `fluid.css`
 * promises it for the deepest tone in so many words. The accent is spent as
 * text (links, kickers, a hovered title), so it is held to the text minimum,
 * stricter than the 3:1 a mark alone would need; it is also the edge of the
 * playing row and the focus ring, which sit beside a tone, held to 3:1. And
 * the ink set on a filled accent — the primary button — holds AA on it.
 */
function measure(preset: ThemePreset): Measure[] {
  const tokens = tokensFor(preset);
  const read = (name: string) => evaluate(`var(${name})`, tokens);
  const page = read("--fe-bg");
  if (page.alpha < 1) throw new Error(`${preset.id}: the page is not opaque`);
  const surfaces: Record<string, Rgba> = {
    page,
    panel: composite(read("--fe-surface"), page),
    raised: composite(read("--fe-surface-2"), page),
    "deepest tone": composite(read("--fe-tone-3"), page),
    ...glowGrounds(preset, tokens, page),
  };
  const out: Measure[] = [];
  const add = (pair: string, fg: Rgba, bg: Rgba, minimum: number) =>
    out.push({
      pair: `${pair}: ${toHex(composite(fg, bg))} on ${toHex(bg)}`,
      ratio: contrastRatio(fg, bg),
      minimum,
    });
  for (const [ground, colour] of Object.entries(surfaces)) {
    for (const ink of ["--fe-ink", "--fe-ink-2", "--fe-ink-3"]) {
      add(`${ink} on ${ground}`, read(ink), colour, TEXT);
    }
  }
  // The accent is not set over the glow: the heads' kickers and counts that
  // sit there are `--fe-ink-3`, measured above.
  for (const ground of ["page", "panel", "raised"]) {
    add(
      `--accent as text on ${ground}`,
      read("--accent"),
      surfaces[ground] as Rgba,
      TEXT,
    );
  }
  add(
    "--accent as an edge beside the playing row's tone",
    read("--accent"),
    composite(read("--fe-tone-2"), page),
    INDICATOR,
  );
  add(
    "--fe-accent-ink on a filled --accent",
    read("--fe-accent-ink"),
    composite(read("--accent"), page),
    TEXT,
  );
  return out;
}

describe("contrast across every preset", () => {
  it.each(THEME_PRESETS.map((preset) => [preset.id, preset] as const))(
    "%s holds WCAG AA on every pairing its material makes",
    (_id, preset) => {
      const failing = measure(preset)
        .filter(({ ratio, minimum }) => ratio < minimum)
        .map(
          ({ pair, ratio, minimum }) =>
            `${pair} is ${ratio.toFixed(2)}:1, below ${minimum}:1`,
        );
      expect(failing).toEqual([]);
    },
  );

  it("reads the theme's own tokens, not the sheet's defaults", () => {
    // A resolver that ignored `applyTheme` would measure Emerald fourteen
    // times and pass the other thirteen for nothing.
    const accents = new Set(
      THEME_PRESETS.map((preset) =>
        toHex(evaluate("var(--accent)", tokensFor(preset))),
      ),
    );
    expect(accents.size).toBeGreaterThanOrEqual(7);
    const grounds = new Set(
      THEME_PRESETS.filter((preset) => preset.mode === "light").map((preset) =>
        toHex(evaluate("var(--fe-bg)", tokensFor(preset))),
      ),
    );
    expect(grounds.size).toBe(6);
    // And each dark preset its own ground, panel and raised surface.
    for (const token of ["--fe-bg", "--fe-surface", "--fe-surface-2"]) {
      const dark = new Set(
        THEME_PRESETS.filter((preset) => preset.mode === "dark").map((preset) =>
          toHex(evaluate(`var(${token})`, tokensFor(preset))),
        ),
      );
      expect(dark.size, token).toBe(8);
    }
  });
});

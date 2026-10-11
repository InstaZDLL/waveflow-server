import { describe, expect, it } from "vitest";

// `?raw`, not `node:fs`: there is no `@types/node` in this client and adding
// one so a test can read a file beside it would be a dependency for the sake
// of a dependency. Vite hands the stylesheet over as a string, and
// `vite-env.d.ts` already references `vite/client`, so this is typed.
import css from "./styles.css?raw";

/**
 * Inside the scale's range, the scale is what is used.
 *
 * This stylesheet carried 49 distinct spacing values and 37 distinct font
 * sizes, with no variable for either — seven of those sizes between 0.70 and
 * 0.86rem, which is the same size spelled seven ways. Nothing was wrong
 * anywhere; nothing agreed anywhere, and that is what read as cardboard.
 *
 * **A rule rather than a list of exceptions.** A hairline below the smallest
 * step is a line and not a space; a `rem` above the largest is a dimension —
 * a clearance for a fixed bar, a decorative glyph hanging off an edge — and
 * neither is a step in a rhythm. So the check bounds itself by the scale's own
 * range instead of carrying an allowlist that would need a reason written
 * beside every entry and would grow without anybody deciding to let it.
 */

const SMALLEST_STEP = 0.25;
const LARGEST_STEP = 5;

const SPACING_OR_TYPE =
  /^(padding|margin|gap|row-gap|column-gap)(-[a-z-]+)?$|^font-size$/;

const STEP_DECLARATION = /--((?:space|text)-[a-z0-9]+):\s*([0-9.]+)rem;/g;

/** Walks declarations outside the blocks that define the tokens. */
function eachDeclaration(
  css: string,
  visit: (property: string, value: string, line: number) => void,
): void {
  let depth = 0;
  let inTokens = false;
  css.split("\n").forEach((line, index) => {
    const opens = (line.match(/\{/g) ?? []).length;
    const closes = (line.match(/\}/g) ?? []).length;
    const wasTokens = inTokens;
    // A scale cannot be written in terms of itself, so `:root` and the theme
    // overrides beside it are skipped.
    const startsTokens =
      depth === 0 && opens > 0 && /(:root|\[data-theme)/.test(line);
    if (startsTokens) inTokens = true;
    depth += opens - closes;
    if (depth <= 0) {
      inTokens = false;
      depth = 0;
    }
    // `startsTokens` as well as the two flags: a `:root { --space-1: 0.25rem; }`
    // written on one line opens and closes in the same breath, so `inTokens`
    // is already back to false by the time this runs.
    if (wasTokens || inTokens || startsTokens) return;
    // **Every declaration on the line, not the line as one declaration.** The
    // rule is about the stylesheet and not about how it happens to be
    // formatted today: `.a { gap: 0.65rem; }` is the same defect written
    // flat, and an anchored pattern would walk straight past it the day a
    // formatter setting changes or somebody writes a rule by hand.
    //
    // The terminator is looked at rather than eaten: consuming the `;` leaves
    // the next declaration without the delimiter its own match needs, so a
    // line carrying two of them gave up one — found by asserting on two.
    const declarations =
      /(?:^|[;{])\s*([a-z-]+)\s*:\s*([^{};]*?)\s*(?=[;{}]|$)/g;
    for (const declaration of line.matchAll(declarations)) {
      visit(declaration[1], declaration[2], index + 1);
    }
  });
}

/** Every raw `rem` inside the scale's range, outside the token blocks. */
function offScale(css: string): string[] {
  const found: string[] = [];
  eachDeclaration(css, (property, value, line) => {
    if (!SPACING_OR_TYPE.test(property)) return;
    for (const raw of value.match(/(?<![\w.-])\d+(?:\.\d+)?rem\b/g) ?? []) {
      const asked = Number.parseFloat(raw);
      if (asked === 0) continue;
      if (asked < SMALLEST_STEP / 2 || asked > LARGEST_STEP) continue;
      found.push(`line ${line}: ${property}: ${raw}`);
    }
  });
  return found;
}

/** The steps the stylesheet declares, in rem. */
function stepsOf(css: string): Record<string, number> {
  const steps: Record<string, number> = {};
  for (const match of css.matchAll(STEP_DECLARATION)) {
    steps[match[1]] = Number(match[2]);
  }
  return steps;
}

/** Each rhythm `clamp()` whose floor ended up above its ceiling. */
function crossedRanges(css: string): { crossed: string[]; checked: number } {
  const steps = stepsOf(css);
  const resolve = (part: string): number | null => {
    const token = /var\(--((?:space|text)-[a-z0-9]+)\)/.exec(part);
    if (token) return steps[token[1]] ?? null;
    const rem = /([0-9.]+)rem/.exec(part);
    return rem ? Number(rem[1]) : null;
  };

  const crossed: string[] = [];
  let checked = 0;
  eachDeclaration(css, (property, value) => {
    if (!SPACING_OR_TYPE.test(property)) return;
    if (!value.startsWith("clamp(")) return;
    const inner = value.slice(value.indexOf("(") + 1, value.lastIndexOf(")"));
    const parts = inner.split(/,(?![^()]*\))/).map((part) => part.trim());
    if (parts.length !== 3) return;
    checked += 1;
    const floor = resolve(parts[0]);
    const ceiling = resolve(parts[2]);
    if (floor !== null && ceiling !== null && floor > ceiling) {
      crossed.push(`${property}: ${value}`);
    }
  });
  return { crossed, checked };
}

describe("the spacing and type scale", () => {
  it("is the only source of rhythm inside its own range", () => {
    expect(offScale(css)).toEqual([]);
  });

  it("declares every step it claims", () => {
    const steps = stepsOf(css);
    expect(Object.keys(steps).sort()).toEqual(
      [
        "space-1",
        "space-2",
        "space-3",
        "space-4",
        "space-6",
        "space-8",
        "space-12",
        "space-16",
        "space-20",
        "text-xs",
        "text-sm",
        "text-base",
        "text-lg",
        "text-xl",
        "text-2xl",
        "text-3xl",
        "text-4xl",
        "text-5xl",
        "text-6xl",
      ].sort(),
    );
    // The two ends the checks above bound themselves by, read off the sheet
    // rather than written down a second time.
    expect(Math.min(...Object.values(steps))).toBe(SMALLEST_STEP);
    expect(Math.max(...Object.values(steps))).toBe(LARGEST_STEP);
  });

  it("never lets a fluid range cross itself", () => {
    // **The one way snapping to a scale can break a rule it did not write.** A
    // `clamp()` whose floor was widened and whose ceiling tightened can end up
    // with the floor above the ceiling, and the browser then serves the floor
    // at every width — a size that silently stops being fluid.
    const { crossed, checked } = crossedRanges(css);
    expect(crossed).toEqual([]);
    // Pinned, so a walk that stops finding them cannot make the line above
    // vacuous.
    expect(checked).toBe(19);
  });

  it("finds the shapes it exists to refuse", () => {
    // The checks have to be able to fail for their reasons, and the stylesheet
    // is the wrong place to prove that from. `0.65rem` is what this pass
    // removes: a tenth away from a step, in a property that carries rhythm.
    expect(offScale(".a {\n  gap: 0.65rem;\n}")).toEqual([
      "line 2: gap: 0.65rem",
    ]);
    // The same defect written flat, which an anchored pattern missed.
    expect(offScale(".a { gap: 0.65rem; }")).toEqual(["line 1: gap: 0.65rem"]);
    // And several on one line, so the walk is not merely finding the first.
    expect(offScale(".a { gap: 0.65rem; padding: 0.3rem; }")).toEqual([
      "line 1: gap: 0.65rem",
      "line 1: padding: 0.3rem",
    ]);
    expect(
      crossedRanges(
        ":root {\n  --space-2: 0.5rem;\n  --space-8: 2rem;\n}\n" +
          ".a {\n  font-size: clamp(var(--space-8), 2vw, var(--space-2));\n}",
      ).crossed,
    ).toEqual(["font-size: clamp(var(--space-8), 2vw, var(--space-2))"]);

    // And the shapes neither of them may flag.
    expect(offScale(".a {\n  gap: 0.05rem;\n}")).toEqual([]);
    expect(offScale(".a {\n  padding-bottom: 6.4rem;\n}")).toEqual([]);
    // A dimension is not rhythm, whatever its size.
    expect(offScale(".a {\n  width: 0.65rem;\n}")).toEqual([]);
    // Nor the block that defines the steps, on two lines or on one.
    expect(offScale(":root {\n  --space-3: 0.75rem;\n}")).toEqual([]);
    expect(offScale(":root { --space-3: 0.75rem; }")).toEqual([]);
  });
});

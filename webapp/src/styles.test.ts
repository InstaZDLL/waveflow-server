import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

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

/** Every raw `rem` inside the scale's range, outside the token blocks. */
function offScale(css: string): string[] {
  const found: string[] = [];
  let depth = 0;
  let inTokens = false;
  css.split("\n").forEach((line, index) => {
    const opens = (line.match(/\{/g) ?? []).length;
    const closes = (line.match(/\}/g) ?? []).length;
    const wasTokens = inTokens;
    // The blocks that define the tokens, which cannot be written in terms of
    // themselves — `:root` and the theme overrides beside it.
    if (depth === 0 && opens > 0 && /(:root|\[data-theme)/.test(line)) {
      inTokens = true;
    }
    depth += opens - closes;
    if (depth <= 0) {
      inTokens = false;
      depth = 0;
    }
    if (wasTokens || inTokens) return;

    const declaration = /^\s*([a-z-]+)\s*:\s*(.+?);?\s*$/.exec(line);
    if (!declaration) return;
    const [, property, value] = declaration;
    if (!SPACING_OR_TYPE.test(property)) return;
    for (const raw of value.match(/(?<![\w.-])\d+(?:\.\d+)?rem\b/g) ?? []) {
      const asked = Number.parseFloat(raw);
      if (asked === 0) continue;
      if (asked < SMALLEST_STEP / 2 || asked > LARGEST_STEP) continue;
      found.push(`line ${index + 1}: ${property}: ${raw}`);
    }
  });
  return found;
}

describe("the spacing and type scale", () => {
  // A path from the project root, not from `import.meta.url`: vitest serves
  // this module over its own transform, so the URL is not a `file:` one and
  // `readFileSync` refuses it.
  const css = readFileSync("src/styles.css", "utf8");

  it("is the only source of rhythm inside its own range", () => {
    expect(offScale(css)).toEqual([]);
  });

  it("declares every step it claims", () => {
    for (const token of [
      "--space-1",
      "--space-2",
      "--space-3",
      "--space-4",
      "--space-6",
      "--space-8",
      "--space-12",
      "--space-16",
      "--space-20",
      "--text-xs",
      "--text-sm",
      "--text-base",
      "--text-lg",
      "--text-xl",
      "--text-2xl",
      "--text-3xl",
      "--text-4xl",
      "--text-5xl",
      "--text-6xl",
    ]) {
      expect(css).toContain(`${token}:`);
    }
  });

  it("finds a value that sits beside a step instead of on it", () => {
    // The check has to be able to fail for its reason, and the stylesheet is
    // the wrong place to prove that from. `0.65rem` is the shape the pass was
    // written to remove: a tenth away from a step, in a property that carries
    // rhythm.
    expect(offScale(".a {\n  gap: 0.65rem;\n}")).toEqual([
      "line 2: gap: 0.65rem",
    ]);
    // And the two kinds it must not flag.
    expect(offScale(".a {\n  gap: 0.05rem;\n}")).toEqual([]);
    expect(offScale(".a {\n  padding-bottom: 6.4rem;\n}")).toEqual([]);
    // Nor a dimension that is not rhythm, whatever its size.
    expect(offScale(".a {\n  width: 0.65rem;\n}")).toEqual([]);
    // Nor the block that defines the steps.
    expect(offScale(":root {\n  --space-3: 0.75rem;\n}")).toEqual([]);
  });
});

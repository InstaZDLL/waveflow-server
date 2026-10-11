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

const TOKEN_SELECTOR = /(:root|\[data-theme)/;

/**
 * The stylesheet with every token-defining block blanked out, line for line.
 *
 * **Blanked rather than skipped, and that distinction cost two rounds.** A
 * line is not a unit of CSS: a token block can open and close inside one, and
 * a rule can share the line with it either side. Skipping the line took the
 * rule with it; walking the line took the tokens as rules. Replacing the
 * block's own characters with spaces leaves the rest of the line to be read
 * normally, and every line number intact for the report.
 */
function withoutTokenBlocks(css: string): string {
  const out = css.split("");
  let at = 0;
  while (at < css.length) {
    const open = css.indexOf("{", at);
    if (open === -1) break;
    // What sits between the last rule's end and this brace — not everything
    // since the cursor, which for a nested rule would drag the enclosing
    // block's own declarations into the selector and match on them.
    const selector = css.slice(at, open).split(/[};]/).pop() ?? "";
    if (!TOKEN_SELECTOR.test(selector)) {
      // Descend instead of jumping past: an `@media` carries ordinary rules,
      // and nothing stops a token block from sitting inside one.
      at = open + 1;
      continue;
    }
    let depth = 0;
    let close = open;
    for (; close < css.length; close += 1) {
      if (css[close] === "{") depth += 1;
      else if (css[close] === "}") {
        depth -= 1;
        if (depth === 0) break;
      }
    }
    const to = Math.min(close, css.length - 1);
    for (let k = open - selector.length; k <= to; k += 1) {
      if (out[k] !== "\n") out[k] = " ";
    }
    at = to + 1;
  }
  return out.join("");
}

/** Walks declarations outside the blocks that define the tokens. */
function eachDeclaration(
  css: string,
  visit: (property: string, value: string, line: number) => void,
): void {
  const masked = withoutTokenBlocks(css);
  // **Over the whole sheet, not line by line.** A declaration is bounded by
  // `;` and braces, and nothing says it fits on one line — three in this sheet
  // already do not, `main`'s padding among them. Two earlier versions of this
  // walk split on newlines first and so read only the opening line of each,
  // which left the assertion below incomplete for the sheet as it stands and
  // not merely for some future edit.
  //
  // The line is then counted from the offset, so a value spanning lines is
  // still reported at the line its property sits on.
  const declarations = /(?:^|[;{}])\s*([a-z-]+)\s*:\s*([^;{}]*)/g;
  for (const declaration of masked.matchAll(declarations)) {
    const at =
      (declaration.index ?? 0) + declaration[0].indexOf(declaration[1]);
    const line = masked.slice(0, at).split("\n").length;
    visit(declaration[1], declaration[2], line);
  }
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

/**
 * The contents of every `clamp()` in a value, by balanced parentheses.
 *
 * **Not only one that is the whole value.** `padding: var(--space-12)
 * clamp(…) var(--space-20)` carries one in the middle, and a `calc()` wraps
 * another to negate it — three in this sheet, which a `startsWith` walked past
 * while the count still read nineteen and looked thorough.
 */
function clampsIn(value: string): string[] {
  const found: string[] = [];
  let at = 0;
  for (;;) {
    const start = value.indexOf("clamp(", at);
    if (start === -1) return found;
    let depth = 0;
    let end = start + "clamp".length;
    for (; end < value.length; end += 1) {
      if (value[end] === "(") depth += 1;
      else if (value[end] === ")") {
        depth -= 1;
        if (depth === 0) break;
      }
    }
    found.push(value.slice(start + "clamp(".length, end));
    at = end + 1;
  }
}

/** A comma-separated list, split only where the commas are not nested. */
function topLevelParts(inner: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let from = 0;
  for (let at = 0; at < inner.length; at += 1) {
    if (inner[at] === "(") depth += 1;
    else if (inner[at] === ")") depth -= 1;
    else if (inner[at] === "," && depth === 0) {
      parts.push(inner.slice(from, at).trim());
      from = at + 1;
    }
  }
  parts.push(inner.slice(from).trim());
  return parts;
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
    for (const inner of clampsIn(value)) {
      const parts = topLevelParts(inner);
      if (parts.length !== 3) continue;
      checked += 1;
      const floor = resolve(parts[0]);
      const ceiling = resolve(parts[2]);
      if (floor !== null && ceiling !== null && floor > ceiling) {
        crossed.push(`${property}: clamp(${inner})`);
      }
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
    // vacuous. Twenty-two, not the nineteen this said at first: three sit
    // inside a longer value — one of them wrapped in a `calc()` to negate it —
    // and a check that only read a `clamp()` occupying the whole value walked
    // past all three while its count still looked thorough.
    expect(checked).toBe(22);
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
    const twoSteps = ":root {\n  --space-2: 0.5rem;\n  --space-8: 2rem;\n}\n";
    expect(
      crossedRanges(
        twoSteps +
          ".a {\n  font-size: clamp(var(--space-8), 2vw, var(--space-2));\n}",
      ).crossed,
    ).toEqual(["font-size: clamp(var(--space-8), 2vw, var(--space-2))"]);
    // **One inside a longer value, and one wrapped to be negated.** Both
    // shapes are in this sheet, and a check reading only a whole-value
    // `clamp()` saw neither.
    expect(
      crossedRanges(
        twoSteps +
          ".a {\n  padding: var(--space-2) clamp(var(--space-8), 2vw, var(--space-2)) 0;\n}",
      ).crossed,
    ).toEqual(["padding: clamp(var(--space-8), 2vw, var(--space-2))"]);
    expect(
      crossedRanges(
        twoSteps +
          ".a {\n  margin: calc(clamp(var(--space-8), 2vw, var(--space-2)) * -1);\n}",
      ).crossed,
    ).toEqual(["margin: clamp(var(--space-8), 2vw, var(--space-2))"]);
    // Two in one value are both read, so the walk is not stopping at the first.
    expect(
      crossedRanges(
        twoSteps +
          ".a {\n  padding: clamp(var(--space-8), 2vw, var(--space-2))\n    clamp(var(--space-8), 1vw, var(--space-2));\n}",
      ).crossed,
    ).toHaveLength(2);

    // And the shapes neither of them may flag.
    expect(offScale(".a {\n  gap: 0.05rem;\n}")).toEqual([]);
    expect(offScale(".a {\n  padding-bottom: 6.4rem;\n}")).toEqual([]);
    // A dimension is not rhythm, whatever its size.
    expect(offScale(".a {\n  width: 0.65rem;\n}")).toEqual([]);
    // Nor the block that defines the steps, on two lines or on one.
    expect(offScale(":root {\n  --space-3: 0.75rem;\n}")).toEqual([]);
    expect(offScale(":root { --space-3: 0.75rem; }")).toEqual([]);

    // **And a rule sharing the line with that block is still read.** Skipping
    // the whole line hid it, which is the blind spot the first remedy traded
    // for the one it closed — either side of the block, since a blank at one
    // end proves nothing about the other.
    expect(
      offScale(":root { --space-3: 0.75rem; } .a { gap: 0.65rem; }"),
    ).toEqual(["line 1: gap: 0.65rem"]);
    expect(
      offScale(".a { gap: 0.65rem; } :root { --space-3: 0.75rem; }"),
    ).toEqual(["line 1: gap: 0.65rem"]);
    // A token block inside an `@media`, which is why the walk descends rather
    // than jumping past a block whose selector does not match.
    expect(
      offScale(
        "@media (hover: none) {\n  :root { --space-3: 0.75rem; }\n  .a { gap: 0.65rem; }\n}",
      ),
    ).toEqual(["line 3: gap: 0.65rem"]);

    // **A declaration that does not fit on one line.** Three in this sheet
    // already do not, and a walk that split on newlines first read only the
    // opening line of each. Reported at the line the property sits on, which
    // is where a reader would look for it.
    expect(
      offScale(".a {\n  padding: var(--space-4)\n    0.65rem;\n}"),
    ).toEqual(["line 2: padding: 0.65rem"]);
    expect(
      offScale(".a {\n  margin: 0 auto\n    0.3rem\n    var(--space-4);\n}"),
    ).toEqual(["line 2: margin: 0.3rem"]);

    // **What it deliberately does not read: a negative.** `-3rem` is a step
    // pulled backwards rather than a value beside one, and the sheet uses two
    // of them to undo a container's own padding. Flagging them would be noise
    // about a number that is already on the scale.
    expect(offScale(".a {\n  margin: -0.65rem;\n}")).toEqual([]);
  });
});

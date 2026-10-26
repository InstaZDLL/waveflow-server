import postcss from "postcss";
import { describe, expect, it } from "vitest";

import { styles as css } from "./sheets";

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
 *
 * **And it reads the sheet with a parser.** Five findings in four review
 * rounds went to a version that walked the text by hand, and every one of them
 * was the same mistake wearing a different hat: a line is not a unit of CSS.
 * A declaration can span lines, two can share one, a token block can open and
 * close inside one with a rule either side, and a comment is text that only
 * looks like CSS. `postcss` — which Vite already uses for this very file —
 * answers all of that by construction: declarations with their own line
 * numbers, comments that are not declarations, and a parent whose selector
 * says whether this is a block that defines the steps.
 *
 * The fixtures below predate it and are kept, because they are what each rule
 * means. That they now pass without being argued for is the point.
 */

const SMALLEST_STEP = 0.25;
const LARGEST_STEP = 5;

const SPACING_OR_TYPE =
  /^(padding|margin|gap|row-gap|column-gap)(-[a-z-]+)?$|^font-size$/;

/**
 * One selector that defines the scale rather than using it.
 *
 * **Anchored, and with no combinator allowed.** A substring test read
 * `:root .a { … }` as a token block and quietly excluded an ordinary rule —
 * `:root` is where the steps are declared, but `:root` *followed by anything
 * descendant* is a rule like any other. Pseudo-classes and attributes stay,
 * because the sheet's second token block is `:root:not(.dark)`.
 */
const TOKEN_SELECTOR = /^(:root|\[data-theme[^\]]*\])[^\s>+~,]*$/;

/** Whether every selector in a list defines tokens. */
function definesTokens(selector: string): boolean {
  const parts = selector
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
  return parts.length > 0 && parts.every((part) => TOKEN_SELECTOR.test(part));
}

type Declaration = {
  property: string;
  value: string;
  line: number;
  /** Inside a block that defines the tokens. */
  defining: boolean;
};

function declarations(source: string): Declaration[] {
  const found: Declaration[] = [];
  postcss.parse(source).walkDecls((declaration) => {
    const parent = declaration.parent;
    const selector =
      parent !== undefined && parent.type === "rule" ? parent.selector : "";
    found.push({
      property: declaration.prop,
      value: declaration.value,
      line: declaration.source?.start?.line ?? 0,
      defining: definesTokens(selector),
    });
  });
  return found;
}

/** Every raw `rem` inside the scale's range, outside the token blocks. */
function offScale(source: string): string[] {
  const found: string[] = [];
  for (const { property, value, line, defining } of declarations(source)) {
    if (defining || !SPACING_OR_TYPE.test(property)) continue;
    // **The sign is read, and the magnitude is what is compared.** A `-3rem`
    // is a step used without being named, not a different kind of value: all
    // four negatives in this sheet landed exactly on a step, which is what
    // showed that leaving them out was a gap rather than a decision. A pull
    // backwards is written `calc(var(--space-12) * -1)`.
    for (const raw of value.match(/(?<![\w.])-?\d+(?:\.\d+)?rem\b/g) ?? []) {
      const asked = Math.abs(Number.parseFloat(raw));
      if (asked === 0) continue;
      if (asked < SMALLEST_STEP / 2 || asked > LARGEST_STEP) continue;
      found.push(`line ${line}: ${property}: ${raw}`);
    }
  }
  return found;
}

/** The steps the stylesheet declares, in rem. */
function stepsOf(source: string): Record<string, number> {
  const steps: Record<string, number> = {};
  for (const { property, value } of declarations(source)) {
    const name = /^--((?:space|text)-[a-z0-9]+)$/.exec(property);
    const rem = /^([0-9.]+)rem$/.exec(value.trim());
    if (name && rem) steps[name[1]] = Number(rem[1]);
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
function crossedRanges(source: string): {
  crossed: string[];
  checked: number;
} {
  const steps = stepsOf(source);
  const resolve = (part: string): number | null => {
    const token = /var\(--((?:space|text)-[a-z0-9]+)\)/.exec(part);
    if (token) return steps[token[1]] ?? null;
    const rem = /([0-9.]+)rem/.exec(part);
    return rem ? Number(rem[1]) : null;
  };

  const crossed: string[] = [];
  let checked = 0;
  for (const { property, value, defining } of declarations(source)) {
    if (defining || !SPACING_OR_TYPE.test(property)) continue;
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
  }
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
    // inside a longer value — one wrapped in a `calc()` to negate it — and a
    // check that only read a `clamp()` occupying the whole value walked past
    // all three while its count still looked thorough. Twenty-four since the
    // share page (RFC-020) added two, its padding and its title.
    expect(checked).toBe(24);
  });

  it("finds the shapes it exists to refuse", () => {
    // Every rule above has to be able to fail for its own reason, and the
    // stylesheet is the wrong place to prove that from. `0.65rem` is what this
    // pass removes: a tenth away from a step, in a property that carries
    // rhythm.
    expect(offScale(".a {\n  gap: 0.65rem;\n}")).toEqual([
      "line 2: gap: 0.65rem",
    ]);
    // Written flat, which an anchored line pattern missed.
    expect(offScale(".a { gap: 0.65rem; }")).toEqual(["line 1: gap: 0.65rem"]);
    // Two on one line, which a pattern eating its own delimiter gave up on.
    expect(offScale(".a { gap: 0.65rem; padding: 0.3rem; }")).toEqual([
      "line 1: gap: 0.65rem",
      "line 1: padding: 0.3rem",
    ]);
    // A declaration that does not fit on one line — three in this sheet do
    // not. Reported at the line its property sits on, where a reader looks.
    expect(
      offScale(".a {\n  padding: var(--space-4)\n    0.65rem;\n}"),
    ).toEqual(["line 2: padding: 0.65rem"]);
    // A negative is a step used without being named, not another kind of
    // value: all four in this sheet landed exactly on a step.
    expect(offScale(".a {\n  margin: -0.65rem;\n}")).toEqual([
      "line 2: margin: -0.65rem",
    ]);
    // A comment is text that only looks like CSS: one mentioning `:root`
    // masked the rule after it, and one between a brace and a property
    // swallowed the declaration behind it.
    expect(
      offScale("/* :root is the token block */ .a { gap: 0.65rem; }"),
    ).toEqual(["line 1: gap: 0.65rem"]);
    expect(offScale(".a { /* a note */ gap: 0.65rem; }")).toEqual([
      "line 1: gap: 0.65rem",
    ]);
    expect(
      offScale(".a {\n  /* two\n     lines */\n  gap: 0.65rem;\n}"),
    ).toEqual(["line 4: gap: 0.65rem"]);

    // And the shapes it must not flag.
    expect(offScale(".a {\n  gap: 0.05rem;\n}")).toEqual([]);
    expect(offScale(".a {\n  padding-bottom: 6.4rem;\n}")).toEqual([]);
    // A dimension is not rhythm, whatever its size.
    expect(offScale(".a {\n  width: 0.65rem;\n}")).toEqual([]);
    // A pull backwards written in terms of the scale.
    expect(offScale(".a {\n  margin: calc(var(--space-12) * -1);\n}")).toEqual(
      [],
    );
    // Nor the block that defines the steps, on two lines or on one — nor a
    // rule sharing the line with it, either side.
    expect(offScale(":root {\n  --space-3: 0.75rem;\n}")).toEqual([]);
    expect(offScale(":root { --space-3: 0.75rem; }")).toEqual([]);
    expect(
      offScale(":root { --space-3: 0.75rem; } .a { gap: 0.65rem; }"),
    ).toEqual(["line 1: gap: 0.65rem"]);
    expect(
      offScale(".a { gap: 0.65rem; } :root { --space-3: 0.75rem; }"),
    ).toEqual(["line 1: gap: 0.65rem"]);
    // A token block inside an `@media`, which a walk that jumped past a
    // non-matching block would never have reached.
    expect(
      offScale(
        "@media (hover: none) {\n  :root { --space-3: 0.75rem; }\n  .a { gap: 0.65rem; }\n}",
      ),
    ).toEqual(["line 3: gap: 0.65rem"]);

    // **`:root` followed by a descendant is a rule like any other.** A
    // substring test read it as a token block and excluded it in silence.
    expect(offScale(":root .a { gap: 0.65rem; }")).toEqual([
      "line 1: gap: 0.65rem",
    ]);
    expect(offScale(":root > .a { gap: 0.65rem; }")).toEqual([
      "line 1: gap: 0.65rem",
    ]);
    // A list where one part is a rule and the other a token block is a rule.
    expect(offScale(":root, .a { gap: 0.65rem; }")).toEqual([
      "line 1: gap: 0.65rem",
    ]);
    // While the two the sheet actually declares its steps in stay excluded,
    // pseudo-class and all.
    expect(offScale(":root:not(.dark) { --space-3: 0.75rem; }")).toEqual([]);
    expect(
      offScale(':root,\n[data-theme="dark"] {\n  --space-3: 0.75rem;\n}'),
    ).toEqual([]);
  });

  it("finds a fluid range that crosses, wherever it sits", () => {
    const twoSteps = ":root {\n  --space-2: 0.5rem;\n  --space-8: 2rem;\n}\n";
    expect(
      crossedRanges(
        `${twoSteps}.a {\n  font-size: clamp(var(--space-8), 2vw, var(--space-2));\n}`,
      ).crossed,
    ).toEqual(["font-size: clamp(var(--space-8), 2vw, var(--space-2))"]);
    // One inside a longer value, and one wrapped to be negated. Both shapes
    // are in this sheet, and a check reading only a whole-value `clamp()` saw
    // neither.
    expect(
      crossedRanges(
        `${twoSteps}.a {\n  padding: var(--space-2) clamp(var(--space-8), 2vw, var(--space-2)) 0;\n}`,
      ).crossed,
    ).toEqual(["padding: clamp(var(--space-8), 2vw, var(--space-2))"]);
    expect(
      crossedRanges(
        `${twoSteps}.a {\n  margin: calc(clamp(var(--space-8), 2vw, var(--space-2)) * -1);\n}`,
      ).crossed,
    ).toEqual(["margin: clamp(var(--space-8), 2vw, var(--space-2))"]);
    // Two in one value are both read, so the walk is not stopping at the
    // first.
    expect(
      crossedRanges(
        `${twoSteps}.a {\n  padding: clamp(var(--space-8), 2vw, var(--space-2))\n    clamp(var(--space-8), 1vw, var(--space-2));\n}`,
      ).crossed,
    ).toHaveLength(2);
  });
});

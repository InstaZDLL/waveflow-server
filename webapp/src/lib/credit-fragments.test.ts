import { describe, expect, it } from "vitest";

import { creditFragments, isSplitCandidate } from "./credit-fragments";

describe("credit fragments", () => {
  it("cuts on the desktop's four commas", () => {
    expect(creditFragments("Ashreveal, Ashish Swargiary")).toEqual([
      "Ashreveal",
      "Ashish Swargiary",
    ]);
    expect(creditFragments("蔡依林，周杰伦")).toEqual(["蔡依林", "周杰伦"]);
    expect(creditFragments("庄心妍、祁隆")).toEqual(["庄心妍", "祁隆"]);
    expect(creditFragments("A､B, C")).toEqual(["A", "B", "C"]);
  });

  it("never cuts inside parentheses", () => {
    expect(creditFragments("Ian Asher, Olly Alexander (Years, Years)")).toEqual(
      ["Ian Asher", "Olly Alexander (Years, Years)"],
    );
  });

  it("keeps the first spelling of a repeated name", () => {
    expect(creditFragments("A, B, a")).toEqual(["A", "B"]);
  });

  it("offers a split only where the server lists one", () => {
    expect(isSplitCandidate("Zedd, Alessia Cara")).toBe(true);
    expect(isSplitCandidate("Tyler, The Creator")).toBe(true);
    expect(isSplitCandidate("Solo,")).toBe(false);
    expect(isSplitCandidate("Simon & Garfunkel")).toBe(false);
    expect(isSplitCandidate("A, a")).toBe(false);
  });
});

import { describe, expect, it } from "vitest";

import {
  type FiledFolder,
  folderOutline,
  folderTrail,
  isWithin,
} from "./folders";

const folder = (id: string, parent: string | null, name = id): FiledFolder => ({
  id,
  parent_id: parent,
  name,
  track_count: 0,
});

describe("the filed tree, read on the client", () => {
  const tree = [
    folder("rock", null, "Rock"),
    folder("indie", "rock", "Indie"),
    folder("lofi", "indie", "lo-fi"),
    folder("ambient", null, "ambient"),
  ];

  it("walks from the root down to a folder", () => {
    expect(folderTrail(tree, "lofi").map((entry) => entry.id)).toEqual([
      "rock",
      "indie",
      "lofi",
    ]);
    expect(folderTrail(tree, "gone")).toEqual([]);
  });

  it("stops on a cycle instead of looping", () => {
    const looped = [folder("a", "b"), folder("b", "a")];
    expect(folderTrail(looped, "a").length).toBeLessThanOrEqual(3);
    expect(folderOutline(looped)).toEqual([]);
  });

  it("outlines siblings by name, case aside, each before its children", () => {
    expect(
      folderOutline(tree).map(({ folder: entry, depth }) => [
        entry.name,
        depth,
      ]),
    ).toEqual([
      ["ambient", 0],
      ["Rock", 0],
      ["Indie", 1],
      ["lo-fi", 2],
    ]);
  });

  it("knows what sits beneath a folder", () => {
    expect(isWithin(tree, "lofi", "rock")).toBe(true);
    expect(isWithin(tree, "rock", "rock")).toBe(true);
    expect(isWithin(tree, "ambient", "rock")).toBe(false);
  });
});

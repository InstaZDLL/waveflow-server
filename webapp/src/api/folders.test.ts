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
  // In the order the server answers: every folder by name, natural order.
  const tree = [
    folder("ambient", null, "ambient"),
    folder("indie", "rock", "Indie"),
    folder("lofi", "indie", "lo-fi"),
    folder("rock", null, "Rock"),
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

  it("outlines siblings in the server's order, each before its children", () => {
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

  it("keeps the server's sibling order rather than sorting again", () => {
    // `natural_order` puts these as listed; a locale sort would move "Ä 2"
    // to the front, and the picker would disagree with the cards.
    const answered = [
      folder("a2", null, "a 2"),
      folder("a10", null, "A 10"),
      folder("umlaut", null, "Ä 2"),
      folder("v2", null, "Vol 2"),
      folder("v10", null, "Vol 10"),
    ];
    expect(
      folderOutline(answered).map(({ folder: entry }) => entry.name),
    ).toEqual(["a 2", "A 10", "Ä 2", "Vol 2", "Vol 10"]);
  });

  it("knows what sits beneath a folder", () => {
    expect(isWithin(tree, "lofi", "rock")).toBe(true);
    expect(isWithin(tree, "rock", "rock")).toBe(true);
    expect(isWithin(tree, "ambient", "rock")).toBe(false);
  });
});

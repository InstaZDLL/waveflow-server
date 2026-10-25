import { describe, expect, it } from "vitest";

import { discographySections, type ReleaseGroup } from "./discography";

const album = (title: string, release_group?: ReleaseGroup | null) => ({
  title,
  release_group,
});

const titles = (sections: ReturnType<typeof discographySections>) =>
  sections.map(({ section, albums }) => [
    section,
    albums.map((entry) => (entry as { title: string }).title),
  ]);

describe("an artist's discography", () => {
  it("lists its sections in page order, whatever order the albums came in", () => {
    const sections = discographySections(
      [
        album("Greatest", "compilation"),
        album("Live at Home", "live"),
        album("Lone", "single"),
        album("Short", "ep"),
        album("Long", "album"),
      ],
      [album("Guest Spot", "album")],
    );
    expect(titles(sections)).toEqual([
      ["album", ["Long"]],
      ["ep", ["Short"]],
      ["single", ["Lone"]],
      ["live", ["Live at Home"]],
      ["compilation", ["Greatest"]],
      ["appearsOn", ["Guest Spot"]],
    ]);
  });

  it("keeps the server's order inside a section", () => {
    const sections = discographySections([
      album("Second", "album"),
      album("First", "album"),
    ]);
    expect(titles(sections)).toEqual([["album", ["Second", "First"]]]);
  });

  it("leaves out the sections that hold nothing", () => {
    expect(titles(discographySections([album("Only", "ep")]))).toEqual([
      ["ep", ["Only"]],
    ]);
    expect(discographySections([], [])).toEqual([]);
  });

  it("files an album an older server sent without a group as an album", () => {
    const sections = discographySections([
      album("Old", undefined),
      album("Null", null),
      album("Odd", "bootleg" as ReleaseGroup),
      // A section name is not a group: only the server's appearances list
      // fills "Appears on".
      album("Sly", "appearsOn" as ReleaseGroup),
    ]);
    expect(titles(sections)).toEqual([
      ["album", ["Old", "Null", "Odd", "Sly"]],
    ]);
  });
});

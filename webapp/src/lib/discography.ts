/**
 * An artist's discography, in the sections the artist page shows
 * (RFC-022 decisions 2 and 3).
 *
 * The server decides which section an album belongs in — `release_group` on
 * every album — so this only orders the sections and drops the empty ones. It
 * never reads `release_types` itself: two clients interpreting MusicBrainz's
 * vocabulary separately is how they come to disagree about the same record.
 */

export type ReleaseGroup = "album" | "ep" | "single" | "live" | "compilation";

/** A section of the page: one release group, or the albums appeared on. */
export type DiscographySection = ReleaseGroup | "appearsOn";

/** The order the page lists its sections in. */
export const SECTION_ORDER: readonly DiscographySection[] = [
  "album",
  "ep",
  "single",
  "live",
  "compilation",
  "appearsOn",
];

const GROUPS = new Set<string>([
  "album",
  "ep",
  "single",
  "live",
  "compilation",
] satisfies ReleaseGroup[]);

/**
 * The albums of an artist page, sectioned, in page order, empty sections left
 * out. Each section keeps the order the server sent, which is year then title.
 *
 * An album with no `release_group` — sent by a server older than RFC-022 — is
 * an album, which is what the page showed for it before.
 */
export function discographySections<
  T extends { release_group?: ReleaseGroup | null },
>(
  albums: readonly T[],
  appearsOn: readonly T[] = [],
): Array<{ section: DiscographySection; albums: T[] }> {
  const sections = new Map<DiscographySection, T[]>();
  for (const album of albums) {
    const group =
      album.release_group && GROUPS.has(album.release_group)
        ? album.release_group
        : "album";
    const list = sections.get(group) ?? [];
    list.push(album);
    sections.set(group, list);
  }
  if (appearsOn.length > 0) sections.set("appearsOn", [...appearsOn]);
  return SECTION_ORDER.flatMap((section) => {
    const list = sections.get(section);
    return list && list.length > 0 ? [{ section, albums: list }] : [];
  });
}

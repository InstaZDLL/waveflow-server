/**
 * How the server cuts a joined artist credit (RFC-015 decision 2), for the
 * one place the client needs to know before asking: whether the artist page
 * offers "Split into N artists" at all. The server decides regardless — a
 * credit it does not take for a candidate answers `422`.
 *
 * The desktop's four characters, never inside parentheses, each fragment
 * trimmed, blanks dropped, duplicates dropped on a folded name with the first
 * spelling kept.
 */
const SEPARATORS = new Set([",", "，", "、", "､"]);

/** Close to the server's canonical name: case, accents and punctuation go. */
function canonical(value: string) {
  return value
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

export function creditFragments(name: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = "";
  for (const character of name) {
    if (character === "(") depth += 1;
    if (character === ")") depth = Math.max(0, depth - 1);
    if (depth === 0 && SEPARATORS.has(character)) {
      parts.push(current);
      current = "";
    } else {
      current += character;
    }
  }
  parts.push(current);
  const seen = new Set<string>();
  return parts
    .map((part) => part.trim())
    .filter((part) => {
      if (!part) return false;
      const key = canonical(part);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

/** Whether the server would list the credit: two fragments, none the whole. */
export function isSplitCandidate(name: string): boolean {
  const parts = creditFragments(name);
  const whole = canonical(name);
  return parts.length >= 2 && parts.every((part) => canonical(part) !== whole);
}

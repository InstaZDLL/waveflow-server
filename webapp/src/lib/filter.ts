/** Drops case and diacritics, so "bjork" reaches "Björk". */
function fold(value: string) {
  return value
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase();
}

/** The filter as the comparison sees it. Empty means no filter is applied. */
export function normalizeFilter(value: string) {
  return fold(value).trim();
}

export function matches(haystack: string | null, needle: string) {
  return haystack ? fold(haystack).includes(needle) : false;
}

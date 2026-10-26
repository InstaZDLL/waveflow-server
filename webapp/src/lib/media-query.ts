/**
 * Calls `listener` whenever `query` stops or starts matching, and answers the
 * function that stops it.
 *
 * `addEventListener` on a `MediaQueryList` arrived in Safari 14; before it,
 * only the deprecated `addListener` exists, and calling the missing method
 * throws inside an effect, which takes the whole page down with it. The old
 * spelling is used where the new one is absent.
 */
export function onMediaChange(
  query: MediaQueryList,
  listener: () => void,
): () => void {
  if (typeof query.addEventListener === "function") {
    query.addEventListener("change", listener);
    return () => query.removeEventListener("change", listener);
  }
  if (typeof query.addListener === "function") {
    query.addListener(listener);
    return () => query.removeListener(listener);
  }
  return () => {};
}

/**
 * Picking rows of a list to act on them together, without React in it.
 *
 * Rows are named by an id, and a row's position in the list is what a click
 * reports. The anchor is the last row picked by itself: Shift on another row
 * picks every row between the two, as file managers do, and leaves the anchor
 * where it was so a second Shift-click redraws the range from the same row.
 */
export type Picked = {
  selected: ReadonlySet<string>;
  /** The position of the last row picked without Shift, if any. */
  anchor: number | null;
};

export const NOTHING_PICKED: Picked = { selected: new Set(), anchor: null };

/**
 * The selection after a click on the row at `position` of `ids`.
 *
 * Without Shift, the row is toggled and becomes the anchor. With Shift and an
 * anchor, every row from the anchor to this one is picked, added to what was
 * already picked. With Shift and no anchor — nothing was clicked yet, or the
 * anchor's row is gone from the list — it is a plain toggle.
 */
export function pick(
  ids: readonly string[],
  current: Picked,
  position: number,
  extend: boolean,
): Picked {
  const id = ids[position];
  if (id === undefined) return current;
  const anchor =
    current.anchor !== null && current.anchor < ids.length
      ? current.anchor
      : null;
  if (extend && anchor !== null) {
    const selected = new Set(current.selected);
    const [from, to] =
      anchor <= position ? [anchor, position] : [position, anchor];
    for (let index = from; index <= to; index += 1) selected.add(ids[index]);
    return { selected, anchor };
  }
  const selected = new Set(current.selected);
  if (selected.has(id)) selected.delete(id);
  else selected.add(id);
  return { selected, anchor: position };
}

/** Every row of `ids` picked, or none when all of them already are. */
export function pickAll(ids: readonly string[], current: Picked): Picked {
  const all = ids.length > 0 && ids.every((id) => current.selected.has(id));
  return all ? NOTHING_PICKED : { selected: new Set(ids), anchor: null };
}

/**
 * What is picked and still in the list, in the list's order: a row that left
 * it — filed elsewhere, or no longer available — is not acted on.
 */
export function pickedIn(ids: readonly string[], current: Picked): string[] {
  return ids.filter((id) => current.selected.has(id));
}

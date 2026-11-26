/**
 * Picking rows of a list to act on them together, without React in it.
 *
 * Rows are named by an id, and a row's position in the list is what a click
 * reports. The anchor is the last row picked by itself: Shift on another row
 * picks every row between the two, as file managers do, and leaves the anchor
 * where it was so a second Shift-click redraws the range from the same row.
 * The anchor is held by its id, not its position: a list re-read with a row
 * gone would otherwise hand the anchor to the row that took its place.
 */
export type Picked = {
  selected: ReadonlySet<string>;
  /** The id of the last row picked without Shift, if any. */
  anchor: string | null;
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
  const anchor = current.anchor === null ? -1 : ids.indexOf(current.anchor);
  if (extend && anchor !== -1) {
    const selected = new Set(current.selected);
    const [from, to] =
      anchor <= position ? [anchor, position] : [position, anchor];
    for (let index = from; index <= to; index += 1) selected.add(ids[index]);
    return { selected, anchor: current.anchor };
  }
  const selected = new Set(current.selected);
  if (selected.has(id)) selected.delete(id);
  else selected.add(id);
  return { selected, anchor: id };
}

/** Every row of `ids` picked, or none when all of them already are. */
export function pickAll(ids: readonly string[], current: Picked): Picked {
  const all = ids.length > 0 && ids.every((id) => current.selected.has(id));
  return all ? NOTHING_PICKED : { selected: new Set(ids), anchor: null };
}

/**
 * The selection without `ids`: what an action took away, and only that, so
 * rows picked while it ran stay picked.
 */
export function unpick(current: Picked, ids: readonly string[]): Picked {
  const selected = new Set(current.selected);
  for (const id of ids) selected.delete(id);
  return { selected, anchor: current.anchor };
}

/**
 * What is picked and still in the list, in the list's order: a row that left
 * it — filed elsewhere, or no longer available — is not acted on.
 */
export function pickedIn(ids: readonly string[], current: Picked): string[] {
  return ids.filter((id) => current.selected.has(id));
}

/** How far a menu keeps from the window's edges. */
const MARGIN = 8;

/**
 * Where a menu of `width` × `height` goes in a window of `viewport`.
 *
 * At `x`, `y` — its left edge there, or its right edge with `align: "end"` —
 * then moved back inside the window when it would hang off an edge. Given
 * `above`, a menu with no room below `y` hangs upward from `above` instead: the
 * top of the button that opened it, so the menu does not cover the button.
 */
export function placeMenu({
  x,
  y,
  above,
  align,
  width,
  height,
  viewport,
}: {
  x: number;
  y: number;
  above?: number;
  align: "start" | "end";
  width: number;
  height: number;
  viewport: { width: number; height: number };
}): { left: number; top: number } {
  const left = align === "end" ? x - width : x;
  const top =
    above !== undefined && y + height + MARGIN > viewport.height
      ? above - height
      : y;
  return {
    left: Math.max(MARGIN, Math.min(left, viewport.width - width - MARGIN)),
    top: Math.max(MARGIN, Math.min(top, viewport.height - height - MARGIN)),
  };
}

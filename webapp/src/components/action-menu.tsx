import { useId, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { Icon } from "./icons";
import { PopupMenu, type PopupMenuItem } from "./popup-menu";

export type ActionMenuItem = Pick<
  PopupMenuItem,
  "key" | "label" | "run" | "danger"
>;

/**
 * A "⋯" button and the short menu it opens: the actions on one card that do
 * not deserve a button of their own.
 *
 * The menu is the track menu's own [`PopupMenu`], hung under the button by its
 * right edge, and above it when there is no room below. Portalled to the body
 * for the track menu's reason: a page block rising into place carries a
 * `translate`, and a fixed child of it would be placed inside the block
 * instead of beside the button.
 *
 * Closing by a key or an entry hands focus back to the button, before the
 * chosen entry runs: an entry that opens a dialog then finds it there to hand
 * back when the dialog closes.
 */
export function ActionMenu({
  label,
  items,
  disabled = false,
}: {
  /** The button's accessible name, which says whose actions these are. */
  label: string;
  items: ActionMenuItem[];
  disabled?: boolean;
}) {
  const [at, setAt] = useState<{ x: number; y: number; above: number } | null>(
    null,
  );
  const button = useRef<HTMLButtonElement>(null);
  const menuId = useId();

  function toggle() {
    if (at) {
      setAt(null);
      return;
    }
    const box = button.current?.getBoundingClientRect();
    if (!box) return;
    setAt({ x: box.right, y: box.bottom + 4, above: box.top - 4 });
  }

  function close(refocus: boolean) {
    setAt(null);
    if (refocus) button.current?.focus();
  }

  return (
    <>
      <button
        ref={button}
        type="button"
        className="action-menu-button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={at !== null}
        aria-controls={at ? menuId : undefined}
        disabled={disabled}
        onClick={toggle}
      >
        <Icon name="more" size={18} />
      </button>
      {at
        ? createPortal(
            <PopupMenu
              id={menuId}
              className="action-menu"
              label={label}
              x={at.x}
              y={at.y}
              above={at.above}
              align="end"
              // Each entry closes the menu itself, focus first, and only then
              // runs: `opensDialog` tells the menu not to close it again.
              items={items.map((item) => ({
                ...item,
                opensDialog: true,
                run: () => {
                  close(true);
                  item.run();
                },
              }))}
              onClose={close}
              anchor={button}
            />,
            document.body,
          )
        : null}
    </>
  );
}

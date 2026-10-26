import {
  type KeyboardEvent,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

import { Icon } from "./icons";

export type ActionMenuItem = {
  key: string;
  label: string;
  run: () => void;
  /** Drawn in the danger colour: the entry that destroys something. */
  danger?: boolean;
};

/**
 * A "⋯" button and the short menu it opens: the actions on one card that do
 * not deserve a button of their own.
 *
 * Drawn as the track menu is — the same panel and entries, the same keys of
 * the ARIA menu pattern — but anchored to its button rather than to a pointer.
 * Portalled to the body and fixed to the viewport for the track menu's reason:
 * a page block rising into place carries a `translate`, and a fixed child of
 * it would be placed inside the block instead of beside the button.
 *
 * Closing hands focus back to the button, before the chosen entry runs.
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
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const menuId = useId();

  function close(refocus: boolean) {
    setOpen(false);
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
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        disabled={disabled}
        onClick={() => setOpen((current) => !current)}
      >
        <Icon name="more" size={18} />
      </button>
      {open && button.current
        ? createPortal(
            <ActionMenuPanel
              id={menuId}
              label={label}
              anchor={button.current}
              items={items}
              onClose={close}
            />,
            document.body,
          )
        : null}
    </>
  );
}

function ActionMenuPanel({
  id,
  label,
  anchor,
  items,
  onClose,
}: {
  id: string;
  label: string;
  anchor: HTMLElement;
  items: ActionMenuItem[];
  onClose: (refocus: boolean) => void;
}) {
  const list = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top: number } | null>(
    null,
  );

  // Under the button, its right edge on the button's, and moved back inside
  // the window when that would hang off an edge — above the button when there
  // is no room below it.
  useLayoutEffect(() => {
    const node = list.current;
    if (!node) return;
    const from = anchor.getBoundingClientRect();
    const rect = node.getBoundingClientRect();
    const margin = 8;
    const below = from.bottom + 4;
    const top =
      below + rect.height + margin > window.innerHeight
        ? from.top - rect.height - 4
        : below;
    setPlace({
      left: Math.max(
        margin,
        Math.min(
          from.right - rect.width,
          window.innerWidth - rect.width - margin,
        ),
      ),
      top: Math.max(margin, top),
    });
  }, [anchor]);

  // Focus moves in once the menu is placed: hidden for its measurement, it
  // could not take focus before.
  const placed = place !== null;
  useEffect(() => {
    if (placed) {
      list.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    }
  }, [placed]);

  // Anything outside closes it, as it closes the track menu; a press on the
  // button is left to the button, whose click toggles it shut.
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    const away = (event: Event) => {
      const target = event.target as Node;
      if (list.current?.contains(target) || anchor.contains(target)) return;
      closeRef.current(false);
    };
    const leave = () => closeRef.current(false);
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("wheel", leave, { capture: true, passive: true });
    window.addEventListener("blur", leave);
    window.addEventListener("resize", leave);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      document.removeEventListener("wheel", leave, true);
      window.removeEventListener("blur", leave);
      window.removeEventListener("resize", leave);
    };
  }, [anchor]);

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const entries = Array.from(
      list.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [],
    );
    const at = entries.indexOf(document.activeElement as HTMLElement);
    const go = (index: number) => {
      event.preventDefault();
      entries[(index + entries.length) % entries.length]?.focus();
    };
    switch (event.key) {
      case "ArrowDown":
        go(at + 1);
        break;
      case "ArrowUp":
        go(at - 1);
        break;
      case "Home":
        go(0);
        break;
      case "End":
        go(entries.length - 1);
        break;
      case "Escape":
        event.preventDefault();
        onClose(true);
        break;
      // Focus goes back to the button first and the Tab is left to run from
      // there, so it lands on what follows the button (or, with Shift, what
      // precedes it). Left in the menu, which sits at the end of the body,
      // the Tab ran off the page and the menu's unmounting dropped focus on
      // the body.
      case "Tab":
        onClose(true);
        break;
    }
  }

  return (
    <div
      ref={list}
      id={id}
      className="track-menu action-menu"
      role="menu"
      aria-label={label}
      // Measured hidden on the first frame, then placed: shown at the window's
      // corner for a frame, it would flash there before moving.
      style={
        place
          ? { left: place.left, top: place.top }
          : { left: 0, top: 0, visibility: "hidden" }
      }
      onKeyDown={onKeyDown}
    >
      {items.map((item) => (
        <button
          key={item.key}
          type="button"
          className={`track-menu-item${item.danger ? " danger" : ""}`}
          role="menuitem"
          tabIndex={-1}
          onClick={() => {
            // Focus goes back to the button first, so an entry that opens a
            // dialog finds it there to hand back when the dialog closes.
            onClose(true);
            item.run();
          }}
        >
          <span>{item.label}</span>
        </button>
      ))}
    </div>
  );
}

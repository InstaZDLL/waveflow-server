import {
  type KeyboardEvent,
  type RefObject,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { Icon, type IconName } from "./icons";

/** One entry of a menu. */
export type PopupMenuItem = {
  key: string;
  icon: IconName;
  label: string;
  run: () => void;
  /**
   * The entry replaces the menu with a dialog of its own, which takes focus.
   * Such an entry closes the menu itself: closing it here would hand focus back
   * to what opened the menu, from under the dialog that has just taken it.
   */
  opensDialog?: boolean;
};

/**
 * A menu fixed to the viewport at a point, as the right click on a track
 * opens one (RFC-012 decision 4), and as the artist page opens its image
 * actions.
 *
 * It is placed at `x`, `y` — its left edge there, or its right edge with
 * `align: "end"`, for a menu hung under a button at the right of the screen —
 * and moved back inside the window when it would hang off an edge. It answers
 * the keys of the ARIA menu pattern, and anything outside it closes it.
 *
 * Fixed rather than absolute, so the caller portals it to the body: an
 * ancestor that clips its overflow, or that carries a transform while it
 * rises into place, would otherwise cut it off or place it inside itself.
 */
export function PopupMenu({
  label,
  x,
  y,
  align = "start",
  items,
  onClose,
  anchor,
}: {
  label: string;
  x: number;
  y: number;
  align?: "start" | "end";
  items: PopupMenuItem[];
  onClose: () => void;
  /**
   * The button that opened the menu, if one did. A press on it is not a press
   * "outside": it toggles the menu, and closing here first would have the
   * same press open it again.
   */
  anchor?: RefObject<HTMLElement | null>;
}) {
  const list = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState({ left: x, top: y });

  // Kept inside the window: a menu opened near the right or bottom edge
  // would otherwise hang off it, half unreadable.
  useLayoutEffect(() => {
    const node = list.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    const margin = 8;
    const left = align === "end" ? x - rect.width : x;
    setPlace({
      left: Math.max(
        margin,
        Math.min(left, window.innerWidth - rect.width - margin),
      ),
      top: Math.max(
        margin,
        Math.min(y, window.innerHeight - rect.height - margin),
      ),
    });
    node.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
  }, [align, x, y]);

  // Anything outside the menu closes it: a click, the wheel or a finger
  // moving the page, the window losing focus.
  //
  // **Not the `scroll` event.** Scrolls nobody asked for happen while a menu
  // is open — focusing the row that raised it scrolls it into view on the next
  // frame, and a page still rising into place moves the row under it — and
  // closing on those shut the menu the instant it appeared. What closes it is
  // the visitor setting out to scroll, outside the menu.
  //
  // Installed once per menu, with the latest `onClose` read through a ref: the
  // parent hands a new one on every render, and re-installing on each would
  // also reset `armed` below in the middle of a gesture. The refs are written
  // after the commit, not during the render: a render React throws away must
  // not leave its callback behind.
  const closeRef = useRef(onClose);
  const anchorRef = useRef(anchor);
  useLayoutEffect(() => {
    closeRef.current = onClose;
    anchorRef.current = anchor;
  });
  useEffect(() => {
    const away = (event: Event) => {
      const target = event.target as Node;
      if (list.current?.contains(target)) return;
      if (anchorRef.current?.current?.contains(target)) return;
      closeRef.current();
    };
    // The wheel closes it, over the menu too, unless the menu has rows to
    // scroll: a track credited to many artists makes it taller than a short
    // window, and it is then capped and scrolls on its own. A menu with nothing
    // to scroll would hand the wheel to the page beneath and leave the menu
    // where its row used to be, so that one still closes.
    const wheel = (event: Event) => {
      const menu = list.current;
      const inside = menu?.contains(event.target as Node) ?? false;
      if (inside && menu && menu.scrollHeight > menu.clientHeight) return;
      closeRef.current();
    };
    const leave = () => closeRef.current();
    // A long press opens the menu with the finger still down, and that finger
    // drifts as it lifts: its moves are the gesture that opened the menu, not
    // one leaving it. Only a touch that starts after the menu is open counts.
    let armed = false;
    const arm = () => {
      armed = true;
    };
    const dragged = (event: Event) => {
      if (armed) away(event);
    };
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("wheel", wheel, { capture: true, passive: true });
    document.addEventListener("touchstart", arm, {
      capture: true,
      passive: true,
    });
    document.addEventListener("touchmove", dragged, {
      capture: true,
      passive: true,
    });
    window.addEventListener("blur", leave);
    window.addEventListener("resize", leave);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      document.removeEventListener("wheel", wheel, true);
      document.removeEventListener("touchstart", arm, true);
      document.removeEventListener("touchmove", dragged, true);
      window.removeEventListener("blur", leave);
      window.removeEventListener("resize", leave);
    };
  }, []);

  // The keys a menu is expected to answer, per the ARIA menu pattern. Tab
  // leaves, and leaving closes: a menu is not a place focus can stay.
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
        onClose();
        break;
      case "Tab":
        onClose();
        break;
    }
  }

  return (
    <div
      ref={list}
      className="track-menu"
      role="menu"
      aria-label={label}
      style={{ left: place.left, top: place.top }}
      onKeyDown={onKeyDown}
    >
      {items.map((item) => (
        <button
          key={item.key}
          type="button"
          className="track-menu-item"
          role="menuitem"
          tabIndex={-1}
          onClick={() => {
            item.run();
            if (!item.opensDialog) onClose();
          }}
        >
          <Icon name={item.icon} size={16} />
          <span>{item.label}</span>
        </button>
      ))}
    </div>
  );
}

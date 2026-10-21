import { useNavigate } from "@tanstack/react-router";
import {
  type KeyboardEvent,
  type MouseEvent,
  type TouchEvent,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import type { Song } from "../api";
import { usePlayer } from "../player/player";
import { useI18n } from "../state/i18n";
import { mayCorrectTracks, useLibraryScope } from "../state/library-scope";
import { Icon } from "./icons";
import { TrackProperties } from "./track-properties";

/** Where a menu was asked for, and for which row. */
type Opened = {
  songs: Song[];
  position: number;
  x: number;
  y: number;
  /** What had focus, so closing gives it back (RFC-012 decision 4). */
  returnTo: HTMLElement | null;
};

/** How long a finger rests on a row before it means "menu". */
const LONG_PRESS_MS = 500;
/** How far it may drift and still be a press rather than a scroll. */
const LONG_PRESS_SLOP_PX = 10;

/**
 * The right click on a track, and what it opens.
 *
 * `rowProps` goes on each row. A menu reachable only by a right click would be
 * one the accessibility gate is right to fail, so the same menu answers the
 * context-menu key and Shift+F10 on anything focused in the row, and a long
 * press on touch, where iOS fires no `contextmenu` at all.
 *
 * It holds only what exists: play, add to the queue, the properties panel, and
 * the tag editor for those who may correct. RFC-012 adds download and filing
 * in later steps.
 */
export function useTrackMenu() {
  const [opened, setOpened] = useState<Opened | null>(null);
  // The row's focus travels with the panel: the `<dialog>` hands focus back
  // to what was focused when it opened, which is the menu entry, gone by then.
  const [properties, setProperties] = useState<{
    song: Song;
    returnTo: HTMLElement | null;
  } | null>(null);
  // A long press that opened the menu is followed, as the finger lifts, by the
  // click the browser synthesises for the tap it thinks it saw; on a row whose
  // title plays the track, that click would start playback under the menu.
  const swallowClick = useRef(false);
  const press = useRef<{
    timer: number;
    x: number;
    y: number;
  } | null>(null);

  function open(songs: Song[], position: number, x: number, y: number) {
    setOpened({
      songs,
      position,
      x,
      y,
      returnTo:
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null,
    });
  }

  function cancelPress() {
    if (press.current) window.clearTimeout(press.current.timer);
    press.current = null;
  }

  const rowProps = (songs: Song[], position: number) => ({
    onContextMenu: (event: MouseEvent<HTMLElement>) => {
      event.preventDefault();
      cancelPress();
      // Android opens the menu from a long press itself, through this event,
      // before the timer below fires: the lift that follows is the same tap
      // to swallow. The next touch disarms it, so it cannot eat a later one.
      const native = event.nativeEvent as globalThis.MouseEvent & {
        pointerType?: string;
      };
      if (native.pointerType === "touch") swallowClick.current = true;
      // A keyboard-raised `contextmenu` carries no pointer position; the menu
      // then opens beside the row instead of in the window's corner.
      if (event.clientX === 0 && event.clientY === 0) {
        const rect = event.currentTarget.getBoundingClientRect();
        open(songs, position, rect.left + 24, rect.bottom);
      } else {
        open(songs, position, event.clientX, event.clientY);
      }
    },
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
      if (
        event.key === "ContextMenu" ||
        (event.key === "F10" && event.shiftKey)
      ) {
        event.preventDefault();
        const rect = event.currentTarget.getBoundingClientRect();
        open(songs, position, rect.left + 24, rect.bottom);
      }
    },
    onClickCapture: (event: MouseEvent<HTMLElement>) => {
      if (!swallowClick.current) return;
      swallowClick.current = false;
      event.preventDefault();
      event.stopPropagation();
    },
    onTouchStart: (event: TouchEvent<HTMLElement>) => {
      swallowClick.current = false;
      const touch = event.touches[0];
      if (!touch || event.touches.length > 1) return;
      cancelPress();
      const x = touch.clientX;
      const y = touch.clientY;
      press.current = {
        x,
        y,
        timer: window.setTimeout(() => {
          press.current = null;
          swallowClick.current = true;
          open(songs, position, x, y);
        }, LONG_PRESS_MS),
      };
    },
    onTouchMove: (event: TouchEvent<HTMLElement>) => {
      const touch = event.touches[0];
      const start = press.current;
      if (
        start &&
        touch &&
        Math.hypot(touch.clientX - start.x, touch.clientY - start.y) >
          LONG_PRESS_SLOP_PX
      ) {
        cancelPress();
      }
    },
    onTouchEnd: cancelPress,
    onTouchCancel: cancelPress,
  });

  function close() {
    const returnTo = opened?.returnTo;
    setOpened(null);
    returnTo?.focus();
  }

  const menu = (
    <>
      {opened ? (
        <TrackMenu
          opened={opened}
          onClose={close}
          onProperties={(song) => {
            setProperties({ song, returnTo: opened.returnTo });
            setOpened(null);
          }}
        />
      ) : null}
      {properties ? (
        <TrackProperties
          song={properties.song}
          onClose={() => {
            const returnTo = properties.returnTo;
            setProperties(null);
            returnTo?.focus();
          }}
        />
      ) : null}
    </>
  );

  return { rowProps, menu };
}

function TrackMenu({
  opened,
  onClose,
  onProperties,
}: {
  opened: Opened;
  onClose: () => void;
  onProperties: (song: Song) => void;
}) {
  const { t } = useI18n();
  const player = usePlayer();
  const navigate = useNavigate();
  const { libraries } = useLibraryScope();
  const list = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState({ left: opened.x, top: opened.y });
  const song = opened.songs[opened.position];
  const mayCorrect = mayCorrectTracks(
    libraries.find((library) => library.id === song.library_id),
  );

  // Kept inside the window: a menu opened near the right or bottom edge
  // would otherwise hang off it, half unreadable.
  useLayoutEffect(() => {
    const node = list.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    const margin = 8;
    setPlace({
      left: Math.max(
        margin,
        Math.min(opened.x, window.innerWidth - rect.width - margin),
      ),
      top: Math.max(
        margin,
        Math.min(opened.y, window.innerHeight - rect.height - margin),
      ),
    });
    node.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
  }, [opened.x, opened.y]);

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
  // also reset `armed` below in the middle of a gesture.
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    const away = (event: Event) => {
      if (!list.current?.contains(event.target as Node)) closeRef.current();
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
    document.addEventListener("wheel", away, { capture: true, passive: true });
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
      document.removeEventListener("wheel", away, true);
      document.removeEventListener("touchstart", arm, true);
      document.removeEventListener("touchmove", dragged, true);
      window.removeEventListener("blur", leave);
      window.removeEventListener("resize", leave);
    };
  }, []);

  const items: Array<{
    key: string;
    icon: Parameters<typeof Icon>[0]["name"];
    label: string;
    run: () => void;
  }> = [
    {
      key: "play",
      icon: "play",
      label: t("trackMenu.play"),
      run: () => player.play(opened.songs, opened.position),
    },
    {
      key: "queue",
      icon: "queue",
      label: t("trackMenu.queue"),
      run: () => player.enqueue([song]),
    },
    {
      key: "properties",
      icon: "info",
      label: t("trackMenu.properties"),
      run: () => onProperties(song),
    },
    ...(mayCorrect
      ? [
          {
            key: "edit",
            icon: "edit" as const,
            label: t("trackMenu.edit"),
            run: () =>
              void navigate({
                to: "/tracks/$trackId/edit",
                params: { trackId: song.id },
              }),
          },
        ]
      : []),
  ];

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
      aria-label={t("trackMenu.label", { title: song.title })}
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
            // Properties replaces the menu with its own dialog, which takes
            // focus; every other entry hands it back to the row.
            if (item.key !== "properties") onClose();
          }}
        >
          <Icon name={item.icon} size={16} />
          <span>{item.label}</span>
        </button>
      ))}
    </div>
  );
}

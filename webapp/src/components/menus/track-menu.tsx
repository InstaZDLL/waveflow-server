import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import {
  type KeyboardEvent,
  type MouseEvent,
  type TouchEvent,
  useEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

import { placeTracks, type Song, streamUrl } from "../../api";
import { creditedArtists } from "../../lib/artist-credits";
import { downloadUrl, saveFrom } from "../../lib/download";
import {
  forgetOffline,
  isKeptOffline,
  keepOffline,
  offlineSupported,
} from "../../lib/offline";
import { usePlayer } from "../../player/player";
import { useI18n } from "../../state/i18n";
import { mayCorrectTracks, useLibraryScope } from "../../state/library-scope";
import { FolderPicker } from "../catalog/folder-picker";
import { Icon } from "../layout/icons";
import { TrackProperties } from "../track/track-properties";
import { PopupMenu, type PopupMenuItem } from "./popup-menu";

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
  // Filing, like Properties, replaces the menu with a dialog of its own.
  const [moving, setMoving] = useState<{
    song: Song;
    returnTo: HTMLElement | null;
  } | null>(null);
  const client = useQueryClient();
  const { t } = useI18n();
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
    onTouchEnd: (event: TouchEvent<HTMLElement>) => {
      cancelPress();
      // Cancelling the lift is what stops the browser from synthesising the
      // tap's click at all; `onClickCapture` above stays as the net for a
      // browser that sends one anyway.
      if (swallowClick.current) {
        swallowClick.current = false;
        event.preventDefault();
      }
    },
    onTouchCancel: cancelPress,
  });

  // A download that could not start says so: the menu that offered it is
  // gone, and a click that does nothing reads as a click that was missed.
  const [downloadFailed, setDownloadFailed] = useState(false);
  useEffect(() => {
    if (!downloadFailed) return;
    const timer = window.setTimeout(() => setDownloadFailed(false), 6000);
    return () => window.clearTimeout(timer);
  }, [downloadFailed]);

  function close() {
    const returnTo = opened?.returnTo;
    setOpened(null);
    returnTo?.focus();
  }

  // The menu and the alert are fixed to the viewport, and `fixed` is only that
  // when no ancestor makes itself their containing block: a page block rising
  // into place carries a transform while it moves, and a fixed child of it is
  // placed inside the block instead — the alert landed under the page header.
  // A portal to the body takes them out of every ancestor that could. The
  // properties dialog needs none: `showModal` puts it in the top layer.
  const menu = (
    <>
      {opened
        ? createPortal(
            <TrackMenu
              opened={opened}
              onClose={close}
              onDownloadFailed={() => setDownloadFailed(true)}
              onProperties={(song) => {
                setProperties({ song, returnTo: opened.returnTo });
                setOpened(null);
              }}
              onMove={(song) => {
                setMoving({ song, returnTo: opened.returnTo });
                setOpened(null);
              }}
            />,
            document.body,
          )
        : null}
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
      {moving ? (
        <FolderPicker
          libraryId={moving.song.library_id}
          title={t("filed.moveTrack", { title: moving.song.title })}
          rootLabel={t("filed.unfiled")}
          nested={false}
          onPick={async (folderId) => {
            await placeTracks(
              moving.song.library_id,
              [moving.song.id],
              folderId,
            );
            await Promise.all([
              client.invalidateQueries({ queryKey: ["filed-tree"] }),
              client.invalidateQueries({ queryKey: ["folder-tracks"] }),
            ]);
          }}
          onClose={() => {
            const returnTo = moving.returnTo;
            setMoving(null);
            returnTo?.focus();
          }}
        />
      ) : null}
      {downloadFailed
        ? createPortal(
            <DownloadFailed onDismiss={() => setDownloadFailed(false)} />,
            document.body,
          )
        : null}
    </>
  );

  return { rowProps, menu };
}

/** Fixed above the player, where it is seen whatever the table's length. */
function DownloadFailed({ onDismiss }: { onDismiss: () => void }) {
  const { t } = useI18n();
  return (
    <div className="track-menu-alert" role="alert">
      <span>{t("trackMenu.downloadFailed")}</span>
      <button
        type="button"
        className="track-menu-alert-dismiss"
        aria-label={t("trackMenu.dismiss")}
        onClick={onDismiss}
      >
        <Icon name="close" size={16} />
      </button>
    </div>
  );
}

function TrackMenu({
  opened,
  onClose,
  onProperties,
  onMove,
  onDownloadFailed,
}: {
  opened: Opened;
  onClose: () => void;
  onProperties: (song: Song) => void;
  onMove: (song: Song) => void;
  onDownloadFailed: () => void;
}) {
  const { t } = useI18n();
  const player = usePlayer();
  const navigate = useNavigate();
  const { libraries } = useLibraryScope();
  const song = opened.songs[opened.position];
  const mayCorrect = mayCorrectTracks(
    libraries.find((library) => library.id === song.library_id),
  );

  const items: PopupMenuItem[] = [
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
      key: "download",
      icon: "download",
      label: t("trackMenu.download"),
      // The original, through a ticket: the browser cannot send the
      // session on a navigation, and the ticket re-checks access.
      run: () =>
        void streamUrl(song.id)
          .then(({ url }) => saveFrom(downloadUrl(url)))
          .catch(onDownloadFailed),
    },
    // Kept by its id, from the original fetched once through a ticket; the
    // ticket's address is not what is stored (RFC-022, offline listening).
    ...(offlineSupported()
      ? [
          isKeptOffline(song.id)
            ? {
                key: "offline",
                icon: "close" as const,
                label: t("trackMenu.forgetOffline"),
                run: () => void forgetOffline(song.id).catch(onDownloadFailed),
              }
            : {
                key: "offline",
                icon: "download" as const,
                label: t("trackMenu.keepOffline"),
                run: () =>
                  void streamUrl(song.id)
                    .then(({ url }) => keepOffline(song, url))
                    .catch(onDownloadFailed),
              },
        ]
      : []),
    // Where the track belongs, from every list it appears in. The artist
    // column of a song table is hidden on a narrow screen, so this is how a
    // phone reaches an artist from a playlist or a mix.
    ...(song.album_id
      ? [
          {
            key: "album",
            icon: "albums" as const,
            label: t("trackMenu.goToAlbum"),
            run: () =>
              void navigate({
                to: "/albums/$albumId",
                params: { albumId: song.album_id as string },
              }),
          },
        ]
      : []),
    ...creditedArtists(song).map((artist) => ({
      key: `artist-${artist.id}`,
      icon: "artists" as const,
      label: t("trackMenu.goToArtist", { name: artist.name }),
      run: () =>
        void navigate({
          to: "/artists/$artistId",
          params: { artistId: artist.id },
        }),
    })),
    {
      key: "properties",
      icon: "info",
      label: t("trackMenu.properties"),
      run: () => onProperties(song),
      opensDialog: true,
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
          // Filing is gated as a correction is (RFC-012 decision 3).
          {
            key: "move",
            icon: "folder" as const,
            label: t("trackMenu.move"),
            run: () => onMove(song),
            opensDialog: true,
          },
        ]
      : []),
  ];

  return (
    <PopupMenu
      label={t("trackMenu.label", { title: song.title })}
      x={opened.x}
      y={opened.y}
      items={items}
      onClose={onClose}
    />
  );
}

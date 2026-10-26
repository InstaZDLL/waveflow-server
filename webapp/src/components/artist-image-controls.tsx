import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { createPortal } from "react-dom";

import {
  ApiError,
  type ArtistDetail,
  type ArtistImageKind,
  removeArtistImage,
  setArtistImage,
} from "../api";
import { artistQuery } from "../api/queries";
import { type TranslationKey, useI18n } from "../state/i18n";
import { DeezerPortraitPicker } from "./deezer-portrait-picker";
import { Icon } from "./icons";
import { PopupMenu, type PopupMenuItem } from "./popup-menu";

/** What the server takes: recognised from the bytes, these three only. */
const ACCEPT = "image/jpeg,image/png,image/webp";

const LABELS: Record<
  ArtistImageKind,
  {
    change: TranslationKey;
    reset: TranslationKey;
    changed: TranslationKey;
    reverted: TranslationKey;
  }
> = {
  portrait: {
    change: "artistImage.changePortrait",
    reset: "artistImage.resetPortrait",
    changed: "artistImage.portraitChanged",
    reverted: "artistImage.portraitReverted",
  },
  banner: {
    change: "artistImage.changeBanner",
    reset: "artistImage.resetBanner",
    changed: "artistImage.bannerChanged",
    reverted: "artistImage.bannerReverted",
  },
};

/**
 * Change an artist's portrait or banner, or go back to the automatic one
 * (RFC-013). Drawn only for the library's owners and managers; the server
 * refuses anyone else regardless. The actions sit behind one menu button in
 * the header's corner; what they report stays in the header, beside it.
 *
 * A file is sent as it is: no crop, no resize. The banner is framed by the
 * page at a fixed position, as the desktop frames it. The portrait may also
 * come from Deezer, searched and previewed by the server.
 */
export function ArtistImageControls({ artist }: { artist: ArtistDetail }) {
  const { t } = useI18n();
  const client = useQueryClient();
  const pickers = {
    portrait: useRef<HTMLInputElement>(null),
    banner: useRef<HTMLInputElement>(null),
  };
  const [busy, setBusy] = useState(false);
  const [searching, setSearching] = useState(false);
  // Where the menu hangs, under the button's right edge; null while closed.
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const [notice, setNotice] = useState<{ error: boolean; text: string } | null>(
    null,
  );

  async function settle(next?: ArtistDetail) {
    // The answer to the write is the page as it now reads; holding it first
    // keeps the old picture off the screen while everything else re-reads.
    if (next) client.setQueryData(artistQuery(artist.id).queryKey, next);
    await Promise.all([
      client.invalidateQueries({ queryKey: ["artist", artist.id] }),
      client.invalidateQueries({ queryKey: ["artists"] }),
    ]);
  }

  async function choose(kind: ArtistImageKind, file: File) {
    setBusy(true);
    setNotice(null);
    try {
      await settle(await setArtistImage(artist.id, kind, file));
      setNotice({ error: false, text: t(LABELS[kind].changed) });
    } catch (error) {
      setNotice({
        error: true,
        text: t(
          error instanceof ApiError && error.status === 413
            ? "artistImage.tooLarge"
            : error instanceof ApiError && error.status === 422
              ? "artistImage.notAnImage"
              : "artistImage.failed",
        ),
      });
    } finally {
      setBusy(false);
    }
  }

  async function reset(kind: ArtistImageKind) {
    setBusy(true);
    setNotice(null);
    try {
      await removeArtistImage(artist.id, kind);
      await settle();
      setNotice({ error: false, text: t(LABELS[kind].reverted) });
    } catch {
      setNotice({ error: true, text: t("artistImage.failed") });
    } finally {
      setBusy(false);
    }
  }

  const kinds: ArtistImageKind[] = ["portrait", "banner"];
  // In the order a reader meets them: the portrait's three ways, then the
  // banner's two. A reset is offered only where there is a choice to undo.
  const items: PopupMenuItem[] = kinds.flatMap((kind) => [
    {
      key: `change-${kind}`,
      icon: "edit" as const,
      label: t(LABELS[kind].change),
      run: () => pickers[kind].current?.click(),
    },
    ...(kind === "portrait"
      ? [
          {
            key: "deezer",
            icon: "search" as const,
            label: t("artistImage.searchDeezer"),
            run: () => {
              setNotice(null);
              setSearching(true);
            },
          },
        ]
      : []),
    ...(artist.image_choice?.[kind]
      ? [
          {
            key: `reset-${kind}`,
            icon: "close" as const,
            label: t(LABELS[kind].reset),
            run: () => void reset(kind),
          },
        ]
      : []),
  ]);

  function toggle() {
    if (menu) {
      setMenu(null);
      return;
    }
    const box = trigger.current?.getBoundingClientRect();
    if (!box) return;
    setMenu({ x: box.right, y: box.bottom + 6 });
  }

  function closeMenu() {
    setMenu(null);
    trigger.current?.focus();
  }

  return (
    <div className="artist-image-controls">
      {/* One discreet button over the picture rather than a row of five:
          the photograph is what the header is for, and the actions are for
          the few who may take them. */}
      <button
        ref={trigger}
        type="button"
        className="artist-image-button artist-image-menu"
        aria-haspopup="menu"
        aria-expanded={menu !== null}
        // The short word on the button, the whole of it in the name: what
        // the button says is how its name begins (WCAG 2.5.3), and the name
        // says what is being edited.
        aria-label={t("artistImage.menu")}
        disabled={busy}
        onClick={toggle}
      >
        <Icon name="more" size={16} />
        {t("artistImage.menuShort")}
      </button>
      {kinds.map((kind) => (
        <input
          key={kind}
          ref={pickers[kind]}
          type="file"
          accept={ACCEPT}
          hidden
          aria-label={t(LABELS[kind].change)}
          onChange={(event) => {
            const file = event.target.files?.[0];
            // Cleared, so the same file chosen again is a change too.
            event.target.value = "";
            if (file) void choose(kind, file);
          }}
        />
      ))}
      {/* Portalled for the reason the track menu is: the header clips its
          overflow, and the menu would be cut off at its lower edge. And
          outside the header in the tree, so none of its light-on-picture ink
          reaches the menu's own panel. */}
      {menu
        ? createPortal(
            <PopupMenu
              label={t("artistImage.menuLabel", { name: artist.name })}
              x={menu.x}
              y={menu.y}
              align="end"
              items={items}
              onClose={closeMenu}
              anchor={trigger}
            />,
            document.body,
          )
        : null}
      {notice ? (
        <p
          className={notice.error ? "error" : "muted artist-image-notice"}
          role={notice.error ? "alert" : "status"}
        >
          {notice.text}
        </p>
      ) : null}
      {searching ? (
        <DeezerPortraitPicker
          artist={artist}
          onPicked={async (next) => {
            await settle(next);
            setNotice({ error: false, text: t(LABELS.portrait.changed) });
          }}
          onClose={() => setSearching(false)}
        />
      ) : null}
    </div>
  );
}

import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";

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
 * refuses anyone else regardless.
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
  return (
    <div className="artist-image-controls">
      <div className="artist-image-buttons">
        {kinds.map((kind) => (
          <span key={kind} className="artist-image-pair">
            <button
              type="button"
              className="artist-image-button"
              disabled={busy}
              onClick={() => pickers[kind].current?.click()}
            >
              <Icon name="edit" size={15} />
              {t(LABELS[kind].change)}
            </button>
            {kind === "portrait" ? (
              <button
                type="button"
                className="artist-image-button"
                disabled={busy}
                onClick={() => {
                  setNotice(null);
                  setSearching(true);
                }}
              >
                <Icon name="search" size={15} />
                {t("artistImage.searchDeezer")}
              </button>
            ) : null}
            {artist.image_choice?.[kind] ? (
              <button
                type="button"
                className="artist-image-button subtle"
                disabled={busy}
                onClick={() => void reset(kind)}
              >
                {t(LABELS[kind].reset)}
              </button>
            ) : null}
            <input
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
          </span>
        ))}
      </div>
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

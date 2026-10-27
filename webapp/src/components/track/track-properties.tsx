import { useQuery } from "@tanstack/react-query";
import { type ReactNode, useEffect, useId, useRef } from "react";

import { formatDuration, type Song } from "../../api";
import { lyricsQuery, trackPropertiesQuery } from "../../api/queries";
import { useI18n } from "../../state/i18n";
import { Artwork } from "../artwork/artwork";
import { Icon } from "../layout/icons";
import { LyricsSection } from "./lyrics-section";
import { QualitySection } from "./quality-section";

/** One row of a list, left out entirely when it has nothing to say. */
type Row = { label: string; value: ReactNode; corrected?: string | null };

/**
 * A track's properties, read-only (RFC-012 decision 4).
 *
 * A native `<dialog>` opened with `showModal`, for the reasons the "more" sheet
 * gives: the focus trap, Escape, and the page behind made inert all come from
 * the platform, and the accessibility gate measures them.
 *
 * Built from what the server already knows. Nothing is computed here that the
 * catalogue did not measure, and a field the response does not carry is left
 * out rather than printed as zero, which would be a measurement.
 */
export function TrackProperties({
  song,
  onClose,
}: {
  song: Song;
  onClose: () => void;
}) {
  const { t, locale } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const { data, error } = useQuery(trackPropertiesQuery(song.id));
  const { data: lyrics, isSuccess: lyricsKnown } = useQuery(
    lyricsQuery(song.id),
  );

  useEffect(() => {
    const node = dialog.current;
    if (node && !node.open) node.showModal();
  }, []);

  const track = data?.track;
  const tracked = data?.tracked;
  const number = new Intl.NumberFormat(locale, { maximumFractionDigits: 1 });
  // A peak is a sample amplitude: 0.98 and 1 are not the same claim.
  const amplitude = new Intl.NumberFormat(locale, { maximumFractionDigits: 3 });
  const decibels = new Intl.NumberFormat(locale, {
    maximumFractionDigits: 2,
    signDisplay: "exceptZero",
  });
  const known = (value: number | null | undefined) =>
    typeof value === "number" ? value : null;
  // Where a tag was corrected, the file's own value, so the panel can say both.
  const fileSaid = (
    field: keyof NonNullable<typeof tracked>["overrides"],
    source: string | number | null | undefined,
  ) =>
    tracked && tracked.overrides[field] !== null
      ? t("properties.fileSaid", { value: source ?? "—" })
      : null;

  const sizeMb = song.size / (1024 * 1024);
  const bitrate = known(song.bitrate);
  const rate = known(song.sample_rate);
  const depth = known(song.bit_depth);
  const channels = known(song.channels);

  const file: Row[] = [
    {
      label: t("properties.path"),
      value: track ? <code>{track.relative_path}</code> : null,
    },
    {
      label: t("properties.format"),
      value: [song.codec, song.suffix.toUpperCase()]
        .filter(Boolean)
        .filter((value, index, all) => all.indexOf(value) === index)
        .join(" · "),
    },
    {
      label: t("properties.bitrate"),
      value: bitrate === null ? null : t("properties.kbps", { value: bitrate }),
    },
    {
      label: t("properties.sampleRate"),
      value:
        rate === null
          ? null
          : t("properties.khz", { value: number.format(rate / 1000) }),
    },
    {
      label: t("properties.bitDepth"),
      value: depth === null ? null : t("properties.bits", { value: depth }),
    },
    {
      label: t("properties.channels"),
      value: channels === null ? null : String(channels),
    },
    {
      label: t("properties.size"),
      value: t("properties.mb", { value: number.format(sizeMb) }),
    },
    {
      label: t("properties.duration"),
      value: formatDuration(song.duration_ms),
    },
    {
      label: t("properties.added"),
      value: track
        ? new Intl.DateTimeFormat(locale, {
            dateStyle: "medium",
            timeStyle: "short",
          }).format(track.created_at)
        : null,
    },
    {
      label: t("properties.hash"),
      value: track ? <code>{track.full_hash}</code> : null,
    },
  ];

  const tags: Row[] = [
    {
      label: t("properties.title"),
      value: track?.title ?? song.title,
      corrected: fileSaid("title", tracked?.source.title),
    },
    {
      label: t("properties.artists"),
      value:
        track?.artists && track.artists.length > 0
          ? track.artists.map((artist) => artist.name).join(", ")
          : song.artist,
      corrected:
        tracked && tracked.overrides.artists !== null
          ? t("properties.corrected")
          : null,
    },
    { label: t("properties.album"), value: song.album },
    { label: t("properties.albumArtist"), value: track?.album_artist },
    {
      label: t("properties.year"),
      value: track?.year ?? null,
      corrected: fileSaid("year", tracked?.source.year),
    },
    {
      label: t("properties.trackNumber"),
      value: song.track,
      corrected: fileSaid("track_number", tracked?.source.track_number),
    },
    {
      label: t("properties.discNumber"),
      value: song.disc,
      corrected: fileSaid("disc_number", tracked?.source.disc_number),
    },
    {
      label: t("properties.genres"),
      value: track?.genres?.length ? track.genres.join(", ") : null,
      corrected:
        tracked && tracked.overrides.genres !== null
          ? t("properties.corrected")
          : null,
    },
    {
      label: t("properties.comment"),
      value: track?.comment,
      corrected: fileSaid("comment", tracked?.source.comment),
    },
    {
      label: t("properties.isrc"),
      value: track?.isrc?.length ? track.isrc.join(", ") : null,
    },
    {
      label: t("properties.musicbrainz"),
      value: track?.musicbrainz_id ? <code>{track.musicbrainz_id}</code> : null,
      corrected: fileSaid(
        "musicbrainz_recording_id",
        tracked?.source.musicbrainz_recording_id,
      ),
    },
  ];

  const gain = (value: number | null | undefined, peak?: number | null) => {
    const db = known(value);
    if (db === null) return null;
    const ceiling = known(peak);
    return ceiling === null
      ? t("properties.db", { value: decibels.format(db) })
      : t("properties.dbPeak", {
          value: decibels.format(db),
          peak: amplitude.format(ceiling),
        });
  };
  const sound: Row[] = [
    {
      label: t("properties.trackGain"),
      value: gain(track?.replay_gain_track_gain, track?.replay_gain_track_peak),
    },
    {
      label: t("properties.albumGain"),
      value: gain(track?.replay_gain_album_gain, track?.replay_gain_album_peak),
    },
    { label: t("properties.bpm"), value: known(track?.bpm) },
  ];

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: Escape closes a <dialog> natively
    <dialog
      ref={dialog}
      className="track-properties"
      aria-labelledby={titleId}
      onClose={onClose}
      onClick={(event) => {
        // The backdrop is the dialog element too, so its target alone does
        // not tell them apart: a click in the panel's padding or between two
        // rows also lands on the dialog itself. Outside its box is the
        // backdrop; inside it is the panel.
        const node = dialog.current;
        if (!node || event.target !== node) return;
        const box = node.getBoundingClientRect();
        const inside =
          event.clientX >= box.left &&
          event.clientX <= box.right &&
          event.clientY >= box.top &&
          event.clientY <= box.bottom;
        if (!inside) node.close();
      }}
    >
      <header>
        <Artwork
          artworkId={song.artwork_hash}
          title={song.title}
          className="properties-cover"
        />
        <div>
          <span className="eyebrow">{t("properties.kicker")}</span>
          <h2 id={titleId}>{song.title}</h2>
          <p className="muted">
            {[song.artist, song.album].filter(Boolean).join(" · ")}
          </p>
        </div>
        <button
          type="button"
          className="properties-close"
          aria-label={t("properties.close")}
          onClick={() => dialog.current?.close()}
        >
          <Icon name="close" />
        </button>
      </header>
      {error ? (
        <p className="error" role="alert">
          {t("properties.loadError")}
        </p>
      ) : null}
      <Section title={t("properties.file")} rows={file} />
      <Section title={t("properties.tags")} rows={tags} />
      <Section title={t("properties.sound")} rows={sound} />
      <QualitySection song={song} />
      <LyricsSection song={song} list={lyrics} known={lyricsKnown} />
    </dialog>
  );
}

function Section({ title, rows }: { title: string; rows: Row[] }) {
  const shown = rows.filter(
    (row) => row.value !== null && row.value !== undefined && row.value !== "",
  );
  if (shown.length === 0) return null;
  return (
    <section>
      <h3>{title}</h3>
      <dl className="property-list">
        {shown.map((row) => (
          <div className="property-row" key={row.label}>
            <dt>{row.label}</dt>
            <dd>
              {row.value}
              {row.corrected ? (
                <small className="property-corrected">{row.corrected}</small>
              ) : null}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

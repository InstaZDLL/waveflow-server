import { useQuery } from "@tanstack/react-query";
import { type ReactNode, useEffect, useState } from "react";

import {
  ApiError,
  type QualityVerdict,
  type Song,
  spectrogramUrl,
} from "../api";
import { trackQualityQuery } from "../api/queries";
import {
  frequencyX,
  spectrumGridlines,
  spectrumPath,
  verdictTone,
} from "../lib/quality";
import { type TranslationKey, useI18n } from "../state/i18n";

/** The chart's box, in SVG units. */
const WIDTH = 480;
const HEIGHT = 140;

/**
 * "Real quality" in a track's properties (RFC-016 decision 10): what the
 * audio says against what the container claims, the numbers behind the
 * verdict, the averaged spectrum the verdict was judged on, and a
 * spectrogram drawn when somebody asks for one.
 *
 * Under the file's own facts and never in their place: the meter above grades
 * the container, which is what it says it does, and this says what the audio
 * does. Neither pretends to be the other.
 */
export function QualitySection({ song }: { song: Song }) {
  const { t, locale } = useI18n();
  const { data } = useQuery(trackQualityQuery(song.id));
  // A verdict this client does not know is left unsaid rather than printed as
  // a key: the type above is a claim about the server, not a guarantee.
  if (!data || !Object.hasOwn(VERDICT_LABEL, data.verdict)) return null;
  const kilohertz = new Intl.NumberFormat(locale, { maximumFractionDigits: 1 });
  const number = new Intl.NumberFormat(locale, { maximumFractionDigits: 0 });
  const measured =
    data.verdict !== "pending" &&
    data.verdict !== "lossy" &&
    data.verdict !== "not_measured" &&
    data.verdict !== "unreadable";

  const rows: Array<{ label: string; value: ReactNode }> = [
    {
      label: t("quality.verdictLabel"),
      value: (
        <span
          className={`quality-verdict ${verdictTone(data.verdict)}`}
          data-verdict={data.verdict}
        >
          {t(VERDICT_LABEL[data.verdict])}
        </span>
      ),
    },
    {
      label: t("quality.highsStop"),
      value: !measured
        ? null
        : data.cutoff_hz === null
          ? t("quality.fullBand")
          : t("quality.khz", {
              value: kilohertz.format(data.cutoff_hz / 1000),
            }),
    },
    {
      label: t("quality.wall"),
      value:
        measured && data.cutoff_hz !== null && data.wall_db !== null
          ? t("quality.db", { value: number.format(data.wall_db) })
          : null,
    },
    {
      label: t("quality.bitsUsed"),
      value:
        data.effective_bits === null
          ? null
          : typeof song.bit_depth === "number"
            ? t("quality.bitsOf", {
                used: data.effective_bits,
                declared: song.bit_depth,
              })
            : String(data.effective_bits),
    },
    {
      label: t("quality.analysed"),
      value:
        data.analyzed_at === null
          ? null
          : new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(
              data.analyzed_at,
            ),
    },
  ];
  const shown = rows.filter((row) => row.value !== null);

  return (
    <section className="quality-section">
      <h3>{t("quality.section")}</h3>
      <p className="muted quality-explain">
        {t(VERDICT_EXPLAIN[data.verdict])}
      </p>
      <dl className="property-list">
        {shown.map((row) => (
          <div className="property-row" key={row.label}>
            <dt>{row.label}</dt>
            <dd>{row.value}</dd>
          </div>
        ))}
      </dl>
      {data.spectrum && data.nyquist_hz ? (
        <SpectrumChart
          spectrum={data.spectrum}
          nyquistHz={data.nyquist_hz}
          cutoffHz={data.cutoff_hz}
          label={
            data.cutoff_hz === null
              ? t("quality.chartFullBand")
              : t("quality.chart", {
                  where: t("quality.khz", {
                    value: kilohertz.format(data.cutoff_hz / 1000),
                  }),
                })
          }
        />
      ) : null}
      <Spectrogram song={song} />
    </section>
  );
}

function SpectrumChart({
  spectrum,
  nyquistHz,
  cutoffHz,
  label,
}: {
  spectrum: number[];
  nyquistHz: number;
  cutoffHz: number | null;
  label: string;
}) {
  const { locale } = useI18n();
  const kilohertz = new Intl.NumberFormat(locale, { maximumFractionDigits: 0 });
  return (
    <figure className="quality-spectrum">
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT + 16}`}
        role="img"
        aria-label={label}
        preserveAspectRatio="none"
      >
        {spectrumGridlines(nyquistHz).map((hz) => {
          const x = frequencyX(hz, nyquistHz, WIDTH);
          return (
            <g key={hz} className="quality-gridline">
              <line x1={x} x2={x} y1={0} y2={HEIGHT} />
              <text x={x} y={HEIGHT + 12} textAnchor="middle">
                {kilohertz.format(hz / 1000)}
              </text>
            </g>
          );
        })}
        <path
          className="quality-curve"
          d={spectrumPath(spectrum, WIDTH, HEIGHT)}
        />
        {cutoffHz === null ? null : (
          <line
            className="quality-wall"
            x1={frequencyX(cutoffHz, nyquistHz, WIDTH)}
            x2={frequencyX(cutoffHz, nyquistHz, WIDTH)}
            y1={0}
            y2={HEIGHT}
          />
        )}
      </svg>
      <figcaption className="muted">{label}</figcaption>
    </figure>
  );
}

/**
 * Drawn on demand and never before: a picture costs the server a decode, so
 * nobody pays for one they did not ask for. The object URL is revoked when the
 * section goes.
 */
function Spectrogram({ song }: { song: Song }) {
  const { t } = useI18n();
  const [state, setState] = useState<
    | { kind: "idle" }
    | { kind: "drawing" }
    | { kind: "shown"; url: string }
    | { kind: "failed"; busy: boolean }
  >({ kind: "idle" });
  const shownUrl = state.kind === "shown" ? state.url : null;
  useEffect(
    () => () => {
      if (shownUrl) URL.revokeObjectURL(shownUrl);
    },
    [shownUrl],
  );

  async function draw() {
    setState({ kind: "drawing" });
    try {
      setState({ kind: "shown", url: await spectrogramUrl(song.id) });
    } catch (error) {
      setState({
        kind: "failed",
        busy: error instanceof ApiError && error.status === 429,
      });
    }
  }

  if (state.kind === "shown") {
    return (
      <figure className="quality-spectrogram">
        <img
          src={state.url}
          alt={t("quality.spectrogramAlt", { title: song.title })}
        />
      </figure>
    );
  }
  return (
    <div className="quality-spectrogram-action">
      <button
        type="button"
        onClick={() => void draw()}
        disabled={state.kind === "drawing"}
      >
        {state.kind === "drawing"
          ? t("quality.drawing")
          : t("quality.showSpectrogram")}
      </button>
      {state.kind === "failed" ? (
        <p className="error" role="alert">
          {t(
            state.busy
              ? "quality.spectrogramBusy"
              : "quality.spectrogramFailed",
          )}
        </p>
      ) : null}
    </div>
  );
}

/** What each verdict is called, on the page and in the properties. */
export const VERDICT_LABEL: Record<QualityVerdict, TranslationKey> = {
  lossy_source: "quality.verdict.lossy_source",
  upsampled: "quality.verdict.upsampled",
  padded: "quality.verdict.padded",
  unreadable: "quality.verdict.unreadable",
  silent: "quality.verdict.silent",
  pending: "quality.verdict.pending",
  consistent: "quality.verdict.consistent",
  lossy: "quality.verdict.lossy",
  not_measured: "quality.verdict.not_measured",
};

/** One sentence on what each verdict means, under the section's title. */
const VERDICT_EXPLAIN: Record<QualityVerdict, TranslationKey> = {
  lossy_source: "quality.explain.lossy_source",
  upsampled: "quality.explain.upsampled",
  padded: "quality.explain.padded",
  unreadable: "quality.explain.unreadable",
  silent: "quality.explain.silent",
  pending: "quality.explain.pending",
  consistent: "quality.explain.consistent",
  lossy: "quality.explain.lossy",
  not_measured: "quality.explain.not_measured",
};

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useId, useState } from "react";

import {
  deleteTranscodingProfile,
  saveTranscodingProfile,
  type TranscodingProfile,
} from "../../api";
import {
  transcodeStatusQuery,
  transcodingProfilesQuery,
} from "../../api/queries";
import { Loading, useReload } from "../../components/layout/page-shell";
import {
  formatLabel,
  offeredFormats,
  readBitrate,
} from "../../lib/transcoding";
import { useI18n } from "../../state/i18n";

/**
 * The account's transcoding profile (RFC-027), on the page otherwise about this
 * browser, and saying so.
 *
 * **It does not say which apps seek badly**, because nobody has measured that.
 * It says what may happen and what puts it right: a row of Original for that
 * app.
 */

/** How `format` spells the original, on the wire and in these selects. */
const ORIGINAL = "raw";

type Outcome = "saved" | "error" | "range" | null;

/** One request at a time, and what became of the last one. */
function useProfileWrite() {
  const reload = useReload(transcodingProfilesQuery().queryKey);
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<Outcome>(null);

  async function write(action: () => Promise<unknown>): Promise<boolean> {
    setBusy(true);
    setOutcome(null);
    try {
      await action();
      setOutcome("saved");
      reload();
      return true;
    } catch {
      setOutcome("error");
      return false;
    } finally {
      setBusy(false);
    }
  }

  return { busy, outcome, setOutcome, write };
}

function WriteOutcome({ outcome }: { outcome: Outcome }) {
  const { t } = useI18n();
  if (outcome === "saved") {
    return (
      <output className="muted" aria-live="polite">
        {t("transcoding.saved")}
      </output>
    );
  }
  if (outcome === null) return null;
  return (
    <p className="error" role="alert">
      {t(
        outcome === "range" ? "transcoding.bitrateRange" : "transcoding.error",
      )}
    </p>
  );
}

/** A format and, unless it is the original, the ceiling it is encoded at. */
function ProfileFields({
  format,
  bitrate,
  formats,
  onFormat,
  onBitrate,
}: {
  format: string;
  bitrate: string;
  formats: string[];
  onFormat: (format: string) => void;
  onBitrate: (bitrate: string) => void;
}) {
  const { t } = useI18n();
  return (
    <>
      <label className="control">
        <span>{t("transcoding.format")}</span>
        <select
          value={format}
          onChange={(event) => onFormat(event.target.value)}
        >
          <option value={ORIGINAL}>{t("transcoding.original")}</option>
          {offeredFormats(formats, format).map((name) => (
            <option key={name} value={name}>
              {formatLabel(name)}
            </option>
          ))}
        </select>
      </label>
      {format === ORIGINAL ? null : (
        <label className="control">
          <span>{t("transcoding.bitrate")}</span>
          <input
            type="number"
            min={32}
            max={512}
            value={bitrate}
            placeholder={t("transcoding.bitrateDefault")}
            onChange={(event) => onBitrate(event.target.value)}
          />
        </label>
      )}
    </>
  );
}

/**
 * The row every app without its own falls back on. Original here is no row at
 * all, which is what the server takes it to mean: the account asks for nothing.
 */
function DefaultProfile({
  current,
  formats,
}: {
  current: TranscodingProfile | undefined;
  formats: string[];
}) {
  const { t } = useI18n();
  const { busy, outcome, setOutcome, write } = useProfileWrite();
  const [format, setFormat] = useState(current?.format ?? ORIGINAL);
  const [bitrate, setBitrate] = useState(current?.bitrate?.toString() ?? "");

  async function save(event: FormEvent) {
    event.preventDefault();
    if (format === ORIGINAL) {
      await write(() => deleteTranscodingProfile(null));
      return;
    }
    const ceiling = readBitrate(bitrate);
    if (ceiling === "invalid") {
      setOutcome("range");
      return;
    }
    await write(async () => {
      const saved = await saveTranscodingProfile({
        format,
        ...(ceiling === null ? {} : { bitrate: ceiling }),
      });
      // The format's default, when the field was left empty, is a number now.
      setBitrate(saved.bitrate?.toString() ?? "");
    });
  }

  return (
    <form
      className="transcoding-form"
      noValidate
      aria-label={t("transcoding.allApps")}
      onSubmit={(event) => void save(event)}
    >
      <ProfileFields
        format={format}
        bitrate={bitrate}
        formats={formats}
        onFormat={setFormat}
        onBitrate={setBitrate}
      />
      <button type="submit" disabled={busy}>
        {t("transcoding.save")}
      </button>
      <WriteOutcome outcome={outcome} />
    </form>
  );
}

/** The rows for one app, and the form that adds or replaces one. */
function ClientProfiles({
  rows,
  seen,
  formats,
}: {
  rows: TranscodingProfile[];
  seen: string[];
  formats: string[];
}) {
  const { t } = useI18n();
  const suggestions = useId();
  const { busy, outcome, setOutcome, write } = useProfileWrite();
  const [client, setClient] = useState("");
  const [format, setFormat] = useState(ORIGINAL);
  const [bitrate, setBitrate] = useState("");

  async function add(event: FormEvent) {
    event.preventDefault();
    const ceiling = format === ORIGINAL ? null : readBitrate(bitrate);
    if (ceiling === "invalid") {
      setOutcome("range");
      return;
    }
    const added = await write(() =>
      saveTranscodingProfile({
        client,
        format,
        ...(ceiling === null ? {} : { bitrate: ceiling }),
      }),
    );
    if (added) {
      setClient("");
      setBitrate("");
    }
  }

  const named = new Set(rows.map((row) => row.client?.toLowerCase()));
  return (
    <>
      <h4>{t("transcoding.oneApp")}</h4>
      <p className="muted">{t("transcoding.oneAppDetail")}</p>
      {rows.length ? (
        <ul className="list resource-list">
          {rows.map((row) => (
            <li key={row.client}>
              <div>
                <strong>{row.client}</strong>
                <small className="muted">
                  {row.bitrate === null
                    ? t("transcoding.original")
                    : t("transcoding.rate", {
                        format: formatLabel(row.format),
                        bitrate: row.bitrate,
                      })}
                </small>
              </div>
              <button
                type="button"
                className="danger"
                disabled={busy}
                aria-label={`${t("transcoding.remove")}: ${row.client}`}
                onClick={() =>
                  void write(() => deleteTranscodingProfile(row.client))
                }
              >
                {t("transcoding.remove")}
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">{t("transcoding.none")}</p>
      )}
      <form
        className="transcoding-form"
        noValidate
        aria-label={t("transcoding.oneApp")}
        onSubmit={(event) => void add(event)}
      >
        <label className="control">
          <span>{t("transcoding.client")}</span>
          <input
            value={client}
            list={suggestions}
            maxLength={120}
            required
            onChange={(event) => setClient(event.target.value)}
          />
          <datalist id={suggestions}>
            {seen
              .filter((name) => !named.has(name.toLowerCase()))
              .map((name) => (
                <option key={name} value={name} />
              ))}
          </datalist>
        </label>
        <ProfileFields
          format={format}
          bitrate={bitrate}
          formats={formats}
          onFormat={setFormat}
          onBitrate={setBitrate}
        />
        <button type="submit" disabled={busy || !client.trim()}>
          {t("transcoding.add")}
        </button>
        <WriteOutcome outcome={outcome} />
      </form>
    </>
  );
}

export function TranscodingProfilesPanel() {
  const { t } = useI18n();
  const profiles = useQuery(transcodingProfilesQuery());
  // The formats, once: this panel has no use for the count the administration
  // screen polls this route for.
  const status = useQuery({
    ...transcodeStatusQuery(),
    refetchInterval: false,
    staleTime: 60_000,
  });

  return (
    <article className="settings-panel">
      <h3>{t("transcoding.title")}</h3>
      <p className="muted">{t("transcoding.detail")}</p>
      <p className="muted">{t("transcoding.seeking")}</p>
      {profiles.data && status.data ? (
        <>
          <DefaultProfile
            current={profiles.data.profiles.find((row) => row.client === null)}
            formats={status.data.formats}
          />
          <ClientProfiles
            rows={profiles.data.profiles.filter((row) => row.client !== null)}
            seen={profiles.data.seen_clients}
            formats={status.data.formats}
          />
        </>
      ) : (
        <Loading error={profiles.error ?? status.error} />
      )}
    </article>
  );
}

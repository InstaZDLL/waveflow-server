import { type FormEvent, useEffect, useMemo, useState } from "react";

import {
  authorizeLastFm,
  discardUncertainScrobble,
  getTrack,
  linkScrobble,
  listHistory,
  listScrobbleDestinations,
  listScrobbleLinks,
  listUncertainScrobbles,
  retryUncertainScrobble,
  type ScrobbleLink,
  type ScrobbleProvider,
  type ScrobbleUnavailable,
  type UncertainScrobble,
  unlinkScrobble,
} from "./api";
import { type Locale, type TranslationKey, useI18n } from "./i18n";
import { Loading, PageHeader, useAsync } from "./pages";
import {
  playsByInstant,
  returnedFrom,
  type ScrobbleRow,
  scrobbleRows,
  tracksToName,
} from "./scrobbling";

/**
 * The four gestures RFC-010 leaves to a client.
 *
 * Read the instances and offer their names with the reason when one is
 * unavailable; link and unlink by the pair; open the Last.fm journey and send
 * the browser where the server says; and show the queue — above all the
 * listens decision 13 refuses to answer in somebody's place.
 *
 * **The path is not a choice.** `lastfm_callback` finishes the journey and
 * redirects to `/settings/scrobbling`, so this page has to be exactly there or
 * the last step of the journey lands on a 404 — which, until this screen
 * existed, is what it did.
 */

const PROVIDER_NAME: Record<ScrobbleProvider, TranslationKey> = {
  listenbrainz: "scrobbling.provider.listenbrainz",
  maloja: "scrobbling.provider.maloja",
  lastfm: "scrobbling.provider.lastfm",
};

const HEALTH_NAME = {
  healthy: "scrobbling.health.healthy",
  degraded: "scrobbling.health.degraded",
  broken: "scrobbling.health.broken",
} as const satisfies Record<ScrobbleLink["health"], TranslationKey>;

const HEALTH_DETAIL = {
  healthy: "scrobbling.healthDetail.healthy",
  degraded: "scrobbling.healthDetail.degraded",
  broken: "scrobbling.healthDetail.broken",
} as const satisfies Record<ScrobbleLink["health"], TranslationKey>;

/**
 * The normalised causes the server writes, put into words.
 *
 * Anything outside this list is shown as it came. A cause this client has not
 * been taught is still information — hiding it would leave somebody reading
 * "the queue is not moving" with nowhere to go — and a new one on the server
 * must not turn into a blank here.
 */
const REASONS = new Set([
  "rate_limited",
  "retryable",
  "auth_broken",
  "rejected",
  "destination_gone",
  "credential_unreadable",
  "attempts_exhausted",
  "unlinked",
  "ambiguous",
  "interrupted",
]);

/**
 * Why an instance cannot be linked, put into words.
 *
 * The server sent finished English prose here until 2026-09-15 and this page
 * printed it, so a French reader was told in English what to change in a
 * configuration file. It sends the case now and the wording is ours — which
 * also means a case this client has not been taught must not become a blank:
 * `unknown` says that much rather than nothing.
 */
const UNAVAILABLE_REASON = {
  no_application_configured: "scrobbling.unavailable.no_application_configured",
  browser_journey_needs_https:
    "scrobbling.unavailable.browser_journey_needs_https",
} as const satisfies Record<ScrobbleUnavailable, TranslationKey>;

function unavailableReason(
  code: ScrobbleUnavailable | undefined,
): TranslationKey {
  // Read through a wider type on purpose: the union above says what this build
  // knows, and a server is free to be newer than the client reading it.
  const known: Partial<Record<string, TranslationKey>> = UNAVAILABLE_REASON;
  return (code && known[code]) || "scrobbling.unavailable.unknown";
}

function when(instant: number, locale: Locale): string {
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(instant));
}

function keyOf(provider: ScrobbleProvider, destination: string): string {
  return `${provider}/${destination}`;
}

export function ScrobblingPage() {
  const { t } = useI18n();
  // One counter for both halves. `scrobble_links` stops counting an entry the
  // moment it is answered, so answering one below has to move the counters
  // above — showing a decision as outstanding after it was made is the same
  // fault as not showing it at all.
  const [revision, setRevision] = useState(0);
  const refresh = () => setRevision((n) => n + 1);
  // Read once, on mount. It describes the navigation that brought us here, and
  // a later render is no longer that navigation.
  const returned = useMemo(() => returnedFrom(window.location.search), []);

  // And spent once it is read. A reload is a later navigation, not this one,
  // so leaving the pair in the address would re-announce a journey that did
  // not just happen — and hand anyone who copies the URL the same claim about
  // an account that is not theirs. `replaceState` rather than a navigation:
  // the page is already rendering the answer, and the notice above is held in
  // `returned`, not read back from here. Only the two the callback added are
  // removed, so anything else in the query survives.
  useEffect(() => {
    if (!returned) return;
    const address = new URL(window.location.href);
    address.searchParams.delete("linked");
    address.searchParams.delete("destination");
    window.history.replaceState(window.history.state, "", address);
  }, [returned]);

  const { value, error } = useAsync(async () => {
    const [destinations, links] = await Promise.all([
      listScrobbleDestinations(),
      listScrobbleLinks(),
    ]);
    return scrobbleRows(destinations, links);
  }, [revision]);

  return (
    <section>
      <PageHeader
        title={t("scrobbling.title")}
        detail={t("scrobbling.detail")}
      />
      {returned ? (
        <p className="notice" role="status">
          {t("scrobbling.returned", { name: returned })}
        </p>
      ) : null}
      {value ? (
        value.length ? (
          <ul className="list resource-list scrobble-list">
            {value.map((row) => (
              <DestinationRow
                key={keyOf(row.provider, row.destination)}
                row={row}
                onChanged={refresh}
              />
            ))}
          </ul>
        ) : (
          <p className="muted">{t("scrobbling.none")}</p>
        )
      ) : (
        <Loading error={error} />
      )}
      <UncertainPanel revision={revision} onAnswered={refresh} />
    </section>
  );
}

function DestinationRow({
  row,
  onChanged,
}: {
  row: ScrobbleRow;
  onChanged: () => void;
}) {
  const { t, locale } = useI18n();
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<TranslationKey | null>(null);

  const name = `${t(PROVIDER_NAME[row.provider])} — ${t("scrobbling.instance", {
    name: row.destination,
  })}`;

  async function act(failure: TranslationKey, gesture: () => Promise<unknown>) {
    setBusy(true);
    setFailed(null);
    try {
      await gesture();
      return true;
    } catch {
      setFailed(failure);
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function link(event: FormEvent) {
    event.preventDefault();
    const done = await act("scrobbling.linkError", () =>
      linkScrobble(row.provider, row.destination, secret),
    );
    if (done) {
      setSecret("");
      onChanged();
    }
  }

  async function withdraw() {
    if (
      await act("scrobbling.unlinkError", () =>
        unlinkScrobble(row.provider, row.destination),
      )
    ) {
      onChanged();
    }
  }

  async function connect() {
    await act("scrobbling.connectError", async () => {
      const { authorize_url } = await authorizeLastFm(row.destination);
      // The address comes from this server over an authenticated call, so the
      // check is not suspicion of it — it is that nothing on this page should
      // be able to navigate anywhere but over https, whatever it was handed.
      // The host is deliberately not pinned: it is the server's constant, and
      // duplicating it here would break the journey the day it moved.
      if (new URL(authorize_url).protocol !== "https:") {
        throw new Error("not an https destination");
      }
      window.location.assign(authorize_url);
    });
  }

  return (
    <li className="scrobble-row">
      <div className="scrobble-identity">
        <strong>{name}</strong>
        {row.link ? (
          <>
            <span className={`badge health-${row.link.health}`}>
              {t(HEALTH_NAME[row.link.health])}
            </span>
            <small className="muted">{t(HEALTH_DETAIL[row.link.health])}</small>
          </>
        ) : row.available === null ? (
          <small className="muted">{t("scrobbling.retired")}</small>
        ) : row.available ? (
          <small className="muted">{t("scrobbling.notLinked")}</small>
        ) : (
          <small className="muted">
            {t("scrobbling.unavailable", {
              reason: t(unavailableReason(row.unavailable)),
            })}
          </small>
        )}
      </div>

      {row.link ? <QueueCounters link={row.link} locale={locale} /> : null}

      <div className="scrobble-actions">
        {row.link ? (
          <button
            type="button"
            className="danger"
            disabled={busy}
            onClick={() => void withdraw()}
            aria-label={`${t("scrobbling.unlink")}: ${name}`}
          >
            {busy ? t("scrobbling.working") : t("scrobbling.unlink")}
          </button>
        ) : row.available === true && row.provider === "lastfm" ? (
          <>
            <button
              type="button"
              disabled={busy}
              onClick={() => void connect()}
            >
              {busy ? t("scrobbling.working") : t("scrobbling.connect")}
            </button>
            <small className="muted">{t("scrobbling.connectDetail")}</small>
          </>
        ) : row.available === true ? (
          <form className="inline-form" onSubmit={(event) => void link(event)}>
            <input
              type="password"
              value={secret}
              onChange={(event) => setSecret(event.target.value)}
              aria-label={`${t("scrobbling.secret")}: ${name}`}
              placeholder={t("scrobbling.secret")}
              autoComplete="off"
              required
            />
            <button type="submit" disabled={busy}>
              {busy ? t("scrobbling.working") : t("scrobbling.link")}
            </button>
            <small className="muted">{t("scrobbling.secretHint")}</small>
          </form>
        ) : null}
      </div>

      {failed ? (
        <p className="error" role="alert">
          {t(failed)}
        </p>
      ) : null}
    </li>
  );
}

/** What the queue behind one link looks like: counters, never an echo. */
function QueueCounters({
  link,
  locale,
}: {
  link: ScrobbleLink;
  locale: Locale;
}) {
  const { t } = useI18n();
  return (
    <div className="scrobble-counters">
      <span>{t("scrobbling.pending", { count: link.pending })}</span>
      <span>{t("scrobbling.retrying", { count: link.retrying })}</span>
      <span>{t("scrobbling.uncertainCount", { count: link.uncertain })}</span>
      {link.oldest_pending_at !== null ? (
        <small className="muted">
          {t("scrobbling.oldest", {
            when: when(link.oldest_pending_at, locale),
          })}
        </small>
      ) : null}
      {link.last_success_at !== null ? (
        <small className="muted">
          {t("scrobbling.lastSuccess", {
            when: when(link.last_success_at, locale),
          })}
        </small>
      ) : null}
      {link.last_failure ? (
        <small className="muted">
          {t("scrobbling.lastFailure", {
            reason: reasonText(link.last_failure, t),
          })}
        </small>
      ) : null}
    </div>
  );
}

function reasonText(
  cause: string,
  t: (key: TranslationKey, values?: Record<string, string | number>) => string,
): string {
  return REASONS.has(cause)
    ? t(`scrobbling.reason.${cause}` as TranslationKey)
    : cause;
}

/**
 * The listens the server will not decide about.
 *
 * Loaded beside their titles rather than as bare timestamps. The entry carries
 * none — decision 12 keeps the envelope out of the API — and `played_at` is
 * documented as the handle a client matches against its own history, which is
 * exactly what this does. A listen the history no longer reaches still shows,
 * with its time: refusing to display it would hide a decision that is still
 * owed, which is the one outcome this panel exists to prevent.
 */
function UncertainPanel({
  revision,
  onAnswered,
}: {
  revision: number;
  onAnswered: () => void;
}) {
  const { t, locale } = useI18n();
  const [busy, setBusy] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  const { value, error } = useAsync(async () => {
    const entries = await listUncertainScrobbles();
    if (entries.length === 0)
      return { entries, titles: new Map<string, string>() };
    // Only when there is something to name. An empty list must not cost a
    // history read on every visit to this page.
    const plays = playsByInstant(await listHistory(200));
    const resolved = await Promise.allSettled(
      tracksToName(entries, plays).map((id) => getTrack(id)),
    );
    const byTrack = new Map<string, string>();
    for (const result of resolved) {
      if (result.status === "fulfilled") {
        byTrack.set(result.value.id, result.value.title);
      }
    }
    const titles = new Map<string, string>();
    for (const entry of entries) {
      const track = plays.get(entry.played_at);
      const title = track ? byTrack.get(track) : undefined;
      if (title) titles.set(entry.id, title);
    }
    return { entries, titles };
  }, [revision]);

  async function answer(entry: UncertainScrobble, send: boolean) {
    setBusy(entry.id);
    setFailed(false);
    try {
      await (send
        ? retryUncertainScrobble(entry.id)
        : discardUncertainScrobble(entry.id));
      onAnswered();
    } catch {
      setFailed(true);
    } finally {
      setBusy(null);
    }
  }

  return (
    <article className="scrobble-uncertain">
      <h3>{t("scrobbling.uncertain")}</h3>
      <p className="muted">{t("scrobbling.uncertainDetail")}</p>
      {failed ? (
        <p className="error" role="alert">
          {t("scrobbling.answerError")}
        </p>
      ) : null}
      {value ? (
        value.entries.length ? (
          <ul className="list resource-list">
            {value.entries.map((entry) => (
              <li key={entry.id}>
                <div>
                  <strong>
                    {value.titles.get(entry.id) ?? t("scrobbling.unnamedTrack")}
                  </strong>
                  <small className="muted">
                    {t("scrobbling.playedAt", {
                      when: when(entry.played_at, locale),
                    })}
                    {" · "}
                    {t(PROVIDER_NAME[entry.provider])}
                    {" · "}
                    {t("scrobbling.instance", { name: entry.destination })}
                    {" · "}
                    {t("scrobbling.attempts", { count: entry.attempts })}
                    {entry.last_failure
                      ? ` · ${reasonText(entry.last_failure, t)}`
                      : ""}
                  </small>
                </div>
                <div className="scrobble-actions">
                  <button
                    type="button"
                    disabled={busy !== null}
                    onClick={() => void answer(entry, true)}
                  >
                    {t("scrobbling.retry")}
                  </button>
                  <button
                    type="button"
                    className="danger"
                    disabled={busy !== null}
                    onClick={() => void answer(entry, false)}
                  >
                    {t("scrobbling.discard")}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">{t("scrobbling.uncertainNone")}</p>
        )
      ) : (
        <Loading error={error} />
      )}
    </article>
  );
}

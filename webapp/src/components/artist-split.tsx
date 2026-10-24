import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { type ArtistDetail, decideCredit } from "../api";
import { creditFragments } from "../lib/credit-fragments";
import { useI18n } from "../state/i18n";

/** How long the offer waits for its confirming click, as on the desktop. */
const CONFIRM_MS = 4000;

/**
 * The desktop's other way into a split (RFC-015): from the artist page of a
 * joined credit, for the library's owners and managers. The first click arms
 * it and says into how many; a second within four seconds splits.
 *
 * Split, this artist no longer exists — its tracks now credit the parts — so
 * the page leaves for the artists list rather than staying on a `404`. The
 * decision can be undone from "Needs attention".
 */
export function ArtistSplit({ artist }: { artist: ArtistDetail }) {
  const { t } = useI18n();
  const client = useQueryClient();
  const navigate = useNavigate();
  const parts = creditFragments(artist.name);
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(false), CONFIRM_MS);
    return () => window.clearTimeout(timer);
  }, [armed]);

  async function split() {
    setBusy(true);
    setFailed(false);
    try {
      await decideCredit(artist.library_id, artist.id, "split");
      // Away first: this page's own query would otherwise be asked again,
      // and answer 404, before the artists list could be reached.
      await navigate({ to: "/artists" });
      await client.invalidateQueries();
    } catch {
      setFailed(true);
      setBusy(false);
    }
  }

  return (
    <div className="artist-split">
      <button
        type="button"
        className={`artist-image-button${armed ? " armed" : ""}`}
        disabled={busy}
        onClick={() => {
          if (armed) {
            setArmed(false);
            void split();
          } else {
            setArmed(true);
          }
        }}
      >
        {t(armed ? "artistSplit.confirm" : "artistSplit.offer", {
          count: parts.length,
        })}
      </button>
      {failed ? (
        <p className="error" role="alert">
          {t("artistSplit.failed")}
        </p>
      ) : null}
    </div>
  );
}

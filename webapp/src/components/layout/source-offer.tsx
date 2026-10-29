import { useQuery } from "@tanstack/react-query";

import { serverIdentityQuery } from "../../api/queries";
import { useI18n } from "../../state/i18n";

/**
 * The server's offer of its own source (AGPL-3.0 §13): which version is
 * running, and a link to the source of exactly that version.
 *
 * Drawn on the sign-in page, which everyone using the server over the network
 * reaches, and under every section of Settings for whoever is signed in. The
 * address is the server's (`/health`), never one built here, so it leads to
 * the tag the running binary was built from rather than to `main`.
 *
 * Nothing is drawn until the server has answered: a link to a guessed address
 * would be an offer of the wrong source.
 */
export function SourceOffer() {
  const { t } = useI18n();
  const { data } = useQuery(serverIdentityQuery());
  if (!data) return null;
  return (
    <p className="source-offer muted">
      {t("source.offer", { version: data.version })}{" "}
      <a href={data.source} target="_blank" rel="noreferrer">
        {t("source.link")}
      </a>
    </p>
  );
}

// Where the server is, as seen from this page (RFC-030).

/**
 * The path the server is reached under: empty, except inside Home Assistant,
 * where its ingress serves everything under `/api/hassio_ingress/<token>`.
 *
 * The server writes it into the shell it serves, in
 * `<meta name="waveflow-base">`, and the client reads it once: it never
 * changes for the life of the document, and asking the DOM on every request
 * would buy nothing.
 */
let base: string | undefined;

export function serverBase(): string {
  if (base === undefined) {
    const content =
      typeof document === "undefined"
        ? null
        : document
            .querySelector('meta[name="waveflow-base"]')
            ?.getAttribute("content");
    base = content?.replace(/\/+$/, "") ?? "";
  }
  return base;
}

/** For tests: forget the base read, so the next call reads the page again. */
export function forgetServerBase(): void {
  base = undefined;
}

/**
 * A path on this server, as this page must ask for it.
 *
 * Every server path in the client goes through here: the API calls through
 * `request`, the few direct `fetch`es, the tickets the server answers as root
 * paths, the images the shell's own files are. A path that does not start at
 * the root — a full URL, another site's, a `blob:` — is returned unchanged, so
 * a value that may be either can be passed as it is.
 */
export function serverUrl(path: string): string {
  if (!path.startsWith("/") || path.startsWith("//")) return path;
  return `${serverBase()}${path}`;
}

/** Whether this page is served under a prefix, inside Home Assistant. */
export function underPrefix(): boolean {
  return serverBase() !== "";
}

/**
 * The credentials of a request the server answers without a session — the
 * probe, a public share.
 *
 * `omit` at the root, so nothing of this page's travels where nothing is
 * needed. Under Home Assistant's prefix, `same-origin`: Home Assistant admits
 * a request to its ingress only with its own `ingress_session` cookie, and a
 * request that sends none is refused before it reaches the server.
 */
export function publicCredentials(): RequestCredentials {
  return underPrefix() ? "same-origin" : "omit";
}

/**
 * Save a URL the browser can fetch without a header, as a file.
 *
 * A navigation cannot carry the session's `Authorization` header, which is why
 * downloads, like playback, go through a stream ticket. The server names the
 * file in `Content-Disposition`; `download` without a value leaves that name
 * alone, and asks the browser to save rather than open whatever it receives.
 */
export function saveFrom(url: string, name = ""): void {
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.rel = "noopener";
  link.style.display = "none";
  document.body.append(link);
  link.click();
  link.remove();
}

/** The download form of a stream ticket's URL (RFC-012 decision 5). */
export function downloadUrl(ticketUrl: string): string {
  return `${ticketUrl}${ticketUrl.includes("?") ? "&" : "?"}download=1`;
}

/**
 * The name a `Content-Disposition` gives, preferring `filename*`.
 *
 * The server sends both forms (RFC 6266): an ASCII fallback in `filename`
 * and the real name, percent-encoded UTF-8, in `filename*`. A browser saving
 * from a navigation reads them itself; a blob saved from script has to be
 * told, or it is saved as a random identifier.
 */
export function fileNameFrom(disposition: string | null): string | null {
  if (!disposition) return null;
  const extended = /filename\*\s*=\s*UTF-8''([^;]+)/i.exec(disposition);
  if (extended) {
    try {
      return decodeURIComponent(extended[1].trim());
    } catch {
      // A malformed escape falls back to the plain form below.
    }
  }
  const plain = /filename\s*=\s*"([^"]*)"/i.exec(disposition);
  return plain ? plain[1] : null;
}

/** Save a blob under a name, through a short-lived object URL. */
export function saveBlob(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob);
  saveFrom(url, name);
  // Revoked on the next turn, once the click has handed the URL over.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

/** The lyrics format a file's name announces, or `null` for any other. */
export function lyricsFormatOf(name: string): "lrc" | "txt" | "ttml" | null {
  const dot = name.lastIndexOf(".");
  // No dot, no extension: a file called `lrc` announces nothing.
  if (dot < 0) return null;
  const extension = name.slice(dot + 1).toLowerCase();
  return extension === "lrc" || extension === "txt" || extension === "ttml"
    ? extension
    : null;
}

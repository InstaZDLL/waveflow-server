/**
 * Save a URL the browser can fetch without a header, as a file.
 *
 * A navigation cannot carry the session's `Authorization` header, which is why
 * downloads, like playback, go through a stream ticket. The server names the
 * file in `Content-Disposition`; `download` without a value leaves that name
 * alone, and asks the browser to save rather than open whatever it receives.
 */
export function saveFrom(url: string): void {
  const link = document.createElement("a");
  link.href = url;
  link.download = "";
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

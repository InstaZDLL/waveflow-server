import { describe, expect, it } from "vitest";

import {
  isBlockedAsMixedContent,
  isPlayableAddress,
  reportsPlaybackFailure,
} from "./radio-page";

describe("a station address", () => {
  it("is absolute http or https, as the server also insists", () => {
    expect(isPlayableAddress("https://radio.example.org/stream")).toBe(true);
    expect(isPlayableAddress("http://radio.example.org:8000/s.mp3?x=1")).toBe(
      true,
    );
    // Trimmed before judging, because a pasted address carries whitespace.
    expect(isPlayableAddress("  https://radio.example.org/s  ")).toBe(true);

    expect(isPlayableAddress("")).toBe(false);
    expect(isPlayableAddress("   ")).toBe(false);
    expect(isPlayableAddress("radio.example.org/stream")).toBe(false);
    expect(isPlayableAddress("/relative.mp3")).toBe(false);
    // `URL` parses these quite happily, so the protocol test is what refuses
    // them — the same division of labour as the server's, where the parser
    // accepts and the scheme list decides.
    expect(isPlayableAddress("mms://radio.example.org/stream")).toBe(false);
    expect(isPlayableAddress("file:///etc/passwd")).toBe(false);
    expect(isPlayableAddress("javascript:alert(1)")).toBe(false);
  });
});

describe("a clear-text stream on a TLS page", () => {
  it("is named as blocked only where the browser will block it", () => {
    expect(
      isBlockedAsMixedContent("http://radio.example.org/s", "https:"),
    ).toBe(true);
    // The same address is fine on a page served in clear text, which is how a
    // server on a home network is usually reached.
    expect(isBlockedAsMixedContent("http://radio.example.org/s", "http:")).toBe(
      false,
    );
    // And a TLS stream is never the problem.
    expect(
      isBlockedAsMixedContent("https://radio.example.org/s", "https:"),
    ).toBe(false);
    // `https:` is not a prefix of `http:` by accident here — the check must
    // not catch the secure address on the strength of the first four letters.
    expect(
      isBlockedAsMixedContent("  https://radio.example.org/s", "https:"),
    ).toBe(false);
  });
});

describe("a play attempt that failed", () => {
  it("reports only when it is still the one on screen, and not when it was interrupted", () => {
    // The ordinary failure: this attempt is current, and the station is
    // genuinely unreachable.
    expect(
      reportsPlaybackFailure(3, 3, new DOMException("no", "NotSupportedError")),
    ).toBe(true);
    // An element error carries no failure value at all, and still reports.
    expect(reportsPlaybackFailure(3, 3)).toBe(true);

    // A second station was started while this one was loading. Its rejection
    // arrives after the new station is on screen; blaming it would put an
    // error over a station that is playing.
    expect(
      reportsPlaybackFailure(2, 3, new DOMException("aborted", "AbortError")),
    ).toBe(false);
    expect(reportsPlaybackFailure(2, 3)).toBe(false);

    // Stop was pressed while this one was loading. `pause()` rejects the
    // pending promise with `AbortError`, which is the page obeying rather than
    // a station that cannot be reached — and the counter has moved, so either
    // rule alone would catch this one.
    expect(
      reportsPlaybackFailure(3, 3, new DOMException("aborted", "AbortError")),
    ).toBe(false);
  });
});

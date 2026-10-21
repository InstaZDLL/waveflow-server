import { describe, expect, it } from "vitest";

import { downloadUrl, fileNameFrom, lyricsFormatOf } from "./download";

describe("a download's URL", () => {
  it("asks the ticket for the file", () => {
    expect(downloadUrl("/api/v2/stream/abc")).toBe(
      "/api/v2/stream/abc?download=1",
    );
  });

  it("keeps a query the ticket URL already carries", () => {
    expect(downloadUrl("/api/v2/stream/abc?x=1")).toBe(
      "/api/v2/stream/abc?x=1&download=1",
    );
  });
});

describe("a name from Content-Disposition", () => {
  it("prefers the UTF-8 form", () => {
    expect(
      fileNameFrom(
        "attachment; filename=\"Bj_rk - Joga.lrc\"; filename*=UTF-8''Bj%C3%B6rk%20-%20J%C3%B3ga.lrc",
      ),
    ).toBe("Björk - Jóga.lrc");
  });

  it("falls back to the ASCII form", () => {
    expect(fileNameFrom('attachment; filename="plain.txt"')).toBe("plain.txt");
    expect(
      fileNameFrom(
        "attachment; filename=\"ok.txt\"; filename*=UTF-8''%E0%A4%A",
      ),
    ).toBe("ok.txt");
  });

  it("has nothing to say without a header", () => {
    expect(fileNameFrom(null)).toBeNull();
    expect(fileNameFrom("attachment")).toBeNull();
  });
});

describe("a lyrics file's format", () => {
  it("is read off its extension, and nothing else is one", () => {
    expect(lyricsFormatOf("Song.LRC")).toBe("lrc");
    expect(lyricsFormatOf("a.b.ttml")).toBe("ttml");
    expect(lyricsFormatOf("notes.txt")).toBe("txt");
    expect(lyricsFormatOf("song.srt")).toBeNull();
    expect(lyricsFormatOf("lrc")).toBeNull();
  });
});

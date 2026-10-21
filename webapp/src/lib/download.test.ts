import { describe, expect, it } from "vitest";

import { downloadUrl } from "./download";

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

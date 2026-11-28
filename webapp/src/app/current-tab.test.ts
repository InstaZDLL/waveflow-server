import { afterEach, describe, expect, it } from "vitest";

import { forgetServerBase } from "../api/base";
import { isCurrentTab } from "./current-tab";

function at(prefix: string, path: string) {
  for (const meta of document.querySelectorAll('meta[name="waveflow-base"]')) {
    meta.remove();
  }
  const meta = document.createElement("meta");
  meta.name = "waveflow-base";
  meta.content = prefix;
  document.head.append(meta);
  forgetServerBase();
  window.history.replaceState(null, "", `${prefix}${path}`);
}

afterEach(() => at("", "/"));

describe("isCurrentTab", () => {
  it("knows the page open at the root", () => {
    at("", "/genres");
    expect(isCurrentTab("/genres")).toBe(true);
    expect(isCurrentTab("/")).toBe(false);
  });

  it("knows it under Home Assistant's prefix too", () => {
    at("/api/hassio_ingress/tok", "/genres");
    expect(isCurrentTab("/genres")).toBe(true);
    expect(isCurrentTab("/songs")).toBe(false);
  });
});

import { afterEach, describe, expect, it } from "vitest";

import { offlineSupported } from "../lib/offline";
import { forgetServerBase, serverBase, serverUrl, underPrefix } from "./base";

function servedUnder(prefix: string | null) {
  for (const meta of document.querySelectorAll('meta[name="waveflow-base"]')) {
    meta.remove();
  }
  if (prefix !== null) {
    const meta = document.createElement("meta");
    meta.name = "waveflow-base";
    meta.content = prefix;
    document.head.append(meta);
  }
  forgetServerBase();
}

afterEach(() => servedUnder(null));

describe("serverUrl", () => {
  it("leaves every path as it is at the root", () => {
    servedUnder("");
    expect(serverBase()).toBe("");
    expect(underPrefix()).toBe(false);
    expect(serverUrl("/api/v2/albums")).toBe("/api/v2/albums");
    // A page whose shell names no base at all is at the root too.
    servedUnder(null);
    expect(serverUrl("/health")).toBe("/health");
  });

  it("puts a server path under Home Assistant's prefix (RFC-030)", () => {
    servedUnder("/api/hassio_ingress/tok");
    expect(underPrefix()).toBe(true);
    expect(serverUrl("/api/v2/stream/abc")).toBe(
      "/api/hassio_ingress/tok/api/v2/stream/abc",
    );
    expect(serverUrl("/login")).toBe("/api/hassio_ingress/tok/login");
  });

  it("leaves alone what does not start at this server's root", () => {
    servedUnder("/api/hassio_ingress/tok");
    for (const address of [
      "https://radio.example/stream",
      "//cdn.example/x.js",
      "blob:http://localhost/abc",
      "relative/path",
    ]) {
      expect(serverUrl(address)).toBe(address);
    }
  });

  it("offers no offline copy under a prefix (RFC-030 decision 8)", () => {
    // jsdom has no Cache Storage; give it one, so the prefix is what decides.
    const global = globalThis as { caches?: unknown };
    const before = global.caches;
    global.caches = {};
    try {
      servedUnder("");
      expect(offlineSupported()).toBe(true);
      servedUnder("/api/hassio_ingress/tok");
      expect(offlineSupported()).toBe(false);
    } finally {
      global.caches = before;
    }
  });
});

/**
 * The source check of RFC-030 decision 4.
 *
 * It reads **literals only**: a root path written out where the browser is
 * handed it — `fetch`, `new URL`, `new EventSource`, `location.assign` or
 * `replace`, a JSX `src` or `href` — without going through `serverUrl`. A path
 * built in a variable, by concatenation or by a helper is out of its reach,
 * and passing here proves nothing about one; the prefixed end-to-end suite is
 * what catches those, for the paths it runs.
 */
describe("the client's source", () => {
  const sources = import.meta.glob<string>(
    ["../**/*.ts", "../**/*.tsx", "!../**/*.test.ts", "!../**/*.test.tsx"],
    { query: "?raw", import: "default", eager: true },
  );
  const quote = "[\"'`]";
  const root = `${quote}/(?!/)`;
  const sinks = [
    new RegExp(`\\bfetch\\(\\s*${root}`, "g"),
    new RegExp(`\\bnew (?:URL|EventSource)\\(\\s*${root}`, "g"),
    new RegExp(`\\blocation\\.(?:assign|replace)\\(\\s*${root}`, "g"),
    new RegExp(`\\b(?:src|href)=${root}`, "g"),
    new RegExp(`\\b(?:src|href)=\\{\\s*${root}`, "g"),
  ];

  it("reads a real tree", () => {
    // A glob that matched nothing would let the check below pass on nothing.
    expect(Object.keys(sources).length).toBeGreaterThan(100);
    expect(Object.keys(sources)).toContain("./client.ts");
  });

  it("hands the browser no root path that skips serverUrl", () => {
    const found: string[] = [];
    for (const [file, text] of Object.entries(sources)) {
      for (const sink of sinks) {
        for (const match of text.matchAll(sink)) {
          const line = text.slice(0, match.index).split("\n").length;
          found.push(`${file}:${line}: ${match[0]}`);
        }
      }
    }
    expect(found).toEqual([]);
  });
});

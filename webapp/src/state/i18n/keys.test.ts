// Every key lives in exactly one domain file, and every domain is assembled.
import { describe, expect, it } from "vitest";

import { en } from "./en";
import { fr } from "./fr";
import { zhCN } from "./zh-CN";

// A key repeated inside one object literal is a compile error, but the
// assembled objects are spreads, where a later domain silently overwrites an
// earlier one. The translated domains are typed against their English
// counterparts, so a translated key outside its domain fails to compile
// already.
const domains = {
  en: import.meta.glob<Record<string, object>>(
    ["./en/*.ts", "!./en/index.ts"],
    { eager: true },
  ),
  fr: import.meta.glob<Record<string, object>>(
    ["./fr/*.ts", "!./fr/index.ts"],
    { eager: true },
  ),
  "zh-CN": import.meta.glob<Record<string, object>>(
    ["./zh-CN/*.ts", "!./zh-CN/index.ts"],
    { eager: true },
  ),
};
const assembled = { en, fr, "zh-CN": zhCN };

describe.each(["en", "fr", "zh-CN"] as const)(
  "%s translation domains",
  (locale) => {
    const owners = new Map<string, string[]>();
    for (const [file, module] of Object.entries(domains[locale])) {
      for (const strings of Object.values(module)) {
        for (const key of Object.keys(strings)) {
          owners.set(key, [...(owners.get(key) ?? []), file]);
        }
      }
    }

    it("defines each key in one domain file only", () => {
      const repeated = [...owners].filter(([, files]) => files.length > 1);
      expect(repeated).toEqual([]);
    });

    it("assembles every domain file", () => {
      expect([...owners.keys()].sort()).toEqual(
        Object.keys(assembled[locale]).sort(),
      );
    });
  },
);

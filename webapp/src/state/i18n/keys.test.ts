// Every key lives in exactly one domain file, every domain is assembled, and
// every translation keeps what the English string fills in.
import { describe, expect, it } from "vitest";

import { locales, type TranslationValue } from ".";
import { en } from "./en";

// A key repeated inside one object literal is a compile error, but the
// assembled objects are spreads, where a later domain silently overwrites an
// earlier one. The translated domains are typed against their English
// counterparts, so a translated key outside its domain fails to compile
// already.
const files = import.meta.glob<
  Record<string, Record<string, TranslationValue>>
>(["./*/*.ts", "!./*/index.ts"], { eager: true });
const indexes = import.meta.glob<
  Record<string, Record<string, TranslationValue>>
>("./*/index.ts", { eager: true });

function directoryOf(path: string): string {
  return path.split("/")[1];
}

const assembled = new Map(
  Object.entries(indexes).map(([path, module]) => [
    directoryOf(path),
    Object.values(module)[0],
  ]),
);

it("offers every translation directory in the picker", () => {
  expect([...assembled.keys()].sort()).toEqual([...locales].sort());
});

describe.each([...assembled.keys()])("%s translation domains", (locale) => {
  const owners = new Map<string, string[]>();
  for (const [file, module] of Object.entries(files)) {
    if (directoryOf(file) !== locale) continue;
    for (const strings of Object.values(module)) {
      for (const key of Object.keys(strings)) {
        owners.set(key, [...(owners.get(key) ?? []), file]);
      }
    }
  }
  const strings = assembled.get(locale) ?? {};

  it("defines each key in one domain file only", () => {
    const repeated = [...owners].filter(([, files]) => files.length > 1);
    expect(repeated).toEqual([]);
  });

  it("assembles every domain file", () => {
    expect([...owners.keys()].sort()).toEqual(Object.keys(strings).sort());
  });

  // The types check the keys; nothing checks what is inside the strings. A
  // translation that drops `{count}` shows the reader a sentence without its
  // number, and one that drops the form for one loses "mono" for "1 ch".
  it("keeps every placeholder and plural form of the English", () => {
    const differing = Object.entries(en as Record<string, TranslationValue>)
      .filter(([key, english]) => {
        const translated = strings[key];
        return (
          allPlaceholders(english) !== allPlaceholders(translated) ||
          typeof english !== typeof translated
        );
      })
      .map(([key]) => key);
    expect(differing).toEqual([]);
  });
});

function placeholders(text: string): string {
  return [...new Set(text.match(/\{\w+\}/g) ?? [])].sort().join(" ");
}

function allPlaceholders(value: TranslationValue | undefined): string {
  if (value === undefined) return "missing";
  if (typeof value === "string") return placeholders(value);
  return placeholders(`${value.one} ${value.other}`);
}

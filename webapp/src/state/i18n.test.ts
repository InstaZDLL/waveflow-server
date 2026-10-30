import { describe, expect, it } from "vitest";

import { localeFor, translate } from "./i18n";

describe("count translations", () => {
  it("uses singular only for one in English and French", () => {
    expect(translate("en", "common.tracks", { count: 0 })).toBe("0 tracks");
    expect(translate("en", "common.tracks", { count: 1 })).toBe("1 track");
    expect(translate("en", "common.tracks", { count: 2 })).toBe("2 tracks");
    expect(translate("fr", "common.tracks", { count: 0 })).toBe("0 pistes");
    expect(translate("fr", "common.tracks", { count: 1 })).toBe("1 piste");
    expect(translate("fr", "common.tracks", { count: 2 })).toBe("2 pistes");
  });

  // Chinese plural rules call nothing singular, so asking them would never
  // choose the form written for exactly one.
  it("uses the form for one in Chinese too", () => {
    expect(translate("zh-CN", "file.channels", { count: 1 })).toBe("单声道");
    expect(translate("zh-CN", "file.channels", { count: 2 })).toBe("2 声道");
  });
});

describe("the starting language", () => {
  it("follows the browser for the languages the client speaks", () => {
    expect(localeFor("fr-CA")).toBe("fr");
    expect(localeFor("en-GB")).toBe("en");
    expect(localeFor("zh-CN")).toBe("zh-CN");
    expect(localeFor("zh")).toBe("zh-CN");
    expect(localeFor("de-DE")).toBe("de");
    expect(localeFor("de-CH")).toBe("de");
    expect(localeFor("es-419")).toBe("es");
    expect(localeFor("it")).toBe("it");
    expect(localeFor("ja-JP")).toBe("ja");
    expect(localeFor("ko-KR")).toBe("ko");
    expect(localeFor("pt-BR")).toBe("en");
  });

  it("gives each Chinese reader the script they read", () => {
    expect(localeFor("zh-TW")).toBe("zh-TW");
    expect(localeFor("zh-HK")).toBe("zh-TW");
    expect(localeFor("zh-MO")).toBe("zh-TW");
    expect(localeFor("zh-Hant")).toBe("zh-TW");
    expect(localeFor("zh-Hant-CN")).toBe("zh-TW");
    expect(localeFor("zh-Hans-HK")).toBe("zh-CN");
    expect(localeFor("zh-SG")).toBe("zh-CN");
  });
});

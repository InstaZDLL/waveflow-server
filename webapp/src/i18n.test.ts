import { describe, expect, it } from "vitest";

import { translate } from "./i18n";

describe("count translations", () => {
  it("uses singular only for one in English and French", () => {
    expect(translate("en", "common.tracks", { count: 0 })).toBe("0 tracks");
    expect(translate("en", "common.tracks", { count: 1 })).toBe("1 track");
    expect(translate("en", "common.tracks", { count: 2 })).toBe("2 tracks");
    expect(translate("fr", "common.tracks", { count: 0 })).toBe("0 pistes");
    expect(translate("fr", "common.tracks", { count: 1 })).toBe("1 piste");
    expect(translate("fr", "common.tracks", { count: 2 })).toBe("2 pistes");
  });
});

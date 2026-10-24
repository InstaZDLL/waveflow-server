import { describe, expect, it } from "vitest";

import {
  analysisIsWorking,
  frequencyX,
  highsStop,
  SPECTRUM_FLOOR_DB,
  spectrumGridlines,
  spectrumPath,
  verdictTone,
} from "./quality";

describe("the averaged spectrum", () => {
  it("runs from the loudest band at the top to the floor at the bottom", () => {
    expect(spectrumPath([0, SPECTRUM_FLOOR_DB], 100, 50)).toBe("M0 0 L100 50");
  });

  it("does not draw below the floor or above the top", () => {
    expect(spectrumPath([5, -500], 10, 10)).toBe("M0 0 L10 10");
  });

  it("draws nothing for nothing", () => {
    expect(spectrumPath([], 100, 50)).toBe("");
  });

  it("places a wall by its share of Nyquist", () => {
    expect(frequencyX(11_025, 22_050, 200)).toBe(100);
    expect(frequencyX(30_000, 22_050, 200)).toBe(200);
  });

  it("offers only the gridlines a file could reach", () => {
    expect(spectrumGridlines(22_050)).toEqual([16_000, 19_000, 20_000, 22_000]);
    expect(spectrumGridlines(20_000)).toEqual([16_000, 19_000]);
    expect(spectrumGridlines(16_000)).toEqual([]);
  });
});

describe("where the highs stop", () => {
  it("says a frequency when there is a wall", () => {
    expect(highsStop("lossy_source", 16_400)).toEqual({
      kind: "at",
      hz: 16_400,
    });
  });

  it("says the band is full only of a file that was measured", () => {
    expect(highsStop("consistent", null)).toEqual({ kind: "fullBand" });
    expect(highsStop("padded", null)).toEqual({ kind: "fullBand" });
  });

  it("says nothing of a silent file, nor of one not measured", () => {
    expect(highsStop("silent", null)).toBeNull();
    expect(highsStop("pending", null)).toBeNull();
    expect(highsStop("unreadable", null)).toBeNull();
    expect(highsStop("lossy", null)).toBeNull();
  });
});

describe("whether the page keeps asking", () => {
  it("asks while the analysis works", () => {
    expect(analysisIsWorking("running")).toBe(true);
    expect(analysisIsWorking("queued")).toBe(true);
    expect(analysisIsWorking("waiting_for_scan")).toBe(true);
  });

  it("stops once it rests", () => {
    expect(analysisIsWorking("idle")).toBe(false);
    expect(analysisIsWorking("off")).toBe(false);
    expect(analysisIsWorking(undefined)).toBe(false);
  });
});

describe("a verdict's tone", () => {
  it("draws an accusation apart from a state", () => {
    expect(verdictTone("lossy_source")).toBe("finding");
    expect(verdictTone("upsampled")).toBe("finding");
    expect(verdictTone("padded")).toBe("mild");
    expect(verdictTone("pending")).toBe("neutral");
    expect(verdictTone("consistent")).toBe("neutral");
  });
});

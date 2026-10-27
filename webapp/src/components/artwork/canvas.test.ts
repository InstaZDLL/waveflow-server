import { describe, expect, it } from "vitest";

import { canvasRefusal } from "./canvas";

/**
 * Each refusal says what the listener can do about it: a quota is not a file
 * that is too large, and neither is a file the server would never take.
 */
describe("canvasRefusal", () => {
  it("names each refusal the canvas route documents", () => {
    expect(canvasRefusal(404)).toBe("canvas.notAllowed");
    expect(canvasRefusal(409)).toBe("canvas.quota");
    expect(canvasRefusal(413)).toBe("canvas.tooLarge");
    expect(canvasRefusal(422)).toBe("canvas.refused");
  });

  it("calls anything else a failure, including no answer at all", () => {
    expect(canvasRefusal(500)).toBe("canvas.error");
    expect(canvasRefusal(503)).toBe("canvas.error");
    expect(canvasRefusal(0)).toBe("canvas.error");
  });
});

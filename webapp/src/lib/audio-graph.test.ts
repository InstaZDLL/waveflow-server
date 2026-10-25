import { describe, expect, it } from "vitest";

import { connectWhenRunning, needsWaking } from "./audio-graph";

/** A context that records what was done to it, and to the graph, in order. */
function fakeContext(options: {
  state: string;
  resumesTo?: string;
  resumeFails?: boolean;
}) {
  const calls: string[] = [];
  const context = {
    state: options.state,
    async resume() {
      calls.push("resume");
      if (options.resumeFails) throw new Error("refused");
      if (options.resumesTo) context.state = options.resumesTo;
    },
    async close() {
      calls.push("close");
      context.state = "closed";
    },
  };
  /** What the player's wiring does: claim the element, last. */
  const wire =
    (fails = false) =>
    () => {
      calls.push("createGain");
      if (fails) throw new Error("already claimed");
      calls.push("createMediaElementSource");
      return "graph";
    };
  return { context, calls, wire };
}

describe("wiring the ReplayGain graph", () => {
  it("wires a running context at once", async () => {
    const { context, calls, wire } = fakeContext({ state: "running" });
    expect(await connectWhenRunning(context, wire())).toBe("graph");
    expect(calls).toEqual(["createGain", "createMediaElementSource"]);
  });

  it("resumes a suspended context before claiming the element", async () => {
    const { context, calls, wire } = fakeContext({
      state: "suspended",
      resumesTo: "running",
    });
    expect(await connectWhenRunning(context, wire())).toBe("graph");
    expect(calls).toEqual(["resume", "createGain", "createMediaElementSource"]);
  });

  it("never claims the element for a context that will not run", async () => {
    // Claimed, the element would play as silence; unclaimed, it plays as it
    // always did, unlevelled.
    const { context, calls, wire } = fakeContext({
      state: "suspended",
      resumesTo: "suspended",
    });
    expect(await connectWhenRunning(context, wire())).toBeNull();
    expect(calls).toEqual(["resume", "close"]);
  });

  it("closes the candidate when the resume or the wiring fails", async () => {
    const refused = fakeContext({ state: "suspended", resumeFails: true });
    expect(
      await connectWhenRunning(refused.context, refused.wire()),
    ).toBeNull();
    expect(refused.calls).toEqual(["resume", "close"]);

    const broken = fakeContext({ state: "running" });
    expect(
      await connectWhenRunning(broken.context, broken.wire(true)),
    ).toBeNull();
    expect(broken.calls).toEqual(["createGain", "close"]);
  });
});

describe("when a graph is asked to run again", () => {
  it("is while the element plays and the context has stopped", () => {
    expect(needsWaking("suspended", false)).toBe(true);
    expect(needsWaking("interrupted", false)).toBe(true);
  });

  it("is not while paused, running or closed", () => {
    expect(needsWaking("suspended", true)).toBe(false);
    expect(needsWaking("running", false)).toBe(false);
    expect(needsWaking("closed", false)).toBe(false);
  });
});

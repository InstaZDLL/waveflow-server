import { describe, expect, it } from "vitest";

import { drainEventStream } from "./api";

/**
 * Scan progress arrives as `text/event-stream` read through `fetch`, because
 * the route wants an `Authorization` header and `EventSource` sends none. That
 * puts the framing on us, and a read never lands on a frame boundary.
 */
describe("drainEventStream", () => {
  it("returns complete frames and keeps the partial tail", () => {
    const drained = drainEventStream(
      'event: snapshot\ndata: {"a":1}\n\nevent: progress\ndata: {"a":2}\n\ndata: {"a"',
    );
    expect(drained.data).toEqual(['{"a":1}', '{"a":2}']);
    // The tail is not an event yet; dropping it here loses one.
    expect(drained.rest).toBe('data: {"a"');
  });

  it("reassembles a payload split across two reads", () => {
    const first = drainEventStream('data: {"total":1');
    expect(first.data).toEqual([]);

    const second = drainEventStream(`${first.rest}23}\n\n`);
    expect(second.data).toEqual(['{"total":123}']);
    expect(second.rest).toBe("");
  });

  it("joins a payload spread over several data lines", () => {
    // The specification concatenates them, and a long `current_path` is how
    // this shows up in practice.
    const drained = drainEventStream('data: {"path":\ndata: "/a/b"}\n\n');
    expect(drained.data).toEqual(['{"path":"/a/b"}']);
  });

  it("ignores keep-alive comments and fields that are not data", () => {
    const drained = drainEventStream(
      ': keep-alive\n\nevent: progress\nid: 4\ndata: {"n":1}\n\n',
    );
    expect(drained.data).toEqual(['{"n":1}']);
  });

  it("has nothing to hand back from an empty buffer", () => {
    expect(drainEventStream("")).toEqual({ data: [], rest: "" });
  });
});

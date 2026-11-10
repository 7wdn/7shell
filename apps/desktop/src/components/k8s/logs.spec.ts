/**
 * The log buffer: what a line is, and what happens when there are too many of them.
 *
 * A pod's log never ends, so the bound is the feature: the two things worth asserting are that the
 * oldest lines go rather than the newest, and that a line which is not timestamped keeps its first
 * word. The second one is how a container that writes its own breadcrumbs loses them.
 */
import { describe, expect, it } from "vitest";

import { LOG_LIMIT, appendLogLines, splitLogLine } from "@/components/k8s/logs";

describe("splitLogLine", () => {
  it("takes the API's timestamp off the front and keeps the message whole", () => {
    expect(splitLogLine("2026-11-10T09:12:33.123456789Z ready to serve")).toEqual({
      at: "2026-11-10T09:12:33.123456789Z",
      text: "ready to serve",
    });
  });

  it("leaves a line without a timestamp as it is", () => {
    expect(splitLogLine("no timestamp here")).toEqual({ at: "", text: "no timestamp here" });
    expect(splitLogLine("")).toEqual({ at: "", text: "" });
  });

  it("does not lose the message when a line is only a timestamp", () => {
    expect(splitLogLine("2026-11-10T09:12:33Z")).toEqual({ at: "2026-11-10T09:12:33Z", text: "" });
  });
});

describe("appendLogLines", () => {
  it("appends in order", () => {
    const buffer = appendLogLines([], ["one", "two"]);
    expect(buffer.map((line) => line.text)).toEqual(["one", "two"]);
  });

  it("drops the oldest lines first when the buffer is full", () => {
    const buffer = appendLogLines([], ["one", "two", "three", "four"], 3);
    expect(buffer.map((line) => line.text)).toEqual(["two", "three", "four"]);
  });

  it("keeps exactly the bound, so the view's memory does not creep up between batches", () => {
    const full = appendLogLines(
      [],
      Array.from({ length: LOG_LIMIT }, (_, index) => `line ${index}`),
    );
    expect(full).toHaveLength(LOG_LIMIT);
    const next = appendLogLines(full, ["newest"]);
    expect(next).toHaveLength(LOG_LIMIT);
    expect(next.at(-1)?.text).toBe("newest");
    expect(next[0]?.text).toBe("line 1");
  });

  it("answers a new array even when there is nothing to add", () => {
    const buffer = appendLogLines([], ["one"]);
    const same = appendLogLines(buffer, []);
    expect(same).toEqual(buffer);
    expect(same).not.toBe(buffer);
  });
});

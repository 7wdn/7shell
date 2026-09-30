/**
 * The path helpers.
 *
 * `ancestorsOf` is the one with a rule worth pinning: it decides which branches the tree opens, and
 * getting it wrong is visible as a tree that does not show where the listing is.
 */
import { describe, expect, it } from "vitest";

import { ancestorsOf } from "@/lib/files/format";

describe("ancestorsOf", () => {
  it("lists every directory that has to be open for a path to be visible", () => {
    expect(ancestorsOf("/var/log/nginx")).toEqual(["/", "/var", "/var/log"]);
  });

  it("leaves out the entry itself, which is what the last segment is", () => {
    expect(ancestorsOf("/etc/hosts")).toEqual(["/", "/etc"]);
  });

  it("answers with the root for a path directly under it", () => {
    expect(ancestorsOf("/home")).toEqual(["/"]);
    expect(ancestorsOf("/")).toEqual(["/"]);
  });

  it("ignores a trailing separator rather than inventing an empty segment", () => {
    expect(ancestorsOf("/var/log/")).toEqual(["/", "/var"]);
  });
});

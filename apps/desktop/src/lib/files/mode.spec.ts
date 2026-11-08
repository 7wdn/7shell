import { describe, expect, it } from "vitest";

import { hasPermission, octalMode, parseOctalMode, togglePermission } from "./mode";

describe("permission modes", () => {
  it("writes only the permission bits, never the file type", () => {
    expect(octalMode(0o100644)).toBe("0644");
    expect(octalMode(0o40755)).toBe("0755");
    expect(octalMode(0o104755)).toBe("4755");
    expect(octalMode(undefined)).toBe("0000");
  });

  it("reads the shapes a person types", () => {
    expect(parseOctalMode("644")).toBe(0o644);
    expect(parseOctalMode("0755")).toBe(0o755);
    expect(parseOctalMode(" 4755 ")).toBe(0o4755);
  });

  it("refuses anything that is not permission bits", () => {
    expect(parseOctalMode("")).toBeNull();
    expect(parseOctalMode("8")).toBeNull();
    expect(parseOctalMode("-1")).toBeNull();
    expect(parseOctalMode("10000")).toBeNull();
    expect(parseOctalMode("rwxr-xr-x")).toBeNull();
  });

  it("reads and toggles the matrix bits", () => {
    expect(hasPermission("0755", 6, 4)).toBe(true);
    expect(hasPermission("0755", 3, 2)).toBe(false);
    expect(hasPermission("0755", 0, 1)).toBe(true);

    expect(togglePermission("0644", 3, 2, true)).toBe("0664");
    expect(togglePermission("0664", 3, 2, false)).toBe("0644");
    // Special bits survive a round trip through the nine ordinary boxes.
    expect(togglePermission("4755", 6, 1, false)).toBe("4655");
  });
});

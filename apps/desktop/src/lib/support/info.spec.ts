import { describe, expect, it } from "vitest";

import {
  buildSupportInfoText,
  connectionKindIds,
  formatOperatingSystem,
  type SupportInfoLabels,
  windowsMarketingName,
} from "@/lib/support/info";
import type { AppInfo } from "@/types/app";

const labels: SupportInfoLabels = {
  appVersion: "7Shell version",
  runtime: "Runtime",
  runtimeDesktop: "Desktop",
  operatingSystem: "Operating system",
  architecture: "Architecture",
  userAgent: "User-Agent",
  connectionTypes: "Connection types",
  unknown: "Unknown",
};

function info(overrides: Partial<AppInfo> = {}): AppInfo {
  return {
    name: "7Shell",
    version: "0.1.3",
    os: "windows",
    osName: "Windows",
    osVersion: "10.0.26300.9974",
    arch: "x86_64",
    debugBuild: false,
    protocolCount: 11,
    descriptorSchemaVersion: 1,
    ...overrides,
  } as AppInfo;
}

describe("the support information", () => {
  it("names Windows 11 by its build, while the kernel still says 10.0", () => {
    expect(windowsMarketingName("10.0.26300.9974")).toBe("Windows 11");
    expect(windowsMarketingName("10.0.19045.3803")).toBe("Windows 10");
    expect(windowsMarketingName("6.3.9600")).toBeNull();
    expect(formatOperatingSystem(info(), "Unknown")).toBe("Windows 11 (10.0.26300.9974)");
  });

  it("joins a platform and its version once", () => {
    expect(formatOperatingSystem(info({ os: "macos", osName: "macOS", osVersion: "14.5" }), "Unknown")).toBe(
      "macOS 14.5",
    );
    expect(
      formatOperatingSystem(info({ os: "linux", osName: "Ubuntu 24.04.1 LTS", osVersion: "24.04.1" }), "Unknown"),
      "a name that already carries the release is not made to say it twice",
    ).toBe("Ubuntu 24.04.1 LTS");
  });

  it("falls back to the identifier when the platform could not name itself", () => {
    expect(formatOperatingSystem(info({ os: "windows", osName: "", osVersion: undefined }), "Unknown")).toBe("Windows");
    expect(formatOperatingSystem(info({ os: "plan9", osName: "", osVersion: undefined }), "Unknown")).toBe("plan9");
    expect(buildSupportInfoText(info({ version: "" }), labels, "")).toContain("7Shell version: Unknown");
  });

  it("lists the container engines rather than their shared protocol", () => {
    const kinds = connectionKindIds();
    expect(kinds).toEqual(expect.arrayContaining(["ssh", "docker", "podman", "nerdctl", "wslc", "kubernetes"]));
    expect(kinds).not.toContain("container");
  });

  it("builds one line per fact, with the user agent and the connection kinds", () => {
    const text = buildSupportInfoText(info(), labels, "Mozilla/5.0 test agent");
    expect(text.split("\n")).toEqual([
      "7Shell version: v0.1.3",
      "Runtime: Desktop",
      "Operating system: Windows 11 (10.0.26300.9974)",
      "Architecture: x86_64",
      "User-Agent: Mozilla/5.0 test agent",
      `Connection types: ${connectionKindIds().join(", ")}`,
    ]);
  });
});

/**
 * The block a person copies into a bug report: what this build is, what it is running on, and what
 * it can connect to.
 *
 * Bug reports go wrong at the environment line: "it does not work on Windows" is a different report
 * on Windows 10 and Windows 11, and "it cannot open containers" is a different one on Docker and on
 * nerdctl. The lines are read by a person and pasted by the same button, so the formatting is
 * plain `Label: value` and the values are the exact strings the application knows.
 */
import type { AppInfo } from "@/types/app";
import { PROTOCOLS } from "@/types/generated/protocols";

export interface SupportInfoLabels {
  appVersion: string;
  runtime: string;
  runtimeDesktop: string;
  operatingSystem: string;
  architecture: string;
  userAgent: string;
  connectionTypes: string;
  unknown: string;
}

/**
 * The name Windows sells its own build under.
 *
 * Windows 11 still reports the kernel as 10.0; the build number is what separates it from Windows
 * 10 (build 22000 and later). A version this does not recognise is left as the raw number rather
 * than guessed at.
 */
export function windowsMarketingName(version: string | null | undefined): string | null {
  const trimmed = version?.trim();
  if (!trimmed) {
    return null;
  }
  const [major, , build] = trimmed.split(".");
  if (!major || !build || !/^\d+$/.test(major) || !/^\d+$/.test(build)) {
    return null;
  }
  if (Number(major) !== 10) {
    return null;
  }
  return Number(build) >= 22000 ? "Windows 11" : "Windows 10";
}

function platformName(info: AppInfo): string {
  const named = info.osName?.trim();
  if (named) {
    return named;
  }
  switch (info.os?.toLowerCase()) {
    case "windows":
      return "Windows";
    case "macos":
    case "darwin":
      return "macOS";
    case "linux":
      return "Linux";
    default:
      return info.os?.trim() || "Unknown";
  }
}

export function formatOperatingSystem(info: AppInfo, unknown: string): string {
  const name = platformName(info);
  const version = info.osVersion?.trim();
  if (!version) {
    return name || unknown;
  }
  if (name.toLowerCase() === "windows") {
    const marketing = windowsMarketingName(version);
    return marketing ? `${marketing} (${version})` : `Windows ${version}`;
  }
  // Some platforms already print the release in the product name (Linux PRETTY_NAME); repeating
  // "24.04 24.04" would make the line look like a parsing bug.
  return name.toLowerCase().includes(version.toLowerCase()) ? name : `${name} ${version}`;
}

function versionWithV(version: string | undefined, unknown: string): string {
  const trimmed = version?.trim();
  if (!trimmed) {
    return unknown;
  }
  return trimmed.startsWith("v") || trimmed.startsWith("V") ? `v${trimmed.slice(1)}` : `v${trimmed}`;
}

/**
 * Every connection kind the picker offers, as ids.
 *
 * `container` is one protocol with four engines behind it; a support report needs the engine that
 * was actually chosen, so the kinds are what it lists.
 */
export function connectionKindIds(): string[] {
  return PROTOCOLS.flatMap((protocol) =>
    protocol.id === "container" ? ["docker", "podman", "nerdctl", "wslc"] : [protocol.id],
  );
}

/** The block as one string, one `Label: value` line each. */
export function buildSupportInfoText(info: AppInfo, labels: SupportInfoLabels, userAgent: string | undefined): string {
  const rows: [string, string][] = [
    [labels.appVersion, versionWithV(info.version, labels.unknown)],
    [labels.runtime, labels.runtimeDesktop],
    [labels.operatingSystem, formatOperatingSystem(info, labels.unknown)],
    [labels.architecture, info.arch?.trim() || labels.unknown],
    [labels.userAgent, userAgent?.trim() || labels.unknown],
    [labels.connectionTypes, connectionKindIds().join(", ")],
  ];
  return rows.map(([label, value]) => `${label}: ${value}`).join("\n");
}

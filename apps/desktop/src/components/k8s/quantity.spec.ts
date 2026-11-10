/**
 * Quantities: the API's spellings, in and out.
 *
 * The failure this prevents is the one a table makes invisible — `10m` sorted below `2m` because the
 * two were compared as text, or a node reported as 100% full because a decimal Gi was read as bytes.
 * Each case here is a spelling the API server really sends.
 */
import { describe, expect, it } from "vitest";

import { formatCpu, formatMemory, numericValueOf, parseCpu, parseMemory, percent } from "@/components/k8s/quantity";

describe("parseCpu", () => {
  it("reads every suffix the API uses into millicores", () => {
    expect(parseCpu("250m")).toBe(250);
    expect(parseCpu("2")).toBe(2000);
    expect(parseCpu("1.5")).toBe(1500);
    expect(parseCpu("250000u")).toBe(250);
    expect(parseCpu("250000000n")).toBe(250);
    expect(parseCpu("1e3")).toBe(1_000_000);
  });

  it("answers zero for what is not a quantity, rather than NaN in a column", () => {
    expect(parseCpu("")).toBe(0);
    expect(parseCpu("plenty")).toBe(0);
  });
});

describe("parseMemory", () => {
  it("reads binary and decimal suffixes into bytes", () => {
    expect(parseMemory("128Mi")).toBe(128 * 1024 ** 2);
    expect(parseMemory("1Gi")).toBe(1024 ** 3);
    expect(parseMemory("2Ti")).toBe(2 * 1024 ** 4);
    expect(parseMemory("1G")).toBe(1e9);
    expect(parseMemory("512")).toBe(512);
    expect(parseMemory("1.5Gi")).toBe(1.5 * 1024 ** 3);
  });

  it("answers zero for what is not a quantity", () => {
    expect(parseMemory("")).toBe(0);
    expect(parseMemory("lots")).toBe(0);
  });
});

describe("formatting", () => {
  it("shows sub-core CPUs in millicores and whole cores without a trailing zero", () => {
    expect(formatCpu(250)).toBe("250m");
    expect(formatCpu(1000)).toBe("1");
    expect(formatCpu(1500)).toBe("1.5");
  });

  it("shows memory in the largest binary unit that fits", () => {
    expect(formatMemory(512)).toBe("512");
    expect(formatMemory(1024)).toBe("1Ki");
    expect(formatMemory(128 * 1024 ** 2)).toBe("128Mi");
    expect(formatMemory(1.5 * 1024 ** 3)).toBe("1.5Gi");
  });

  it("refuses to invent a percentage where there is no total", () => {
    expect(percent(250, 0)).toBe("—");
    expect(percent(250, 1000)).toBe("25%");
  });
});

describe("numericValueOf", () => {
  it("reads percentages, durations, ratios and memory sizes as numbers", () => {
    expect(numericValueOf("42%")).toBe(42);
    expect(numericValueOf("3d")).toBe(3 * 86_400);
    expect(numericValueOf("2/3")).toBe(2);
    expect(numericValueOf("128Mi")).toBe(128 * 1024 ** 2);
    expect(numericValueOf("12")).toBe(12);
  });

  it("answers null for a reading that is not an amount", () => {
    expect(numericValueOf("—")).toBeNull();
    expect(numericValueOf("Running")).toBeNull();
    expect(numericValueOf("")).toBeNull();
  });
});

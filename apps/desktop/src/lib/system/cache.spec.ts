/**
 * What is remembered about the machines behind addresses, and what is refused.
 *
 * The rules worth pinning down are the ones that keep a *stored* map honest, since it outlives the
 * process that wrote it: junk reads as nothing remembered, an entry that describes no machine is
 * dropped, an answer that says nothing never replaces one that said something, and the map cannot
 * grow without end.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  MAX_REMEMBERED_SYSTEMS,
  SYSTEM_CACHE_KEY,
  parseSystemCache,
  readSystemCache,
  rememberSystem,
  saysNothing,
  writeSystemCache,
} from "./cache";
import type { HostSystem } from "@/types/system";

const UBUNTU: HostSystem = {
  name: "Ubuntu 24.04.1 LTS",
  kernel: "Linux",
  distro: "ubuntu",
  distroLike: ["debian"],
};

const SILENT: HostSystem = { name: null, kernel: null, distro: null, distroLike: [] };

beforeEach(() => {
  localStorage.clear();
});

describe("what was remembered", () => {
  it("reads back what it wrote, under the one key it uses", () => {
    writeSystemCache({ "root@example.test:22": UBUNTU });

    expect(readSystemCache()).toEqual({ "root@example.test:22": UBUNTU });
    expect(localStorage.getItem(SYSTEM_CACHE_KEY)).toContain("Ubuntu 24.04.1 LTS");
  });

  it("reads nothing out of junk", () => {
    for (const raw of [null, "", "{", "[]", '"ubuntu"', "12", "null"]) {
      expect(parseSystemCache(raw), raw ?? "null").toEqual({});
    }
  });

  it("drops entries that describe no machine", () => {
    const raw = JSON.stringify({
      "kept@host.test:22": { name: "  ", kernel: "Linux", distro: null },
      "silent@host.test:22": { name: null, kernel: null, distro: null, distroLike: [] },
      "string@host.test:22": "ubuntu",
      "": UBUNTU,
    });

    expect(parseSystemCache(raw)).toEqual({
      "kept@host.test:22": { name: null, kernel: "Linux", distro: null, distroLike: [] },
    });
  });

  it("keeps the fields of an identity and nothing else a stored entry carries", () => {
    const raw = JSON.stringify({ "root@host.test:22": { ...UBUNTU, distroLike: ["debian", 7, null], extra: "no" } });

    expect(parseSystemCache(raw)["root@host.test:22"]).toEqual(UBUNTU);
  });

  it("survives storage that refuses to hold anything", () => {
    // A private-mode WebView throws on the write rather than on the call; the mark still has to be
    // drawn in the run that learned it, so the failure is the next launch's problem.
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    expect(() => writeSystemCache({ "root@host.test:22": UBUNTU })).not.toThrow();
    setItem.mockRestore();
  });
});

describe("one more answer", () => {
  it("moves the address to the end, so the next oldest is the one dropped", () => {
    const first = rememberSystem({}, "a@host.test:22", UBUNTU);
    const second = rememberSystem(first, "b@host.test:22", { ...UBUNTU, distro: "debian" });
    const again = rememberSystem(second, "a@host.test:22", { ...UBUNTU, name: "Ubuntu 25.04" });

    expect(Object.keys(again)).toEqual(["b@host.test:22", "a@host.test:22"]);
    expect(again["a@host.test:22"]?.name).toBe("Ubuntu 25.04");
  });

  it("never lets a host that said nothing replace what was learned", () => {
    const cache = { "a@host.test:22": UBUNTU };

    // The same object comes back, so a caller can write it without comparing contents.
    expect(rememberSystem(cache, "a@host.test:22", SILENT)).toBe(cache);
    expect(
      rememberSystem(cache, "a@host.test:22", { name: null, kernel: null, distro: null, distroLike: ["debian"] }),
    ).toBe(cache);
    expect(rememberSystem(cache, "  ", UBUNTU)).toBe(cache);
  });

  it("keeps the map bounded, dropping the address seen longest ago", () => {
    let cache: Record<string, HostSystem> = {};
    for (let index = 0; index <= MAX_REMEMBERED_SYSTEMS; index++) {
      cache = rememberSystem(cache, `user@host-${index}.test:22`, UBUNTU);
    }

    const addresses = Object.keys(cache);
    expect(addresses.length).toBe(MAX_REMEMBERED_SYSTEMS);
    expect(addresses).not.toContain("user@host-0.test:22");
    expect(addresses.at(-1)).toBe(`user@host-${MAX_REMEMBERED_SYSTEMS}.test:22`);
  });
});

describe("an identity that says nothing", () => {
  it("is one with no name, no kernel and no id — a family hint is not an answer", () => {
    expect(saysNothing(SILENT)).toBe(true);
    expect(saysNothing({ name: null, kernel: null, distro: null, distroLike: ["debian"] })).toBe(true);
    expect(saysNothing(UBUNTU)).toBe(false);
    expect(saysNothing({ name: null, kernel: "Linux", distro: null, distroLike: [] })).toBe(false);
    expect(saysNothing({ name: null, kernel: null, distro: "alpine", distroLike: [] })).toBe(false);
  });
});

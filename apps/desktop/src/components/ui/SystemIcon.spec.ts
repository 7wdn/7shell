/**
 * The mark in a row: that a system with a drawing gets one, that a system without gets none, and
 * that the drawing is sized by the row rather than by the pixel size the file was saved with.
 *
 * The last one is the reason this spec exists at all. The files under `assets/os` are library
 * downloads and carry `width="64" height="64"` of their own; a mark that kept them would be a 64px
 * logo in a 13px list, which is a layout bug that only shows up on screen.
 */
import { type App, createApp, h } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import SystemIcon from "@/components/ui/SystemIcon.vue";
import type { HostSystem } from "@/types/system";

let app: App | undefined;
let host: HTMLDivElement;

function mount(system: HostSystem | null, props: { size?: number; label?: string | null } = {}): void {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(SystemIcon, { system, ...props }) });
  app.mount(host);
}

const UBUNTU: HostSystem = {
  name: "Ubuntu 24.04.1 LTS",
  kernel: "Linux",
  distro: "ubuntu",
  distroLike: ["debian"],
};

beforeEach(() => {
  document.body.innerHTML = "";
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  document.body.innerHTML = "";
});

describe("the mark", () => {
  it("draws the system the host named, inline", () => {
    mount(UBUNTU);

    const svg = host.querySelector("svg");
    expect(svg).not.toBeNull();
    expect(host.innerHTML, "the file's prolog belongs to the file, not the row").not.toContain("<!DOCTYPE");
  });

  it("is sized by the box it was given, which is the wider default", () => {
    mount(UBUNTU);

    const box = host.querySelector(".os-mark") as HTMLElement;
    const svg = host.querySelector("svg");
    // Sixteen rather than the thirteen a stroke icon uses: a mark brings its own white space, and a
    // 13px box made it read as the smaller of the two in a session row.
    expect(box.style.width).toBe("16px");
    expect(box.style.height).toBe("16px");

    // The drawing's own `width` would win over the box's, which is the bug this guards.
    expect(svg?.getAttribute("width")).toBeNull();
    expect(svg?.getAttribute("height")).toBeNull();
  });

  it("takes the size it was asked for", () => {
    mount(UBUNTU, { size: 20 });

    const box = host.querySelector(".os-mark") as HTMLElement;
    expect(box.style.width).toBe("20px");
    expect(box.style.height).toBe("20px");
  });

  it("says what it is when it is given something to say", () => {
    mount(UBUNTU, { label: "Remote system: Ubuntu 24.04.1 LTS" });

    const box = host.querySelector(".os-mark") as HTMLElement;
    expect(box.getAttribute("role")).toBe("img");
    expect(box.getAttribute("aria-label")).toBe("Remote system: Ubuntu 24.04.1 LTS");
    expect(box.getAttribute("aria-hidden")).toBeNull();
  });

  it("is silent when nobody named it, because the row already says what it is", () => {
    mount(UBUNTU);

    const box = host.querySelector(".os-mark") as HTMLElement;
    expect(box.getAttribute("aria-hidden")).toBe("true");
    expect(box.getAttribute("aria-label")).toBeNull();
  });

  it("draws nothing at all for a host that said nothing", () => {
    mount(null);

    expect(host.querySelector("svg")).toBeNull();
    expect(host.querySelector(".os-mark")).toBeNull();
  });

  it("draws nothing for a system this build has no mark for", () => {
    // OpenBSD answered quite clearly; there is simply no drawing for it, and an empty box would be
    // a hole in the row where the caller expects its own fallback.
    mount({ name: "OpenBSD 7.5", kernel: "OpenBSD", distro: null, distroLike: [] });

    expect(host.querySelector("svg")).toBeNull();
    expect(host.querySelector(".os-mark")).toBeNull();
  });
});

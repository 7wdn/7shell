/**
 * The file-type drawing in a row.
 *
 * The table's own suite says which drawing a name gets; this one is about the two things only the
 * component can get wrong: sizing a copied SVG by the row rather than by the pixel size the file
 * carries, and keeping the stroke glyph where there is no type drawing to show.
 */
import { type App, createApp, h } from "vue";
import { afterEach, describe, expect, it } from "vitest";

import FileTypeIcon from "@/components/files/FileTypeIcon.vue";
import type { SftpEntry } from "@/types/session";

let app: App | undefined;

function mount(name: string, kind: SftpEntry["kind"], size?: number, open?: boolean): HTMLElement {
  const host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(FileTypeIcon, { name, kind, size, open }) });
  app.mount(host);
  return host;
}

afterEach(() => {
  app?.unmount();
  app = undefined;
  document.body.innerHTML = "";
});

describe("the file-type drawing", () => {
  it("draws a known type inline and lets the row's box decide its size", () => {
    const host = mount("photo.png", "file");

    const box = host.querySelector<HTMLElement>(".file-type-icon");
    expect(box).not.toBeNull();
    expect(box?.style.width).toBe("16px");
    expect(box?.style.height).toBe("16px");

    const svg = box?.querySelector("svg");
    expect(svg).not.toBeNull();
    // The copied files carry a pixel size; keeping it would draw a 32px picture in a 16px row.
    expect(svg?.getAttribute("width")).toBeNull();
    expect(svg?.getAttribute("height")).toBeNull();
  });

  it("takes the size it was asked for", () => {
    const host = mount("photo.png", "file", 20);

    expect(host.querySelector<HTMLElement>(".file-type-icon")?.style.width).toBe("20px");
  });

  it("keeps the stroke glyph for a file whose type it does not know", () => {
    const host = mount("notes.xyz", "file");

    expect(host.querySelector(".file-type-icon")).toBeNull();
    expect(host.querySelector("svg")).not.toBeNull();
  });

  it("draws a directory as a solid folder that keeps its colour in every theme", () => {
    const host = mount("logs", "directory");

    const box = host.querySelector<HTMLElement>(".file-type-icon");
    expect(box?.classList.contains("text-primary"), "a folder is a drawing, not themed chrome").toBe(false);
    expect(box?.querySelector("svg")?.getAttribute("fill")).toBe("#ffca28");
    expect(box?.querySelectorAll("path")).toHaveLength(1);
  });

  it("draws an unfolded branch with the open folder", () => {
    const host = mount("logs", "directory", 16, true);

    expect(host.querySelector(".file-type-icon")?.querySelectorAll("path")).toHaveLength(2);
  });

  it("keeps the link glyph for a symbolic link", () => {
    const host = mount("latest", "symlink");

    expect(host.querySelector(".file-type-icon")).toBeNull();
    expect(host.querySelector("svg")).not.toBeNull();
  });
});

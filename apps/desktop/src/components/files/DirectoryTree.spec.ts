import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/backend/api", () => ({ backend: {} }));

import DirectoryTree from "@/components/files/DirectoryTree.vue";
import { i18n } from "@/i18n";
import { useFilesStore } from "@/stores/files";
import type { SftpEntry } from "@/types/session";

const HANDLE = "f1";

function directory(path: string): SftpEntry {
  return { name: path.split("/").pop() ?? "", path, kind: "directory", size: 0 };
}

const branches: Record<string, SftpEntry[]> = {
  "/": [directory("/home"), directory("/empty")],
  "/home": [directory("/home/user")],
  "/home/user": [directory("/home/user/logs")],
  "/home/user/logs": [directory("/home/user/logs/archive")],
  "/empty": [],
};

const directories = vi.fn<(handle: string, path: string) => Promise<SftpEntry[]>>();
const navigate = vi.fn<(path: string) => void>();
let currentPath = ref("/");
let host: HTMLDivElement;
let app: App | undefined;

async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
}

async function render(path = "/"): Promise<void> {
  currentPath.value = path;
  app = createApp({
    render: () =>
      h(DirectoryTree, {
        handle: HANDLE,
        currentPath: currentPath.value,
        width: 224,
        onNavigate: navigate,
      }),
  }).use(i18n);
  app.mount(host);
  await settle();
}

function nameButton(path: string): HTMLButtonElement {
  const button = [...host.querySelectorAll<HTMLButtonElement>("button[title]")].find((item) => item.title === path);
  if (!button) {
    throw new Error(`the directory tree has no row for ${path}`);
  }
  return button;
}

function arrow(path: string): HTMLButtonElement {
  const button = nameButton(path).parentElement?.querySelector<HTMLButtonElement>("button[aria-expanded]");
  if (!button) {
    throw new Error(`the directory tree has no arrow for ${path}`);
  }
  return button;
}

function target(path: string, part: "name" | "icon"): Element {
  const element = nameButton(path).querySelector(part === "name" ? "span.truncate" : "svg");
  if (!element) {
    throw new Error(`the directory tree has no ${part} for ${path}`);
  }
  return element;
}

async function doubleClick(path: string, part: "name" | "icon" = "name"): Promise<void> {
  // Dispatching dblclick alone misses the navigation and prop update the two clicks cause first.
  target(path, part).dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
  await settle();
  target(path, part).dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 2 }));
  await settle();
  target(path, part).dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 }));
  await settle();
}

function readsOf(path: string): number {
  return directories.mock.calls.filter(([handle, requested]) => handle === HANDLE && requested === path).length;
}

beforeEach(() => {
  setActivePinia(createPinia());
  currentPath = ref("/");
  directories.mockReset().mockImplementation(async (_handle, path) => branches[path] ?? []);
  navigate.mockReset().mockImplementation((path) => {
    currentPath.value = path;
  });
  vi.spyOn(useFilesStore(), "directories").mockImplementation(directories);
  host = document.createElement("div");
  document.body.append(host);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("double-clicking directories in the tree", () => {
  it.each([0, 1])(
    "keeps a single activation with click count %i as navigation without opening the branch",
    async (detail) => {
      await render();

      target("/home", "name").dispatchEvent(new MouseEvent("click", { bubbles: true, detail }));
      await settle();

      expect(navigate).toHaveBeenCalledExactlyOnceWith("/home");
      expect(currentPath.value).toBe("/home");
      expect(arrow("/home").getAttribute("aria-expanded")).toBe("false");
      expect(readsOf("/home")).toBe(0);
    },
  );

  it.each(["name", "icon"] as const)("opens a branch by its %s while navigating only once", async (part) => {
    await render();

    await doubleClick("/home", part);

    expect(navigate).toHaveBeenCalledExactlyOnceWith("/home");
    expect(currentPath.value).toBe("/home");
    expect(arrow("/home").getAttribute("aria-expanded")).toBe("true");
    expect(nameButton("/home/user").textContent).toContain("user");
    expect(readsOf("/home")).toBe(1);
  });

  it("closes an open branch and reopens its cached children without another read", async () => {
    await render();
    await doubleClick("/home");

    await doubleClick("/home");

    expect(arrow("/home").getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelector('button[title="/home/user"]')).toBeNull();

    await doubleClick("/home");

    expect(arrow("/home").getAttribute("aria-expanded")).toBe("true");
    expect(nameButton("/home/user")).toBeDefined();
    expect(readsOf("/home")).toBe(1);
    expect(navigate).toHaveBeenCalledTimes(3);
  });

  it("leaves arrow clicks independent of navigation and the name's double-click", async () => {
    await render();

    arrow("/home").click();
    await settle();
    expect(arrow("/home").getAttribute("aria-expanded")).toBe("true");
    expect(nameButton("/home/user")).toBeDefined();

    arrow("/home").click();
    await settle();
    expect(arrow("/home").getAttribute("aria-expanded")).toBe("false");

    arrow("/home").dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
    await settle();
    arrow("/home").dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 2 }));
    await settle();
    arrow("/home").dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 }));
    await settle();

    expect(arrow("/home").getAttribute("aria-expanded")).toBe("false");
    expect(navigate).not.toHaveBeenCalled();
    expect(readsOf("/home")).toBe(1);
  });

  it("keeps an opening branch open without duplicating its pending read", async () => {
    let finish!: (entries: SftpEntry[]) => void;
    const pending = new Promise<SftpEntry[]>((resolve) => {
      finish = resolve;
    });
    directories.mockImplementation(async (_handle, path) => (path === "/home" ? pending : (branches[path] ?? [])));
    await render();

    await doubleClick("/home");
    expect(arrow("/home").disabled).toBe(true);
    await doubleClick("/home", "icon");

    expect(arrow("/home").getAttribute("aria-expanded")).toBe("true");
    expect(readsOf("/home")).toBe(1);

    finish(branches["/home"] ?? []);
    await vi.waitFor(() => expect(arrow("/home").disabled).toBe(false));

    expect(nameButton("/home/user")).toBeDefined();
    expect(arrow("/home").getAttribute("aria-expanded")).toBe("true");
    expect(readsOf("/home")).toBe(1);
  });

  it("finishes opening an empty directory without leaving a loading arrow", async () => {
    await render();

    await doubleClick("/empty");

    expect(arrow("/empty").getAttribute("aria-expanded")).toBe("true");
    expect(arrow("/empty").disabled).toBe(false);
    expect(readsOf("/empty")).toBe(1);
  });

  it("keeps the root closed when its navigation answer arrives after the double-click", async () => {
    let finishNavigation!: () => void;
    navigate.mockImplementation((path) => {
      finishNavigation = () => {
        currentPath.value = path;
      };
    });
    await render("/home/user");
    expect(arrow("/").getAttribute("aria-expanded")).toBe("true");

    await doubleClick("/");

    expect(navigate).toHaveBeenCalledExactlyOnceWith("/");
    expect(currentPath.value).toBe("/home/user");
    expect(arrow("/").getAttribute("aria-expanded")).toBe("false");

    finishNavigation();
    await settle();

    expect(currentPath.value).toBe("/");
    expect(arrow("/").getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelector('button[title="/home"]')).toBeNull();

    currentPath.value = "/home/user/logs";
    await settle();

    expect(arrow("/").getAttribute("aria-expanded")).toBe("true");
    expect(arrow("/home").getAttribute("aria-expanded")).toBe("true");
    expect(arrow("/home/user").getAttribute("aria-expanded")).toBe("true");
    expect(nameButton("/home/user/logs")).toBeDefined();
    expect(arrow("/home/user/logs").getAttribute("aria-expanded")).toBe("false");
  });
});

/**
 * The shortcuts page: recording a key, putting one back, and saying when two rows collide.
 *
 * The registry's own rules are tested where they live; this file is about the surface — that a press
 * becomes the binding of the row that was recording, that Escape leaves the row as it was, and that a
 * key two actions in one surface share is said out loud rather than silently losing to whichever one
 * is listened for first.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import ShortcutsSettings from "@/components/settings/ShortcutsSettings.vue";
import { i18n } from "@/i18n";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";

let app: App | undefined;
let host: HTMLDivElement;

const UPLOAD = "Upload the files on the clipboard";
const ACCEPT = "Take the row the arrows are on";
const TRIGGER = "Open the completion list";

function buttonFor(label: string): HTMLButtonElement | null {
  return host.querySelector<HTMLButtonElement>(`button[aria-label="Change the key for: ${label}"]`);
}

function rowFor(label: string): HTMLElement | null {
  return buttonFor(label)?.closest("div") ?? null;
}

function press(target: Element | null | undefined, held: Record<string, unknown>): void {
  target?.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "",
      bubbles: true,
      cancelable: true,
      ctrlKey: false,
      metaKey: false,
      altKey: false,
      shiftKey: false,
      ...held,
    }),
  );
}

async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(ShortcutsSettings) }).use(i18n);
  app.mount(host);
  await nextTick();
}

beforeEach(() => {
  setActivePinia(createPinia());
  document.body.innerHTML = "";
  // The platform decides which glyph is drawn; the assertions below are about the binding, and a
  // store with nothing loaded reads as the platform that spells things out.
  useAppStore().info = {
    name: "7Shell",
    version: "0.1.0",
    os: "windows",
    arch: "x86_64",
    debugBuild: false,
    protocolCount: 1,
    descriptorSchemaVersion: 1,
  };
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the shortcuts page", () => {
  it("lists every action with the key it answers to", async () => {
    await mount();

    expect(host.textContent).toContain(UPLOAD);
    expect(host.textContent).toContain("Ctrl + V");
    expect(host.textContent).toContain("Ctrl + F");
    expect(host.textContent).toContain("Alt + /");
  });

  it("records the combination somebody presses for the row that is recording", async () => {
    await mount();

    const upload = buttonFor(UPLOAD);
    upload?.click();
    await nextTick();
    expect(upload?.textContent?.trim(), "the row says what it is waiting for").toBe("Press a key…");

    press(upload, { key: "k", ctrlKey: true });
    await nextTick();

    expect(usePreferencesStore().keybindings["global.uploadClipboard"]).toBe("Mod+K");
    expect(rowFor(UPLOAD)?.textContent).toContain("Ctrl + K");
    expect(upload?.textContent?.trim(), "and stops waiting").toBe("Change");
  });

  it("takes the press before the browser or the button can act on it", async () => {
    await mount();

    const upload = buttonFor(UPLOAD);
    upload?.click();
    await nextTick();

    const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    upload?.dispatchEvent(event);
    await nextTick();

    expect(event.defaultPrevented, "a combination the button would act on is still a binding").toBe(true);
    expect(usePreferencesStore().keybindings["global.uploadClipboard"]).toBe("Enter");
  });

  it("leaves the key as it was when Escape is pressed", async () => {
    await mount();

    const upload = buttonFor(UPLOAD);
    upload?.click();
    await nextTick();
    press(upload, { key: "Escape" });
    await nextTick();

    expect(usePreferencesStore().keybindings["global.uploadClipboard"]).toBeUndefined();
    expect(upload?.textContent?.trim()).toBe("Change");
  });

  it("puts the default back, and takes the row out of the settings file with it", async () => {
    usePreferencesStore().setKeybinding("terminal.find", "Mod+G");
    await mount();

    expect(host.textContent).toContain("Ctrl + G");
    const reset = [...host.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Reset");
    reset?.click();
    await nextTick();

    expect(usePreferencesStore().keybindings["terminal.find"]).toBeUndefined();
    expect(host.textContent).toContain("Ctrl + F");
  });

  it("unbinds a key somebody does not want", async () => {
    await mount();

    const unbind = [...host.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Unbind");
    expect(unbind, "a row that still answers to its default has nothing to unbind").toBeUndefined();

    usePreferencesStore().setKeybinding("terminal.find", "Mod+G");
    await nextTick();
    const now = [...host.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Unbind");
    now?.click();
    await nextTick();

    expect(usePreferencesStore().keybindings["terminal.find"]).toBe("");
    expect(rowFor("Find in the terminal")?.textContent).toContain("None");
  });

  it("says when two actions in one surface answer to the same key", async () => {
    usePreferencesStore().setKeybinding("commands.completionAccept", "Alt+/");
    await mount();

    // The row that collides names the other one: which of the two wins is an order nobody can see, so
    // the page says who the other is rather than picking one quietly.
    expect(rowFor(ACCEPT)?.textContent).toContain(TRIGGER);
  });
});

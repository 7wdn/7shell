/**
 * The block that chooses the picture and shapes it.
 *
 * What can only be seen here is the block's own arithmetic and hand-over: that the two sliders write
 * the numbers the settings file holds, that a value past the end of a slider is kept inside it, and
 * that the buttons go through the shell rather than reaching for a file — the interface never holds a
 * path, which is the promise the whole feature is built on and the thing a later change could quietly
 * break. What the window then looks like with a picture behind it is the layer's business, and the
 * real-window run is where that is seen.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  backgroundImageChoose: vi.fn(async () => null),
  backgroundImageClear: vi.fn(async () => undefined),
  backgroundImageRead: vi.fn(async () => ""),
}));

import BackgroundSettings from "@/components/settings/BackgroundSettings.vue";
import { i18n } from "@/i18n";
import { backgroundImageChoose, backgroundImageClear } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";

const chooseMock = vi.mocked(backgroundImageChoose);
const clearMock = vi.mocked(backgroundImageClear);

let host: HTMLDivElement;
let app: App | undefined;

function render(): void {
  app = createApp({ render: () => h(BackgroundSettings) }).use(i18n);
  app.mount(host);
}

function slider(name: string): HTMLInputElement {
  const found = host.querySelector<HTMLInputElement>(`input[type="range"][aria-label="${name}"]`);
  if (!found) {
    throw new Error(`the block has no slider named ${name}`);
  }
  return found;
}

function button(label: string): HTMLButtonElement {
  const found = [...host.querySelectorAll("button")].find((candidate) => candidate.textContent?.trim() === label);
  if (!found) {
    throw new Error(`the block has no button saying ${label}`);
  }
  return found;
}

/** What a slider is drawn with, which is also what it will report when it is moved. */
function move(input: HTMLInputElement, value: number): void {
  input.value = String(value);
  input.dispatchEvent(new Event("input"));
}

async function settle(): Promise<void> {
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.append(host);
  chooseMock.mockReset().mockResolvedValue(null);
  clearMock.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  app?.unmount();
  host.remove();
  app = undefined;
});

describe("the background picture's block", () => {
  it("says there is no picture until one is chosen", () => {
    render();

    expect(host.textContent).toContain("No image set");
    expect(() => button("Clear"), "there is nothing to clear").toThrow();
  });

  it("names the picture after the shell has copied one in", async () => {
    chooseMock.mockResolvedValue("holiday photo.jpg");
    render();

    button("Choose image…").click();
    await settle();

    expect(chooseMock).toHaveBeenCalledWith("Image");
    expect(usePreferencesStore().backgroundImage.fileName).toBe("holiday photo.jpg");
    expect(host.textContent).toContain("holiday photo.jpg");
  });

  it("takes the copy away as well as the name when the picture is cleared", async () => {
    usePreferencesStore().setBackgroundImageName("holiday.jpg");
    render();

    button("Clear").click();
    await settle();

    expect(clearMock, "the copy goes through the shell, and now").toHaveBeenCalled();
    expect(usePreferencesStore().backgroundImage.fileName).toBeUndefined();
  });

  it("writes the two numbers the sliders stand for", () => {
    render();

    move(slider("Transparency"), 45);
    move(slider("Blur"), 7);

    expect(usePreferencesStore().backgroundImage.transparency).toBe(45);
    expect(usePreferencesStore().backgroundImage.blur).toBe(7);
  });

  it("keeps a value past the end of its slider inside it", () => {
    render();

    // The sliders themselves cannot go past their ends, but a number arriving from anywhere else can:
    // a window whose surfaces are entirely clear is one nobody can put back.
    usePreferencesStore().setBackgroundTransparency(400);
    usePreferencesStore().setBackgroundBlur(-9);

    expect(usePreferencesStore().backgroundImage.transparency).toBe(95);
    expect(usePreferencesStore().backgroundImage.blur).toBe(0);
    expect(slider("Transparency").getAttribute("max")).toBe("95");
    expect(slider("Blur").getAttribute("max")).toBe("20");
  });
});

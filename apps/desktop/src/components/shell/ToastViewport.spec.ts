/**
 * The drawing half of the toast: which live region a message lands in, and what the line carries.
 *
 * Both regions exist before any message does — that is the point of the two-region design — so the
 * first assertion is that an empty viewport is still two regions.
 */
import { createApp, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ToastViewport from "@/components/shell/ToastViewport.vue";
import { useToast } from "@/composables/useToast";

const { action, visible, toast } = useToast();

const mounted: Array<() => void> = [];

function mount(): HTMLElement {
  const app = createApp(ToastViewport);
  const root = document.createElement("div");
  document.body.append(root);
  app.mount(root);
  mounted.push(() => {
    app.unmount();
    root.remove();
  });
  return root;
}

const politeLine = (root: HTMLElement): Element | null => root.querySelector('[aria-live="polite"] p');
const assertiveLine = (root: HTMLElement): Element | null => root.querySelector('[aria-live="assertive"] p');

describe("the toast viewport", () => {
  beforeEach(() => {
    visible.value = false;
    action.value = undefined;
  });

  afterEach(() => {
    while (mounted.length > 0) {
      mounted.pop()?.();
    }
    document.body.innerHTML = "";
  });

  it("keeps both live regions in the document while nothing is being said", () => {
    const root = mount();

    expect(root.querySelector('[aria-live="polite"]')).not.toBeNull();
    expect(root.querySelector('[aria-live="assertive"]')).not.toBeNull();
    expect(politeLine(root)).toBeNull();
    expect(assertiveLine(root)).toBeNull();
  });

  it("lets a confirmation wait its turn in the polite region", async () => {
    const root = mount();

    toast("Session saved", "confirmation");
    await nextTick();

    expect(politeLine(root)?.textContent).toContain("Session saved");
    expect(assertiveLine(root)).toBeNull();
  });

  it("interrupts for a failure, in the region that can", async () => {
    const root = mount();

    toast("Upload failed", "failure");
    await nextTick();

    expect(assertiveLine(root)?.textContent).toContain("Upload failed");
    expect(assertiveLine(root)?.getAttribute("role")).toBe("alert");
    expect(politeLine(root)).toBeNull();
  });

  it("treats a refusal as an interruption too", async () => {
    const root = mount();

    toast("That name is taken", "refusal");
    await nextTick();

    expect(assertiveLine(root)?.textContent).toContain("That name is taken");
  });

  it("draws the action only when the message brought one, and runs it when clicked", async () => {
    const root = mount();
    const onClick = vi.fn();

    toast("Upload failed", "failure");
    await nextTick();
    expect(root.querySelector("button")).toBeNull();

    toast("Batch finished", "background", { label: "Show results", onClick });
    await nextTick();
    const button = root.querySelector("button");
    expect(button?.textContent?.trim()).toBe("Show results");

    button?.click();
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("takes the action away when the message is replaced by one without it", async () => {
    const root = mount();

    toast("Batch finished", "background", { label: "Show results", onClick: vi.fn() });
    await nextTick();
    expect(root.querySelector("button")).not.toBeNull();

    toast("Copied", "receipt");
    await nextTick();

    expect(politeLine(root)?.textContent).toContain("Copied");
    expect(root.querySelector("button")).toBeNull();
  });
});

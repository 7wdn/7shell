/**
 * The switch: what it says, what switching does, and what it does not do on its own.
 *
 * The two things worth asserting here are the ones a hand-drawn control loses first: the state a
 * reader who cannot see it is told (`role="switch"` and `aria-checked`, which have to agree with
 * the value), and that it is *controlled* — clicking reports a value and changes nothing by itself,
 * so the switch and the setting behind it can never come to disagree.
 *
 * The keyboard case is about reka-ui's own handling rather than about this component, and it is
 * here because a switch nobody can operate without a mouse is a switch that lost half its reason
 * for existing. Enter is what the component listens for; Space reaches it as a click, because the
 * root it draws is a real button.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import Switch from "@/components/ui/Switch.vue";

let app: App | undefined;
let host: HTMLDivElement;
let chosen: boolean[] = [];

/**
 * Mounts the switch. `follow` is whether the caller takes the change, which is what tells a
 * controlled control from one that moves its own handle.
 */
function mount(modelValue: boolean, options: { disabled?: boolean; follow?: boolean } = {}): void {
  const follow = options.follow !== false;
  const disabled = options.disabled === true;

  app = createApp({
    data: () => ({ value: modelValue }),
    render() {
      return h(Switch, {
        modelValue: this.value,
        disabled,
        ariaLabel: "Host monitor",
        "onUpdate:modelValue": (value: boolean) => {
          chosen.push(value);
          if (follow) {
            this.value = value;
          }
        },
      });
    },
  });

  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
}

/** The control itself, which reka-ui draws as a button wearing the switch role. */
function control(): HTMLElement {
  const element = host.querySelector<HTMLElement>('[role="switch"]');
  if (!element) {
    throw new Error("the switch drew no control");
  }
  return element;
}

async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
}

beforeEach(() => {
  chosen = [];
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("what the switch says", () => {
  it("is named by the row it belongs to", async () => {
    mount(false);
    await settle();
    expect(control().getAttribute("aria-label")).toBe("Host monitor");
  });

  it("says whether it is on, so the state is not only in the pixels", async () => {
    mount(true);
    await settle();
    expect(control().getAttribute("aria-checked")).toBe("true");
    expect(control().getAttribute("data-state")).toBe("checked");
  });

  it("says so when it is off", async () => {
    mount(false);
    await settle();
    expect(control().getAttribute("aria-checked")).toBe("false");
    expect(control().getAttribute("data-state")).toBe("unchecked");
  });
});

describe("switching", () => {
  it("reports the state it was switched to", async () => {
    mount(false);
    await settle();

    control().click();
    await settle();

    expect(chosen).toEqual([true]);
    expect(control().getAttribute("aria-checked")).toBe("true");
  });

  it("reports going back off as well", async () => {
    mount(true);
    await settle();

    control().click();
    await settle();

    expect(chosen).toEqual([false]);
    expect(control().getAttribute("aria-checked")).toBe("false");
  });

  /**
   * The value is the caller's, not this component's: a switch that moved before the setting behind
   * it did would be showing a choice that was refused or has not been written yet.
   */
  it("does not move by itself when the caller does not take the change", async () => {
    mount(false, { follow: false });
    await settle();

    control().click();
    await settle();

    expect(chosen).toEqual([true]);
    expect(control().getAttribute("aria-checked")).toBe("false");
  });

  it("switches from the keyboard", async () => {
    mount(false);
    await settle();

    control().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();

    expect(chosen).toEqual([true]);
  });

  it("does not move when it is disabled", async () => {
    mount(false, { disabled: true });
    await settle();

    control().click();
    await settle();

    expect(chosen).toEqual([]);
    expect(control().hasAttribute("disabled")).toBe(true);
  });
});

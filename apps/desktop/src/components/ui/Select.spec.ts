/**
 * The dropdown itself: what it says, what choosing does, and what the keyboard does.
 *
 * The point of this component is that the list is drawn by this application rather than by the
 * platform, so what is asserted here is the part that stops being free once that is true — the
 * value that comes out, the label that goes in, the disabled option that cannot be chosen, and the
 * keyboard working the way a select's keyboard works.
 *
 * The last case is not about this component at all: it is the reason it exists. A native
 * `<select>` reappearing anywhere in the interface is the regression, so it is checked rather than
 * remembered.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import Select, { type SelectOption } from "@/components/ui/Select.vue";

const OPTIONS: SelectOption[] = [
  { value: "", label: "No group" },
  { value: "prod", label: "Production" },
  { value: "lab", label: "Lab", detail: "labuser@127.0.0.1:2222" },
  { value: "retired", label: "Retired", disabled: true },
];

let app: App | undefined;
let host: HTMLDivElement;
let chosen: string[] = [];

function mount(options: readonly SelectOption[] = OPTIONS, modelValue = ""): void {
  app = createApp({
    data: () => ({ value: modelValue }),
    render() {
      return h(Select, {
        modelValue: this.value,
        options,
        placeholder: "Choose a session…",
        ariaLabel: "The group this session belongs to",
        "onUpdate:modelValue": (value: string) => {
          this.value = value;
          chosen.push(value);
        },
      });
    },
  });
  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
}

/** The trigger, which is the combobox reka-ui draws. */
function trigger(): HTMLElement {
  const element = host.querySelector<HTMLElement>('[role="combobox"]');
  if (!element) {
    throw new Error("the select drew no trigger");
  }
  return element;
}

/** The list, which is teleported to the body rather than drawn inside the component. */
function list(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[role="listbox"]');
}

function options(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('[role="option"]')];
}

async function open(): Promise<void> {
  trigger().dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse" }));
  trigger().click();
  await settle();
}

async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  // One turn of the event loop as well: the layer that notices a pointer going down outside is
  // registered on a timer, so that the click which opened the list cannot be the one that closes it.
  await new Promise((resolve) => setTimeout(resolve, 0));
}

/** Choosing, which reka-ui does on the pointer being released rather than on the click. */
function choose(index: number, event = "pointerup"): void {
  options()[index]?.dispatchEvent(new PointerEvent(event, { bubbles: true, pointerType: "mouse" }));
}

beforeEach(() => {
  chosen = [];
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  // The list lives on the body, so unmounting the component is not enough to take it away.
  for (const element of document.querySelectorAll('[role="listbox"]')) {
    element.remove();
  }
});

describe("what the control says", () => {
  it("shows the chosen option's label, and its detail after it", async () => {
    mount(OPTIONS, "lab");
    await settle();
    expect(trigger().textContent).toContain("Lab");
    expect(trigger().textContent).toContain("labuser@127.0.0.1:2222");
  });

  it("shows the placeholder when no option carries the value", async () => {
    mount(OPTIONS, "gone");
    await settle();
    expect(trigger().textContent).toContain("Choose a session…");
  });

  /**
   * The empty string is a real answer here — "No group" is what a session with no group has — and
   * reka-ui would read it as "nothing is chosen" if it were handed over untranslated.
   */
  it("treats the empty option as a choice rather than as an absence", async () => {
    mount(OPTIONS, "");
    await settle();
    expect(trigger().textContent).toContain("No group");
    expect(trigger().textContent).not.toContain("Choose a session…");
  });

  it("names the control for a reader who cannot see the label beside it", async () => {
    mount();
    await settle();
    expect(trigger().getAttribute("aria-label")).toBe("The group this session belongs to");
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
  });
});

describe("choosing", () => {
  it("reports the value of the option that was chosen and closes the list", async () => {
    mount();
    await open();
    expect(list()).not.toBeNull();

    choose(1);
    await settle();

    expect(chosen).toEqual(["prod"]);
    expect(list()).toBeNull();
    expect(trigger().textContent).toContain("Production");
  });

  it("reports the empty string for the option that is worth the empty string", async () => {
    mount(OPTIONS, "prod");
    await open();
    choose(0);
    await settle();
    expect(chosen).toEqual([""]);
  });

  it("does not choose an option that is disabled", async () => {
    mount();
    await open();
    choose(3);
    await settle();
    expect(chosen).toEqual([]);
    expect(list(), "the list stays up, since nothing was chosen").not.toBeNull();
  });

  it("gives the detail to the reader as part of the option's name", async () => {
    mount();
    await open();
    const named = options()[2]?.getAttribute("aria-labelledby");
    expect(named, "the option is labelled by its own text").toBeTruthy();
    expect(document.getElementById(named as string)?.textContent).toContain("labuser@127.0.0.1:2222");
  });
});

describe("the keyboard", () => {
  it("opens on an arrow key and chooses with Enter", async () => {
    mount(OPTIONS, "prod");
    trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    await settle();
    expect(list()).not.toBeNull();

    // The keyboard is on the chosen option, so one more step lands on the one after it.
    trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    await settle();
    options()[2]?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();

    expect(chosen).toEqual(["lab"]);
    expect(list()).toBeNull();
  });
  it("closes on Escape without choosing anything", async () => {
    mount(OPTIONS, "prod");
    await open();
    expect(list()).not.toBeNull();

    list()?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();

    expect(chosen).toEqual([]);
    expect(list()).toBeNull();
  });

  it("closes when the pointer goes down outside it", async () => {
    mount();
    await open();
    expect(list()).not.toBeNull();

    document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    await settle();

    expect(list()).toBeNull();
  });
});

/**
 * What the list says about itself while somebody is reading it.
 *
 * A dropdown has nothing to do with this; a caller whose options are things you have to see — a
 * colour — has. One case checks that the report arrives at all, and the other is the one that
 * matters most: a preview left standing after the list closes is a window wearing something nobody
 * can see the reason for.
 */
describe("the option under the pointer", () => {
  function mountListening(): { hovered: string[]; leave: () => number } {
    const hovered: string[] = [];
    let left = 0;
    app = createApp({
      render: () =>
        h(Select, {
          modelValue: "prod",
          options: OPTIONS,
          onOptionHover: (value: string) => hovered.push(value),
          onOptionLeave: () => {
            left += 1;
          },
        }),
    });
    host = document.createElement("div");
    document.body.append(host);
    app.mount(host);
    return { hovered, leave: () => left };
  }

  it("reports the option the pointer came to rest on", async () => {
    const { hovered } = mountListening();
    await open();

    options()[2]?.dispatchEvent(new PointerEvent("pointerenter"));

    expect(hovered).toEqual(["lab"]);
  });

  it("says the pointer is off the list when the list closes", async () => {
    const { leave } = mountListening();
    await open();
    options()[1]?.dispatchEvent(new PointerEvent("pointerenter"));

    list()?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();

    expect(leave(), "a preview must not outlive the list it came from").toBeGreaterThan(0);
  });
});

/**
 * The one thing that would undo all of the above: somebody reaching for the platform control again
 * because it is one line shorter. The interface has one dropdown, and this is it.
 */
describe("the source tree", () => {
  const SRC = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

  function sources(dir: string): string[] {
    return readdirSync(dir).flatMap((entry) => {
      const full = path.join(dir, entry);
      if (statSync(full).isDirectory()) {
        return entry === "generated" ? [] : sources(full);
      }
      return full.endsWith(".vue") ? [full] : [];
    });
  }

  it("draws no native select anywhere in the interface", () => {
    const offenders = sources(SRC).filter((file) => /<select[\s>]/.test(readFileSync(file, "utf8")));
    expect(offenders, "use components/ui/Select.vue instead of a native <select>").toEqual([]);
  });
});

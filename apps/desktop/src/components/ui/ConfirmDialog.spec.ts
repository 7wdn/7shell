/**
 * The confirmation dialog: what it says, where the keyboard starts and goes back to, and what
 * answers "no".
 *
 * The reasons this component exists are the ones a strip inside a row could not give: a question
 * that can be read on its own, a keyboard that lands on the answer which changes nothing and
 * returns to whoever asked it, and two ways out (Escape, a click outside) that both mean cancel.
 * The last case is the one that keeps a slow action honest: while it is running the dialog stops
 * being dismissible, because what is behind it has already started.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { i18n } from "@/i18n";

let app: App | undefined;
let host: HTMLDivElement;
let answers: string[] = [];

/**
 * A page with one control on it that asks, which is what every caller is: the question is opened by
 * something, and that something is where the keyboard has to come back to.
 */
function mount(options: { loading?: boolean; details?: readonly string[] } = {}): void {
  app = createApp({
    data: () => ({ open: false }),
    render() {
      return h("div", [
        h("button", { id: "asker", onClick: () => (this.open = true) }, "Ask the question"),
        this.open
          ? h(
              ConfirmDialog,
              {
                open: true,
                title: "Delete group",
                message: "Delete the group “Lab”? Its sessions move out to Ungrouped.",
                confirmLabel: "Delete group",
                danger: true,
                loading: options.loading === true,
                details: options.details,
                "onUpdate:open": (value: boolean) => {
                  answers.push(`open:${value}`);
                  this.open = value;
                },
                onConfirm: () => {
                  answers.push("confirm");
                },
              },
              { default: () => h("p", { id: "extra" }, "extra content") },
            )
          : null,
      ]);
    },
  }).use(i18n);

  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
}

/** Opens it the way a caller does: the control that asks takes the focus, then the question is up. */
async function openIt(): Promise<void> {
  asker().focus();
  asker().click();
  await settle();
}

function asker(): HTMLButtonElement {
  const element = host.querySelector<HTMLButtonElement>("#asker");
  if (!element) {
    throw new Error("the harness has no control to ask with");
  }
  return element;
}

/** The dialog is teleported to the body, so it is not inside the host element. */
function dialog(): HTMLElement {
  const element = document.querySelector<HTMLElement>('[role="dialog"], [role="alertdialog"]');
  if (!element) {
    throw new Error("the dialog was not drawn");
  }
  return element;
}

function button(name: string): HTMLButtonElement {
  const element = [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent?.trim() === name,
  );
  if (!element) {
    throw new Error(`no button says ${name}`);
  }
  return element;
}

async function escape(): Promise<void> {
  // Cancelable, because that is what a browser sends and what a handler is allowed to refuse: an
  // event that could not be cancelled would let the "will not be dismissed" case pass by accident.
  dialog().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  await settle();
}

async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(() => {
  answers = [];
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.querySelectorAll('[role="dialog"], [role="alertdialog"]')) {
    node.remove();
  }
});

describe("what the question says", () => {
  it("names the action, says what it takes with it, and repeats the name on the button", async () => {
    mount();
    await openIt();

    expect(dialog().textContent).toContain("Delete group");
    expect(dialog().textContent).toContain("Its sessions move out to Ungrouped");
    expect(button("Delete group")).toBeTruthy();
  });

  it("keeps a secondary cancel, in the shared words", async () => {
    mount();
    await openIt();
    expect(button("Cancel")).toBeTruthy();
  });

  it("lists what the sentence cannot name, when there is a list of it", async () => {
    mount({ details: ["/etc/nginx/nginx.conf", "/etc/nginx/conf.d"] });
    await openIt();

    expect(dialog().textContent).toContain("/etc/nginx/nginx.conf");
    expect(dialog().textContent).toContain("/etc/nginx/conf.d");
  });

  it("draws whatever else the caller needs answered, inside the same question", async () => {
    mount();
    await openIt();
    expect(dialog().textContent).toContain("extra content");
  });
});

describe("answering", () => {
  it("starts the keyboard on the button that changes nothing", async () => {
    mount();
    await openIt();
    expect(document.activeElement).toBe(button("Cancel"));
  });

  it("reports the decision the confirm button stands for", async () => {
    mount();
    await openIt();

    button("Delete group").click();
    await settle();

    expect(answers).toEqual(["confirm"]);
  });

  it("takes Escape as a cancel", async () => {
    mount();
    await openIt();

    await escape();

    expect(answers).toEqual(["open:false"]);
  });

  it("takes a click outside as a cancel too", async () => {
    mount();
    await openIt();

    document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    document.body.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    await settle();

    expect(answers).toEqual(["open:false"]);
  });

  it("puts the keyboard back on the control that asked", async () => {
    mount();
    await openIt();

    await escape();

    expect(document.activeElement).toBe(asker());
  });
});

describe("while the action is running", () => {
  it("refuses to be dismissed with Escape, because what is behind it has already started", async () => {
    mount({ loading: true });
    await openIt();

    await escape();

    expect(answers).toEqual([]);
  });

  it("refuses to be dismissed by a click outside either", async () => {
    mount({ loading: true });
    await openIt();

    document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    await settle();

    expect(answers).toEqual([]);
  });

  it("does not let either button be pressed a second time", async () => {
    mount({ loading: true });
    await openIt();

    expect(button("Delete group").disabled).toBe(true);
    expect(button("Cancel").disabled).toBe(true);
  });
});

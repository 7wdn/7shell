/**
 * The question a sealed export asks: naming the file, taking the passphrase, and saying when it was
 * the wrong one.
 *
 * What only this component can be held to: the field starts empty, the answer goes out exactly once,
 * and a refusal is read in the field that was just filled rather than anywhere else. The question
 * itself belongs to the store — this is only the screen it is asked on.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import ImportPassphraseDialog from "@/components/sessions/ImportPassphraseDialog.vue";
import { i18n } from "@/i18n";
import { ShellError } from "@/lib/backend/errors";
import type { ImportQuestion } from "@/stores/sessions";

let app: App | undefined;
let host: HTMLDivElement;
let answers: (string | null)[] = [];
/** How a test puts a question on screen, the way the store does. */
let ask!: (question: ImportQuestion | null) => void;

function refusal(): ShellError {
  // The envelope a wrong passphrase comes back as, shaped the way the shell sends it.
  return new ShellError({
    version: 1,
    code: "SESSION-BUNDLE-PASSPHRASE",
    messageKey: "errors.session.bundlePassphrase",
    messageParams: {},
    origin: { subsystem: "store" },
    outcome: "notSent",
  });
}

function mount(): void {
  app = createApp({
    data: () => ({ question: null as ImportQuestion | null }),
    render() {
      return h(ImportPassphraseDialog, {
        question: this.question,
        onAnswer: (passphrase: string | null) => answers.push(passphrase),
      });
    },
    mounted() {
      ask = (question) => {
        this.question = question;
      };
    },
  }).use(i18n);

  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
}

/** The dialog is teleported to the body, so it is not inside the host element. */
function dialog(): HTMLElement {
  const element = document.querySelector<HTMLElement>('[role="dialog"]');
  if (!element) {
    throw new Error("the question was not drawn");
  }
  return element;
}

function field(): HTMLInputElement {
  const element = dialog().querySelector<HTMLInputElement>("input[type=password]");
  if (!element) {
    throw new Error("the question has no field to answer in");
  }
  return element;
}

function button(name: string): HTMLButtonElement {
  const element = [...dialog().querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent?.trim() === name,
  );
  if (!element) {
    throw new Error(`no button says ${name}`);
  }
  return element;
}

async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(async () => {
  answers = [];
  mount();
  ask({ fileName: "library.json", busy: false, failure: null });
  await settle();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the passphrase a sealed export asks for", () => {
  it("names the file it is about, and labels the field it asks in", () => {
    const said = dialog().textContent ?? "";

    expect(said).toContain("library.json");
    // No visible label — the sentence above the field is the heading — but the field still has a
    // name for anyone who cannot see that sentence and the placeholder together.
    expect(field().getAttribute("aria-label")).toBe("Passphrase");
  });

  it("hands back what was typed, once", async () => {
    const complained = field();
    complained.value = "correct horse";
    complained.dispatchEvent(new Event("input"));
    await settle();

    button("Import").click();
    await settle();

    expect(answers).toEqual(["correct horse"]);
  });

  it("answers nothing when the question is closed without one", async () => {
    button("Cancel").click();
    await settle();

    expect(answers).toEqual([null]);
  });

  it("says a refused passphrase in the field that was just filled in", async () => {
    ask({ fileName: "library.json", busy: false, failure: refusal() });
    await settle();

    expect(dialog().textContent).toContain("did not open this file");
    expect(field().value, "a rejected passphrase is not left there to be pressed again").toBe("");
  });

  it("sends nothing twice while an answer is on its way", async () => {
    ask({ fileName: "library.json", busy: true, failure: null });
    await settle();

    const complained = field();
    complained.value = "correct horse";
    complained.dispatchEvent(new Event("input"));
    await settle();
    button("Import").click();
    field().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await settle();

    expect(answers).toEqual([]);
    expect(button("Import").disabled).toBe(true);
  });
});

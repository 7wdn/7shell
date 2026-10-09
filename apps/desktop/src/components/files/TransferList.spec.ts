/**
 * The transfer list, as a person meets it: what each row offers, and what pressing it does.
 *
 * The rules about files — what a part file is, when a retry continues — live in `sevenshell-sftp`
 * and are tested against a real server there. What can only be seen here is the wiring: that the
 * button on a row is the one thing that row needs next, and that it reaches the call it names.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  transfersOpen: vi.fn(async () => []),
  transferCancel: vi.fn(async () => {}),
  transferPause: vi.fn(async () => {}),
  transferResume: vi.fn(async () => {}),
  transferRetry: vi.fn(async () => {}),
}));

import TransferList from "@/components/files/TransferList.vue";
import { i18n } from "@/i18n";
import { transferCancel, transferPause, transferResume, transferRetry } from "@/lib/backend/tauri";
import { useFilesStore } from "@/stores/files";
import type { TransferInfo, TransferState } from "@/types/session";

const cancelMock = vi.mocked(transferCancel);
const pauseMock = vi.mocked(transferPause);
const resumeMock = vi.mocked(transferResume);
const retryMock = vi.mocked(transferRetry);

let app: App | undefined;
let host: HTMLDivElement;

function transfer(id: string, state: TransferState, overrides: Partial<TransferInfo> = {}): TransferInfo {
  return {
    id,
    direction: "download",
    name: `${id}.txt`,
    remotePath: `/home/labuser/${id}.txt`,
    size: 1024,
    transferred: 512,
    state,
    packed: false,
    ...overrides,
  };
}

/**
 * Draws the list over rows the store is simply told about.
 *
 * The polling belongs to whichever frame owns the list, and what this file is about is what a row
 * offers — so the rows are put in the store rather than waited for from a server.
 */
async function render(...rows: TransferInfo[]): Promise<void> {
  const pinia = createPinia();
  setActivePinia(pinia);

  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(TransferList) })
    .use(pinia)
    .use(i18n);

  useFilesStore().transfers = [...rows];
  app.mount(host);
  await nextTick();
}

/** The one button on a row, by the label a person reads. */
function buttonFor(label: string): HTMLButtonElement {
  const button = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  if (!button) {
    throw new Error(`no button reading ${label}`);
  }
  return button;
}

function labels(): string[] {
  return [...host.querySelectorAll<HTMLButtonElement>("li button")].map((button) => button.textContent?.trim() ?? "");
}

beforeEach(() => {
  cancelMock.mockClear();
  pauseMock.mockClear();
  resumeMock.mockClear();
  retryMock.mockClear();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("a transfer's row", () => {
  it("offers what the row needs next, and nothing else", async () => {
    await render(
      transfer("t1", { kind: "running" }),
      transfer("t2", { kind: "paused" }),
      transfer("t3", { kind: "failed", detail: "the channel failed" }),
      transfer("t4", { kind: "done" }),
      transfer("t5", { kind: "cancelled" }),
    );

    expect(labels()).toEqual(["Hold", "Continue", "Try again", "Close", "Close"]);
  });

  it("holds a running transfer, and lets a paused one go", async () => {
    await render(transfer("t1", { kind: "running" }));
    buttonFor("Hold").click();
    await vi.waitFor(() => expect(pauseMock).toHaveBeenCalledWith("t1"));

    // What the row says after the backend has done it: the same transfer, waiting.
    useFilesStore().transfers = [transfer("t1", { kind: "paused" })];
    await nextTick();

    buttonFor("Continue").click();
    await vi.waitFor(() => expect(resumeMock).toHaveBeenCalledWith("t1"));
  });

  it("tries a stopped transfer again", async () => {
    await render(transfer("t3", { kind: "failed", detail: "the channel failed" }));

    // The reason it stopped is on the row rather than behind a hover: it is the thing somebody needs
    // to decide whether trying again is worth it.
    expect(host.textContent).toContain("the channel failed");

    buttonFor("Try again").click();
    await vi.waitFor(() => expect(retryMock).toHaveBeenCalledWith("t3"));
  });

  it("dismisses a finished one through the same call that stops a running one", async () => {
    await render(transfer("t4", { kind: "done" }));

    buttonFor("Close").click();

    await vi.waitFor(() => expect(cancelMock).toHaveBeenCalledWith("t4"));
  });
});

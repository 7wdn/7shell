/**
 * The keys page: making a key, importing one, and the public line that goes on a server.
 *
 * What the key *is* — its type, its fingerprint, whether the file it came from can be read — is the
 * ssh crate's business and is tested there, against real keys. This file is about the surface: which
 * question reaches the shell, what the page shows afterwards, and what a person can take away from
 * it. The one thing worth stating out loud, because it is the reason the page exists: a public line
 * crosses to the interface and no private one ever does.
 */
import { createPinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  keysList: vi.fn(),
  keyGenerate: vi.fn(),
  keyPublic: vi.fn(),
  keyImport: vi.fn(),
  keyForget: vi.fn(),
  sessionsList: vi.fn(),
}));

import KnownKeys from "@/components/settings/KnownKeys.vue";
import { i18n } from "@/i18n";
import { keyForget, keyGenerate, keyPublic, keysList, sessionsList } from "@/lib/backend/tauri";
import type { CreatedKey, KeyInfo } from "@/types/session";

const listMock = vi.mocked(keysList);
const generateMock = vi.mocked(keyGenerate);
const publicMock = vi.mocked(keyPublic);
const forgetMock = vi.mocked(keyForget);
const sessionsMock = vi.mocked(sessionsList);

let app: App | undefined;
let host: HTMLDivElement;

function key(name: string, keyRef: string): KeyInfo {
  return {
    keyRef,
    displayName: name,
    algorithm: "ssh-ed25519",
    fingerprint: `SHA256:${name}`,
    hasPassphrase: false,
  };
}

/** The public line of a key called `made`, and the receipt a generation answers with. */
const PUBLIC_LINE = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI made someone@example";

function created(): CreatedKey {
  return { info: key("id_ed25519", "key-made"), publicKey: PUBLIC_LINE };
}

async function mount(stored: readonly KeyInfo[] = [], then: readonly KeyInfo[] = stored): Promise<void> {
  listMock.mockReset();
  listMock.mockResolvedValueOnce([...stored]);
  listMock.mockResolvedValue([...then]);

  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(KnownKeys) })
    .use(createPinia())
    .use(i18n);
  app.mount(host);
  await vi.waitFor(() => {
    expect(listMock, "the page has asked what is stored").toHaveBeenCalled();
  });
  await nextTick();
}

async function press(label: string | RegExp, where: ParentNode = host): Promise<void> {
  const button = [...where.querySelectorAll("button")].find((candidate) => {
    const reading = candidate.textContent?.trim() ?? "";
    const named = candidate.getAttribute("aria-label") ?? "";
    return typeof label === "string" ? reading === label || named === label : label.test(reading) || label.test(named);
  });
  if (!button) {
    throw new Error(`no button reading ${String(label)}`);
  }
  button.click();
  await nextTick();
}

async function type(id: string, value: string): Promise<void> {
  const element = host.querySelector<HTMLInputElement>(`#${id}`);
  if (!element) {
    throw new Error(`the page has no field ${id}`);
  }
  element.value = value;
  element.dispatchEvent(new Event("input"));
  await nextTick();
}

/**
 * Opens the dropdown, which is the interface's own control: its list is teleported to the body and
 * reka-ui opens it on a pointer going down rather than on a click.
 */
async function openKinds(): Promise<void> {
  const trigger = host.querySelector<HTMLElement>('[role="combobox"]');
  if (!trigger) {
    throw new Error("the page has no dropdown");
  }
  trigger.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse" }));
  trigger.click();
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(() => {
  generateMock.mockReset();
  publicMock.mockReset().mockResolvedValue(PUBLIC_LINE);
  forgetMock.mockReset();
  sessionsMock.mockReset().mockResolvedValue([]);
  vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn(async () => {}) } });
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.body.querySelectorAll('[role="dialog"]')) {
    node.remove();
  }
});

describe("the keys page", () => {
  it("offers the three kinds of key this build can make", async () => {
    await mount();

    // The dropdown's rows, which are the three OpenSSH itself offers; the labels are what a person
    // reads and the values are what the request carries.
    await openKinds();
    const options = [...document.body.querySelectorAll<HTMLElement>('[role="option"]')].map((option) =>
      option.textContent?.trim(),
    );
    expect(options).toEqual(["Ed25519", "ECDSA (P-256)", "RSA 3072"]);
  });

  it("makes a key and shows the line that goes on a server", async () => {
    generateMock.mockResolvedValue(created());
    await mount([], [key("id_ed25519", "key-made")]);

    await press("Make a key");

    await vi.waitFor(() => expect(generateMock).toHaveBeenCalled());
    expect(generateMock.mock.calls[0]?.[0], "the kind the dropdown shows").toBe("ed25519");
    expect(generateMock.mock.calls[0]?.[2], "no passphrase typed, so the key has none").toBeNull();

    // The line is on screen without a second call: it came back with the key that was made, and
    // showing it is the whole point of making one here.
    await vi.waitFor(() => expect(host.textContent).toContain(PUBLIC_LINE));
    expect(publicMock, "nothing was fetched again").not.toHaveBeenCalled();
  });

  it("hands the passphrase that is on the panel to the key it makes", async () => {
    generateMock.mockResolvedValue(created());
    await mount([], [key("id_rsa", "key-made")]);

    await type("key-passphrase", "protect this");
    await press("Make a key");

    await vi.waitFor(() => expect(generateMock).toHaveBeenCalled());
    expect(generateMock.mock.calls[0]?.[2]).toBe("protect this");
  });

  it("shows the public line of a key that was imported earlier, and copies it", async () => {
    await mount([key("id_ed25519", "key-old")]);

    await press("Show the public key");

    await vi.waitFor(() => expect(publicMock).toHaveBeenCalledWith("key-old"));
    await vi.waitFor(() => expect(host.textContent).toContain(PUBLIC_LINE));

    await press("Copy");
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(PUBLIC_LINE);
    // The button says so, because a copy that failed silently looks exactly like one that worked
    // until somebody pastes.
    await vi.waitFor(() => expect(host.textContent).toContain("Copied"));
  });

  it("says what forgetting costs before the key goes", async () => {
    await mount([key("id_ed25519", "key-old")], []);

    await press("Forget this key");
    const asking = document.body.querySelector<HTMLElement>('[role="dialog"]');
    expect(asking?.textContent).toContain("id_ed25519");

    await press("Forget it", asking as HTMLElement);

    await vi.waitFor(() => expect(forgetMock).toHaveBeenCalledWith("key-old"));
    await vi.waitFor(() => expect(host.textContent).toContain("No key yet"));
  });
});

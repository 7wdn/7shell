/**
 * The policy matrix page, as the window draws it.
 *
 * What is asserted here is what the page would get wrong on its own: a rule that reached the
 * settings file while somebody was still typing, a matrix that was left out of a save, and a
 * restore that could not be told apart from the default the build ships.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  settingsGet: vi.fn(),
  settingsSet: vi.fn(),
  systemTheme: vi.fn(),
  onSystemThemeChanged: vi.fn(),
  windowThemeApply: vi.fn(),
}));

import PolicyMatrixPanel from "@/components/settings/PolicyMatrixPanel.vue";
import { i18n } from "@/i18n";
import { settingsSet } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { DEFAULT_POLICY_MATRIX, type PolicyMatrix } from "@/types/app";

const settingsSetMock = vi.mocked(settingsSet);

let app: App | undefined;
let host: HTMLDivElement;

async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(PolicyMatrixPanel) })
    .use(createPinia())
    .use(i18n);
  app.mount(host);
  await nextTick();
}

/** One turn of the event loop, which is what reka-ui's dismiss layer is registered on. */
async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function trigger(selector: string): HTMLElement {
  const element = host.querySelector<HTMLElement>(selector);
  if (!element) {
    throw new Error(`no control ${selector}`);
  }
  return element;
}

/** Opens one of the action dropdowns and takes the option whose label is given. */
async function choose(selector: string, label: string): Promise<void> {
  const element = trigger(selector);
  element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse" }));
  element.click();
  await settle();

  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((node) =>
    (node.textContent ?? "").trim().startsWith(label),
  );
  if (!option) {
    throw new Error(`the list has no option ${label}`);
  }
  option.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "mouse" }));
  await settle();
}

async function typeKey(row: HTMLElement, key: string): Promise<void> {
  const input = row.querySelector<HTMLInputElement>("[data-policy-key]");
  if (!input) {
    throw new Error("the row has no key field");
  }
  input.value = key;
  input.dispatchEvent(new Event("input"));
  await nextTick();
}

async function press(selector: string): Promise<void> {
  const button = host.querySelector<HTMLButtonElement>(selector);
  if (!button) {
    throw new Error(`no button ${selector}`);
  }
  button.click();
  await settle();
}

function globalRule(level: string): HTMLElement {
  return trigger(`[data-policy-layer="global"] [data-policy-rule="${level}"]`);
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  settingsSetMock.mockResolvedValue(undefined);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.querySelectorAll('[role="listbox"]')) {
    node.remove();
  }
});

describe("the global rule", () => {
  it("starts from the built-in default and says that the two destructive levels cannot be allowed away", async () => {
    await mount();

    // The rule the store starts from: safe and caution run, the two destructive levels ask.
    expect(globalRule("safe").textContent).toContain("Allow");
    expect(globalRule("caution").textContent).toContain("Allow");
    expect(globalRule("destructive").textContent).toContain("Ask");
    expect(globalRule("catastrophic").textContent).toContain("Ask");

    // The note the resolver's own normalisation is worth: stated once, for both levels.
    expect(host.querySelector("[data-policy-normalized]")?.textContent).toContain("raised to Ask");
    expect(host.querySelector("[data-policy-most-restrictive]")?.textContent).toContain("strictest action decides");
  });

  it("keeps a rule edit in the draft until Apply writes it", async () => {
    await mount();
    const preferences = usePreferencesStore();

    await choose('[data-policy-layer="global"] [data-policy-rule="destructive"]', "Deny");

    // The draft moved; neither the store nor the settings file did.
    expect(globalRule("destructive").textContent).toContain("Deny");
    expect(settingsSetMock).not.toHaveBeenCalled();
    expect(preferences.policyMatrix.global.destructive).toBe("ask");
    expect(preferences.dirty).toBe(false);

    await press("[data-policy-apply]");
    await vi.waitFor(() => expect(settingsSetMock).toHaveBeenCalledTimes(1));

    const payload = settingsSetMock.mock.calls[0]?.[0];
    expect(payload?.policyMatrix?.global.destructive).toBe("deny");
    expect(payload?.policyMatrix?.global.safe).toBe("allow");
    // The applied matrix is the store's now, and the page says it was written.
    expect(preferences.policyMatrix.global.destructive).toBe("deny");
    await vi.waitFor(() => expect(host.querySelector("[data-policy-notice]")).not.toBeNull());
  });
});

describe("the exception rows", () => {
  it("writes a named row into its layer and leaves unnamed rows out", async () => {
    await mount();

    await press('[data-policy-add="group"]');
    const row = trigger('[data-policy-layer="group"] [data-policy-row]');
    await typeKey(row, "prod");
    await choose('[data-policy-layer="group"] [data-policy-rule="destructive"]', "Allow");

    // A second row nobody has named is a row in progress, not an entry in the file.
    await press('[data-policy-add="group"]');

    await press("[data-policy-apply]");
    await vi.waitFor(() => expect(settingsSetMock).toHaveBeenCalledTimes(1));

    const payload = settingsSetMock.mock.calls[0]?.[0];
    expect(payload?.policyMatrix?.groups).toEqual({
      prod: { safe: "allow", caution: "allow", destructive: "allow", catastrophic: "ask" },
    });
    expect(payload?.policyMatrix?.hosts).toEqual({});
    expect(payload?.policyMatrix?.directories).toEqual({});
  });

  it("removes a row from the draft without touching the file", async () => {
    await mount();
    const preferences = usePreferencesStore();
    const withRow: PolicyMatrix = {
      ...DEFAULT_POLICY_MATRIX,
      groups: { prod: { safe: "deny", caution: "deny", destructive: "deny", catastrophic: "deny" } },
    };
    // The store's matrix is what the page reads, so a matrix that arrives after the page opened
    // still finds the draft: that is what a reload or another page's Apply looks like.
    preferences.setPolicyMatrix(withRow);
    await settle();

    expect(host.querySelectorAll('[data-policy-layer="group"] [data-policy-row]')).toHaveLength(1);

    await press('[data-policy-layer="group"] [data-policy-remove]');

    expect(host.querySelectorAll('[data-policy-layer="group"] [data-policy-row]')).toHaveLength(0);
    expect(settingsSetMock).not.toHaveBeenCalled();

    await press("[data-policy-apply]");
    await vi.waitFor(() => expect(settingsSetMock).toHaveBeenCalledTimes(1));
    expect(settingsSetMock.mock.calls[0]?.[0]?.policyMatrix?.groups).toEqual({});
  });
});

describe("restoring the default", () => {
  it("puts the draft back to the built-in rule, and Apply writes that rule", async () => {
    await mount();
    const preferences = usePreferencesStore();
    preferences.setPolicyMatrix({
      global: { safe: "deny", caution: "deny", destructive: "deny", catastrophic: "deny" },
      groups: { prod: { safe: "deny", caution: "deny", destructive: "deny", catastrophic: "deny" } },
      hosts: {},
      directories: {},
    });
    await settle();

    expect(globalRule("safe").textContent).toContain("Deny");

    await press("[data-policy-restore]");

    expect(globalRule("safe").textContent).toContain("Allow");
    expect(host.querySelectorAll('[data-policy-layer="group"] [data-policy-row]')).toHaveLength(0);
    // Still a draft: nothing was written by the restore itself.
    expect(settingsSetMock).not.toHaveBeenCalled();

    await press("[data-policy-apply]");
    await vi.waitFor(() => expect(settingsSetMock).toHaveBeenCalledTimes(1));
    expect(settingsSetMock.mock.calls[0]?.[0]?.policyMatrix).toEqual(DEFAULT_POLICY_MATRIX);
  });
});

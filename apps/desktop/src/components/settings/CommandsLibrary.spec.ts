/**
 * The command library on the settings page: the same list the panel under a terminal edits.
 *
 * The store's own rules are tested where they live. This file is about the surface — that the table
 * shows what is saved with the trigger and the line, that adding opens the form the store edits, and
 * that editing a row loads that command into it.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CommandsLibrary from "@/components/settings/CommandsLibrary.vue";
import { i18n } from "@/i18n";
import { useCommandsStore } from "@/stores/commands";
import type { Snippet } from "@/types/snippet";

let app: App | undefined;
let host: HTMLDivElement;

function snippet(name: string, prefix: string, command: string): Snippet {
  return { id: name, groupId: "default", name, prefix, command, appendCr: true, order: 0, slotHistory: [] };
}

async function mount(commands: readonly Snippet[] = []): Promise<void> {
  // The library is in the store before the page is mounted, which is what a load that has already
  // happened looks like; the page then opens the first group on its own.
  useCommandsStore().groups = [{ id: "default", name: "", order: 0, commands: [...commands] }];
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(CommandsLibrary) }).use(i18n);
  app.mount(host);
  await nextTick();
}

/** The rows are drawn once a group has been opened, which is a turn after the mount. */
async function rows(): Promise<void> {
  await vi.waitFor(() => {
    expect(host.querySelectorAll("[data-command-row]").length).toBeGreaterThan(0);
  });
}

function field(label: string): HTMLInputElement | null {
  return host.querySelector<HTMLInputElement>(`input[aria-label="${label}"], textarea[aria-label="${label}"]`);
}

beforeEach(() => {
  setActivePinia(createPinia());
  document.body.innerHTML = "";
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the command library page", () => {
  it("lists what is saved with the trigger and the line", async () => {
    await mount([snippet("重启 nginx", "res", "systemctl restart #1")]);
    await rows();

    expect(host.textContent).toContain("重启 nginx");
    expect(host.textContent).toContain("res");
    expect(host.textContent).toContain("systemctl restart #1");
  });

  it("says so when a command has no trigger, rather than showing an empty cell", async () => {
    await mount([snippet("开机时长", "", "uptime")]);
    await rows();

    expect(host.textContent).toContain("No trigger");
  });

  it("opens the form on Add, and the store is what the form edits", async () => {
    await mount();
    const add = [...host.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Add a command");
    add?.click();
    await nextTick();

    expect(useCommandsStore().draft).toMatchObject({ id: "", name: "", prefix: "", command: "" });
    expect(field("What it is called"), "the form is drawn from the draft").not.toBeNull();
  });

  it("loads a command into the form when its row is edited", async () => {
    await mount([snippet("重启 nginx", "res", "systemctl restart #1")]);
    await rows();

    const edit = host.querySelector<HTMLButtonElement>('button[aria-label="Edit: 重启 nginx"]');
    edit?.click();
    await nextTick();

    expect(useCommandsStore().draft).toMatchObject({
      id: "重启 nginx",
      name: "重启 nginx",
      prefix: "res",
      command: "systemctl restart #1",
    });
  });

  it("will not save a command without a name and a line", async () => {
    await mount();
    const add = [...host.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Add a command");
    add?.click();
    await nextTick();

    const save = (): HTMLButtonElement | undefined =>
      [...host.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Save");
    expect(save()?.disabled, "nothing is written yet").toBe(true);

    const name = field("What it is called");
    if (name) {
      name.value = "重启 nginx";
      name.dispatchEvent(new Event("input"));
    }
    await nextTick();
    expect(save()?.disabled, "a name with no line is not a command").toBe(true);

    const line = host.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="The line, with #1… #5 where values go"]',
    );
    if (line) {
      line.value = "systemctl restart #1";
      line.dispatchEvent(new Event("input"));
    }
    await nextTick();
    expect(save()?.disabled).toBe(false);
  });
});

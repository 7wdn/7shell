/**
 * The filter over this machine's OpenSSH aliases.
 *
 * A configuration with many hosts is the reason the field exists, so what is held to here is the
 * answer a keystroke gives: the list narrows by any of the three values a row shows, a filter that
 * matches nothing says so instead of showing an empty box, and a reopened picker starts whole.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ sshConfigHosts: vi.fn() }));
vi.mock("@/lib/backend/api", () => ({ backend: { sessions: { sshConfigHosts: mocks.sshConfigHosts } } }));

import SshConfigHostDialog from "@/components/sessions/SshConfigHostDialog.vue";
import { i18n } from "@/i18n";
import type { SshConfigHost } from "@/types/session";

const hosts: SshConfigHost[] = [
  { alias: "web", host: "10.0.0.5", user: "deploy", port: 22, unsupported: [], proxyJump: "bastion" },
  { alias: "db-1", host: "10.0.0.9", user: "postgres", port: 5432, unsupported: [] },
  { alias: "bastion", host: "edge.example.com", user: "ops", port: 22, unsupported: [] },
];

let app: App | undefined;
let host: HTMLDivElement;
let setOpen!: (open: boolean) => void;

function mount(): void {
  app = createApp({
    data: () => ({ open: false }),
    render() {
      return h(SshConfigHostDialog, {
        open: this.open,
        "onUpdate:open": (value: boolean) => {
          this.open = value;
        },
      });
    },
    mounted() {
      setOpen = (open) => {
        this.open = open;
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
    throw new Error("the picker was not drawn");
  }
  return element;
}

function filterField(): HTMLInputElement {
  const element = dialog().querySelector<HTMLInputElement>("[data-ssh-config-filter]");
  if (!element) {
    throw new Error("the picker has no filter field");
  }
  return element;
}

function rows(): HTMLElement[] {
  return [...dialog().querySelectorAll<HTMLElement>("[data-ssh-config-host]")];
}

async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

async function type(value: string): Promise<void> {
  const field = filterField();
  field.value = value;
  field.dispatchEvent(new Event("input"));
  await settle();
}

beforeEach(async () => {
  mocks.sshConfigHosts.mockReset().mockResolvedValue(hosts);
  mount();
  setOpen(true);
  await settle();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the OpenSSH host filter", () => {
  it("narrows the list by the alias, the address or the account", async () => {
    await type("db");
    expect(rows().map((row) => row.textContent)).toHaveLength(1);
    expect(rows()[0]?.textContent).toContain("db-1");

    await type("edge.example");
    expect(rows()).toHaveLength(1);
    expect(rows()[0]?.textContent).toContain("bastion");

    await type("postgres");
    expect(rows()).toHaveLength(1);
    expect(rows()[0]?.textContent).toContain("db-1");
  });

  it("shows the route a host from the file goes through", async () => {
    await type("web");
    expect(rows()[0]?.textContent).toContain("via bastion");
  });

  it("says that nothing matches rather than drawing an empty box", async () => {
    await type("nothing-answers");

    expect(rows()).toHaveLength(0);
    expect(dialog().textContent).toContain("No hosts match this filter.");
  });

  it("starts from the whole list when the picker is opened again", async () => {
    await type("db");
    expect(rows()).toHaveLength(1);

    setOpen(false);
    await settle();
    setOpen(true);
    await settle();

    expect(filterField().value).toBe("");
    expect(rows()).toHaveLength(hosts.length);
  });
});

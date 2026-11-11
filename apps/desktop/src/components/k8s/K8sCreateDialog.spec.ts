/**
 * Creating an object from a manifest.
 *
 * The dialog is text rather than a form, so what is worth pinning is the text it starts with and the
 * one case where it refuses to send: a namespaced kind has no collection to be posted to while the
 * list is showing every namespace, and saying that up front is better than the server's "namespace is
 * required" arriving after somebody wrote a manifest.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it } from "vitest";

import K8sCreateDialog from "@/components/k8s/K8sCreateDialog.vue";
import { customResourceDescriptor, type ResourceDescriptor } from "@/components/k8s/resources";
import { i18n } from "@/i18n";

const widgetCrd = {
  apiVersion: "apiextensions.k8s.io/v1",
  kind: "CustomResourceDefinition",
  metadata: { name: "widgets.example.com" },
  spec: {
    group: "example.com",
    names: { plural: "widgets", kind: "Widget" },
    scope: "Namespaced",
    versions: [{ name: "v1", served: true, storage: true, additionalPrinterColumns: [] }],
  },
};

let app: App | undefined;
let host: HTMLDivElement;
let confirmed: string[] = [];

function t(key: string, params?: Record<string, string>): string {
  return params ? i18n.global.t(key, params) : i18n.global.t(key);
}

function widget(): ResourceDescriptor {
  const found = customResourceDescriptor(widgetCrd);
  if (!found) {
    throw new Error("the Widget descriptor was not built from its definition");
  }
  return found;
}

function mount(namespace: string): void {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({
    render: () =>
      h(K8sCreateDialog, {
        open: true,
        descriptor: widget(),
        namespace,
        busy: false,
        failure: null,
        onConfirm: (manifest: string) => confirmed.push(manifest),
        "onUpdate:open": () => undefined,
      }),
  });
  app.use(i18n);
  app.mount(host);
}

/** The dialog is teleported to the body, so it is not inside the host element. */
function dialog(): HTMLElement {
  const found = document.querySelector<HTMLElement>('[role="dialog"]');
  if (!found) {
    throw new Error("the dialog was not drawn");
  }
  return found;
}

function editor(): HTMLTextAreaElement {
  const found = dialog().querySelector<HTMLTextAreaElement>("textarea");
  if (!found) {
    throw new Error("the dialog has no manifest editor");
  }
  return found;
}

function button(label: string): HTMLButtonElement {
  const found = [...dialog().querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  if (!found) {
    throw new Error(`no button says ${label}: ${dialog().textContent}`);
  }
  return found;
}

async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
}

afterEach(() => {
  confirmed = [];
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.querySelectorAll('[role="dialog"]')) {
    node.remove();
  }
});

describe("the manifest it starts with", () => {
  it("is the kind's own template, aimed at the namespace on screen, and is what gets sent", async () => {
    mount("team-a");
    await settle();

    expect(editor().value).toContain("apiVersion: example.com/v1");
    expect(editor().value).toContain("kind: Widget");
    expect(editor().value).toContain("namespace: team-a");

    const written = editor().value;
    button(t("k8s.create.submit", { kind: "Widget" })).click();
    await settle();

    expect(confirmed).toEqual([written]);
  });

  it("names the cluster's own kind rather than the group's label", async () => {
    mount("team-a");
    await settle();
    expect(dialog().textContent).toContain(t("k8s.create.title", { kind: "Widget" }));
    expect(dialog().textContent).not.toContain(t("k8s.resources.custom"));
  });
});

describe("a kind that belongs in a namespace", () => {
  it("will not submit while the list is showing every namespace, and says why", async () => {
    mount("");
    await settle();

    expect(button(t("k8s.create.submit", { kind: "Widget" })).disabled).toBe(true);
    expect(dialog().textContent).toContain(t("k8s.create.namespace"));
  });
});

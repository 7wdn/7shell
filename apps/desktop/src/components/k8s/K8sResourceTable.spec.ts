/**
 * The list toolbar, and the one control on it that is a promise about the kind.
 *
 * The filter, the count and the ordering are the table's visible work; what is pinned here is that
 * "New" is drawn from the descriptor's own decision rather than from the presence of starting text,
 * because a kind can hold a manifest worth opening without this build offering to post one.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it } from "vitest";

import K8sResourceTable from "@/components/k8s/K8sResourceTable.vue";
import type { K8sObject } from "@/components/k8s/objects";
import { descriptorOf, type ResourceDescriptor } from "@/components/k8s/resources";
import { i18n } from "@/i18n";

let app: App | undefined;
let host: HTMLDivElement;

function t(key: string): string {
  return i18n.global.t(key);
}

function mount(descriptor: ResourceDescriptor, rows: readonly K8sObject[] = []): void {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({
    render: () =>
      h(K8sResourceTable, {
        descriptor,
        rows,
        loading: false,
        failure: null,
        watchFailure: null,
        metricsAvailable: true,
        usageOf: () => null,
      }),
  });
  app.use(i18n);
  app.mount(host);
}

/** A row's action button, by the words it wears, or `null` where the row does not offer it. */
function actionButton(row: Element | undefined, label: string): HTMLButtonElement | null {
  if (!row) {
    return null;
  }
  return (
    [...row.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.getAttribute("aria-label") === label) ?? null
  );
}

/** The New button, by the words it wears, or `null` where the kind does not offer one. */
function createButton(): HTMLButtonElement | null {
  return (
    [...host.querySelectorAll<HTMLButtonElement>("button")].find(
      (button) => button.textContent?.trim() === t("k8s.list.create"),
    ) ?? null
  );
}

function descriptor(key: string): ResourceDescriptor {
  const found = descriptorOf(key);
  if (!found) {
    throw new Error(`the descriptor table lost ${key}`);
  }
  return found;
}

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the New button", () => {
  it("is drawn for a kind that offers to be created", async () => {
    mount(descriptor("pods"));
    await nextTick();
    expect(createButton()).not.toBeNull();
  });

  it("is absent for a kind whose descriptor holds a template but does not offer creation", async () => {
    // The template is still there: the button is the flag, and a flag that was derived from the
    // template's presence would draw this one.
    const pods = descriptor("pods");
    expect(pods.createTemplate).toBeDefined();
    mount({ ...pods, canCreate: false });
    await nextTick();
    expect(createButton()).toBeNull();
  });
});

describe("a row's actions", () => {
  /**
   * The gate is about the object rather than the kind: a pod's file browser needs a container it
   * can exec into, and a pending pod has none. If the table drew the kind's whole list, this would
   * show a Files button on the pending row that could only produce a refusal.
   */
  it("offers Files to a running pod and withholds it from a pending one", async () => {
    mount(descriptor("pods"), [
      { metadata: { name: "web-1", namespace: "team-a" }, status: { phase: "Running" } },
      { metadata: { name: "web-2", namespace: "team-a" }, status: { phase: "Pending" } },
    ]);
    await nextTick();

    const rows = [...host.querySelectorAll("tbody tr")];
    expect(rows).toHaveLength(2);
    expect(actionButton(rows[0], t("k8s.actions.files"))).not.toBeNull();
    expect(actionButton(rows[1], t("k8s.actions.files"))).toBeNull();
    // Everything the kind offers regardless of phase stays on both rows.
    expect(actionButton(rows[0], t("k8s.actions.logs"))).not.toBeNull();
    expect(actionButton(rows[1], t("k8s.actions.logs"))).not.toBeNull();
  });
});

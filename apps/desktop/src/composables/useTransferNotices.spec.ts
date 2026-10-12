/**
 * The policy half of the transfer notice: say it only when the panel that would have shown it is put
 * away, and consume it either way.
 *
 * The store is replaced with a small fake because the question here is not what the store does — that
 * is its own spec's job — but what this policy decides about what is on screen.
 */
import { createApp, defineComponent, h, nextTick, reactive } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

interface FailedTransfer {
  id: string;
  direction: "download" | "upload";
  name: string;
}

/**
 * `reactive` rather than `ref`s, because that is what a Pinia store hands back: the properties are
 * the values, not wrappers around them, and a watcher on one has to see the assignment.
 */
const files = reactive({
  unattendedFailures: [] as ReadonlyArray<FailedTransfer>,
  attendFailures: vi.fn<(ids: readonly string[]) => void>(),
  panelExpanded: false,
});

vi.mock("@/stores/files", () => ({ useFilesStore: () => files }));

import { useToast } from "@/composables/useToast";
import { useTransferNotices } from "@/composables/useTransferNotices";
import { i18n } from "@/i18n";

const { message, tier, action, visible, toast } = useToast();

const { attendFailures } = files;

const Host = defineComponent({
  setup() {
    useTransferNotices();
    return () => h("div");
  },
});

let unmount: (() => void) | undefined;

/** Mounts the policy the way the shell does: once, in a component's setup. */
function mount(): void {
  const app = createApp(Host);
  app.use(i18n);
  const root = document.createElement("div");
  document.body.append(root);
  app.mount(root);
  unmount = () => {
    app.unmount();
    root.remove();
  };
}

const DOWNLOAD_FAILED = { id: "x1", direction: "download", name: "notes.txt" } as const;

describe("what a failed transfer says", () => {
  beforeEach(() => {
    files.unattendedFailures = [];
    files.panelExpanded = false;
    attendFailures.mockReset();
    visible.value = false;
    action.value = undefined;
  });

  afterEach(() => {
    unmount?.();
    unmount = undefined;
    document.body.innerHTML = "";
  });

  it("names the file when the panel is tucked away", async () => {
    mount();

    files.unattendedFailures = [DOWNLOAD_FAILED];
    await nextTick();

    expect(message.value).toBe("Download failed: notes.txt");
    expect(tier.value).toBe("unseenFailure");
    expect(visible.value).toBe(true);
  });

  it("offers the panel as the way back, rather than sending anybody to a tab", async () => {
    mount();

    files.unattendedFailures = [DOWNLOAD_FAILED];
    await nextTick();

    expect(action.value?.label).toBe("Show transfers");
    action.value?.onClick();
    expect(files.panelExpanded).toBe(true);
  });

  it("says nothing when the failed row is already on screen, and still consumes it", async () => {
    mount();
    files.panelExpanded = true;

    files.unattendedFailures = [DOWNLOAD_FAILED];
    await nextTick();

    expect(visible.value).toBe(false);
    expect(attendFailures).toHaveBeenCalledWith([DOWNLOAD_FAILED.id]);
  });

  it("counts them when several arrive in the same read", async () => {
    mount();

    files.unattendedFailures = [DOWNLOAD_FAILED, { id: "x2", direction: "upload", name: "photo.jpg" }];
    await nextTick();

    expect(message.value).toBe("2 transfers failed");
  });

  it("stays quiet while nothing has failed", async () => {
    mount();

    await nextTick();
    toast("something else", "confirmation");
    await nextTick();

    expect(attendFailures).not.toHaveBeenCalled();
    expect(message.value).toBe("something else");
  });
});

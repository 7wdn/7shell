/**
 * The policy half of the transfer notice: say it only when the list that would have shown it is not
 * on screen, and consume it either way.
 *
 * The stores are replaced with small fakes because the question here is not what the stores do —
 * that is their own spec's job — but what this policy decides about what is on screen.
 */
import { createApp, defineComponent, h, nextTick, reactive } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

interface FailedTransfer {
  id: string;
  direction: "download" | "upload";
  name: string;
}

interface WorkspaceTab {
  id: string;
  kind: "terminal" | "files";
}

/**
 * `reactive` rather than `ref`s, because that is what a Pinia store hands back: the properties are
 * the values, not wrappers around them, and a watcher on one has to see the assignment.
 */
const files = reactive({
  unattendedFailures: [] as ReadonlyArray<FailedTransfer>,
  attendFailures: vi.fn<(ids: readonly string[]) => void>(),
});

const terminals = reactive({
  tabs: [] as ReadonlyArray<WorkspaceTab>,
  activeTab: null as WorkspaceTab | null,
  activate: vi.fn<(id: string) => void>(),
});

vi.mock("@/stores/files", () => ({ useFilesStore: () => files }));
vi.mock("@/stores/terminals", () => ({ useTerminalsStore: () => terminals }));

import { useToast } from "@/composables/useToast";
import { useTransferNotices } from "@/composables/useTransferNotices";
import { i18n } from "@/i18n";

const { message, tier, action, visible, toast } = useToast();

const { attendFailures } = files;
const { activate } = terminals;

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

const FILE_TAB = { id: "tab-files", kind: "files" } as const;
const TERMINAL_TAB = { id: "tab-terminal", kind: "terminal" } as const;

const DOWNLOAD_FAILED = { id: "x1", direction: "download", name: "notes.txt" } as const;

describe("what a failed transfer says", () => {
  beforeEach(() => {
    files.unattendedFailures = [];
    attendFailures.mockReset();
    terminals.tabs = [];
    terminals.activeTab = null;
    activate.mockReset();
    visible.value = false;
    action.value = undefined;
  });

  afterEach(() => {
    unmount?.();
    unmount = undefined;
    document.body.innerHTML = "";
  });

  it("names the file when the list is not the thing on screen", async () => {
    mount();
    terminals.tabs = [FILE_TAB];
    terminals.activeTab = TERMINAL_TAB;

    files.unattendedFailures = [DOWNLOAD_FAILED];
    await nextTick();

    expect(message.value).toBe("Download failed: notes.txt");
    expect(tier.value).toBe("unseenFailure");
    expect(visible.value).toBe(true);
  });

  it("offers the way back to the list, since every browser shows the same one", async () => {
    mount();
    terminals.tabs = [TERMINAL_TAB, FILE_TAB];
    terminals.activeTab = TERMINAL_TAB;

    files.unattendedFailures = [DOWNLOAD_FAILED];
    await nextTick();

    expect(action.value?.label).toBe("Show transfers");
    action.value?.onClick();
    expect(activate).toHaveBeenCalledWith(FILE_TAB.id);
  });

  it("says nothing when the failed row is already on screen, and still consumes it", async () => {
    mount();
    terminals.tabs = [FILE_TAB];
    terminals.activeTab = FILE_TAB;

    files.unattendedFailures = [DOWNLOAD_FAILED];
    await nextTick();

    expect(visible.value).toBe(false);
    expect(attendFailures).toHaveBeenCalledWith([DOWNLOAD_FAILED.id]);
  });

  it("counts them when several arrive in the same read", async () => {
    mount();
    terminals.tabs = [FILE_TAB];
    terminals.activeTab = TERMINAL_TAB;

    files.unattendedFailures = [DOWNLOAD_FAILED, { id: "x2", direction: "upload", name: "photo.jpg" }];
    await nextTick();

    expect(message.value).toBe("2 transfers failed");
  });

  it("offers no action when there is no browser left to open", async () => {
    mount();
    terminals.tabs = [TERMINAL_TAB];
    terminals.activeTab = TERMINAL_TAB;

    files.unattendedFailures = [DOWNLOAD_FAILED];
    await nextTick();

    expect(visible.value).toBe(true);
    expect(action.value).toBeUndefined();
  });

  it("stays quiet while nothing has failed", async () => {
    mount();
    terminals.activeTab = TERMINAL_TAB;

    await nextTick();
    toast("something else", "confirmation");
    await nextTick();

    expect(attendFailures).not.toHaveBeenCalled();
    expect(message.value).toBe("something else");
  });
});

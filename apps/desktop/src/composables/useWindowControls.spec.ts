import { createApp, defineComponent, h } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  windowMinimize: vi.fn(),
  windowToggleMaximize: vi.fn(),
  windowIsMaximized: vi.fn(),
  windowClose: vi.fn(),
  windowIsAlwaysOnTop: vi.fn(),
  windowSetAlwaysOnTop: vi.fn(),
  onWindowResized: vi.fn(),
}));

import { useWindowControls, type WindowControls } from "@/composables/useWindowControls";
import {
  onWindowResized,
  windowClose,
  windowIsAlwaysOnTop,
  windowIsMaximized,
  windowMinimize,
  windowSetAlwaysOnTop,
  windowToggleMaximize,
} from "@/lib/backend/tauri";

const minimizeMock = vi.mocked(windowMinimize);
const toggleMaximizeMock = vi.mocked(windowToggleMaximize);
const closeMock = vi.mocked(windowClose);
const isMaximizedMock = vi.mocked(windowIsMaximized);
const onResizedMock = vi.mocked(onWindowResized);
const isAlwaysOnTopMock = vi.mocked(windowIsAlwaysOnTop);
const setAlwaysOnTopMock = vi.mocked(windowSetAlwaysOnTop);

/** Mounts a throwaway component so the composable's lifecycle hooks actually run. */
function mount(platform: string): { controls: WindowControls; unmount: () => void } {
  let controls!: WindowControls;
  const app = createApp(
    defineComponent({
      setup() {
        controls = useWindowControls(() => platform);
        return () => h("div");
      },
    }),
  );
  app.mount(document.createElement("div"));
  return { controls, unmount: () => app.unmount() };
}

/** Lets the mounted hook's awaited promises settle. */
const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

describe("useWindowControls", () => {
  beforeEach(() => {
    minimizeMock.mockReset().mockResolvedValue(undefined);
    toggleMaximizeMock.mockReset().mockResolvedValue(undefined);
    closeMock.mockReset().mockResolvedValue(undefined);
    isMaximizedMock.mockReset().mockResolvedValue(false);
    onResizedMock.mockReset().mockResolvedValue(() => {});
    isAlwaysOnTopMock.mockReset().mockResolvedValue(false);
    setAlwaysOnTopMock.mockReset().mockResolvedValue(undefined);
  });

  it("draws its own buttons on Windows and Linux", () => {
    expect(mount("windows").controls.showControls.value).toBe(true);
    expect(mount("linux").controls.showControls.value).toBe(true);
  });

  it("leaves the traffic lights to macOS", () => {
    const { controls } = mount("macos");

    expect(controls.isMac.value).toBe(true);
    expect(controls.showControls.value).toBe(false);
  });

  it("drives the window through the transport", async () => {
    const { controls } = mount("windows");
    await settle();

    await controls.minimize();
    await controls.toggleMaximize();
    await controls.close();

    expect(minimizeMock).toHaveBeenCalledTimes(1);
    expect(toggleMaximizeMock).toHaveBeenCalledTimes(1);
    expect(closeMock).toHaveBeenCalledTimes(1);
  });

  it("knows whether the window is maximized, and keeps knowing", async () => {
    const { controls } = mount("windows");
    await settle();
    expect(controls.isMaximized.value).toBe(false);

    // The window reports a resize; the state is re-read rather than guessed.
    isMaximizedMock.mockResolvedValue(true);
    const listener = onResizedMock.mock.calls[0]?.[0];
    expect(listener).toBeDefined();
    listener?.();
    await settle();

    expect(controls.isMaximized.value).toBe(true);
  });

  it("stops listening when the interface goes away", async () => {
    const unsubscribe = vi.fn();
    onResizedMock.mockResolvedValue(unsubscribe);
    const { unmount } = mount("windows");
    await settle();

    unmount();

    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });

  it("does not ask for a window that is not there", async () => {
    const { isDesktopRuntime } = await import("@/lib/runtime");
    vi.mocked(isDesktopRuntime).mockReturnValue(false);

    const { controls } = mount("windows");

    expect(controls.isDesktop).toBe(false);
    expect(controls.showControls.value).toBe(false);
    expect(onResizedMock).not.toHaveBeenCalled();

    vi.mocked(isDesktopRuntime).mockReturnValue(true);
  });
});

/**
 * Keeping the window on top, which is the window's own state.
 *
 * What this file is for is the half a component cannot see: the toggle *asks* the window for the
 * opposite of what it last knew, and then draws what the window says — not what it asked for. A
 * window that cannot be pinned (a platform refusing, a permission missing) would otherwise be drawn
 * as pinned, and the second press would ask to unpin something that was never pinned.
 */
describe("keeping the window on top", () => {
  it("reads the window's own state when it mounts", async () => {
    isAlwaysOnTopMock.mockResolvedValue(true);
    const { controls } = mount("windows");
    await settle();

    expect(controls.isAlwaysOnTop.value, "a window already pinned is drawn pinned").toBe(true);
  });

  it("asks for the opposite of what the window last said", async () => {
    isAlwaysOnTopMock.mockResolvedValueOnce(false);
    const { controls } = mount("windows");
    await settle();

    // What the window will answer after the press: this one pinned, and then not.
    isAlwaysOnTopMock.mockResolvedValue(true);
    await controls.toggleAlwaysOnTop();
    expect(setAlwaysOnTopMock).toHaveBeenLastCalledWith(true);
    expect(controls.isAlwaysOnTop.value).toBe(true);

    isAlwaysOnTopMock.mockResolvedValue(false);
    await controls.toggleAlwaysOnTop();
    expect(setAlwaysOnTopMock).toHaveBeenLastCalledWith(false);
    expect(controls.isAlwaysOnTop.value).toBe(false);
  });

  it("draws what the window says rather than what was asked for", async () => {
    const { controls } = mount("windows");
    await settle();

    // A window that refuses: the press asked to pin it, and it says it is still not pinned.
    setAlwaysOnTopMock.mockResolvedValue(undefined);
    isAlwaysOnTopMock.mockResolvedValue(false);
    await controls.toggleAlwaysOnTop();

    expect(controls.isAlwaysOnTop.value, "the button follows the window, not the click").toBe(false);
  });
});

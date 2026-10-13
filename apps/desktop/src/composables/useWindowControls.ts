/**
 * The window controls the interface draws for itself.
 *
 * Windows and Linux run undecorated, so this composable supplies what the operating system
 * would otherwise provide: the three buttons, and the state the maximize button needs to know
 * whether it offers "maximize" or "restore". macOS keeps its native traffic lights, so there
 * the controls are hidden and only the inset is needed.
 *
 * Dragging and double-click-to-maximize are not here: the `data-tauri-drag-region` attribute
 * handles both, including the platform differences around double-click.
 */

import { computed, onBeforeUnmount, onMounted, ref, type ComputedRef, type Ref } from "vue";

import { backend } from "@/lib/backend/api";
import { isDesktopRuntime } from "@/lib/runtime";

export interface WindowControls {
  /** True when running inside the desktop shell rather than a browser. */
  readonly isDesktop: boolean;
  /** True on macOS, where the traffic lights are the system's and must not be duplicated. */
  readonly isMac: ComputedRef<boolean>;
  /** True when the interface should draw its own buttons. */
  readonly showControls: ComputedRef<boolean>;
  /** True when the window is currently maximized. */
  readonly isMaximized: Ref<boolean>;
  /** True while the window has been told to stay above every other one. */
  readonly isAlwaysOnTop: Ref<boolean>;
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  toggleAlwaysOnTop(): Promise<void>;
  close(): Promise<void>;
}

/** The OS name reported by the backend, such as `windows` or `macos`. */
export type PlatformName = () => string;

export function useWindowControls(platform: PlatformName): WindowControls {
  const isDesktop = isDesktopRuntime();
  const isMac = computed(() => platform().toLowerCase() === "macos");
  const showControls = computed(() => isDesktop && !isMac.value);
  const isMaximized = ref(false);
  /**
   * Whether the window stays above the others.
   *
   * The window's own state rather than a setting somebody chose: a window that came back pinned after
   * a restart is a window somebody has to unpin before they can see what is behind it, and nobody
   * asked for that. What the button offers is for as long as the window is open.
   */
  const isAlwaysOnTop = ref(false);

  let unsubscribe: (() => void) | undefined;

  async function refreshMaximized(): Promise<void> {
    try {
      isMaximized.value = await backend.window.isMaximized();
    } catch (error) {
      // The window may be closing, or a browser may be answering: leaving the glyph as
      // "maximize" is the harmless state either way.
      console.warn("the maximized state could not be read", error);
    }
  }

  async function refreshAlwaysOnTop(): Promise<void> {
    try {
      isAlwaysOnTop.value = await backend.window.isAlwaysOnTop();
    } catch (error) {
      // Same as the glyph above: a window that cannot be asked is one drawn unpinned, which is what
      // it was anyway until somebody pinned it.
      console.warn("the always-on-top state could not be read", error);
    }
  }

  /** Flips it, and draws what the window says afterwards rather than what was asked for. */
  async function toggleAlwaysOnTop(): Promise<void> {
    try {
      await backend.window.setAlwaysOnTop(!isAlwaysOnTop.value);
      await refreshAlwaysOnTop();
    } catch (error) {
      console.warn("always on top failed", error);
    }
  }

  async function run(operation: string, action: () => Promise<void>): Promise<void> {
    try {
      await action();
      await refreshMaximized();
    } catch (error) {
      console.warn(`${operation} failed`, error);
    }
  }

  onMounted(async () => {
    if (!isDesktop) {
      return;
    }
    unsubscribe = await backend.window.onResized(() => void refreshMaximized());
    await Promise.all([refreshMaximized(), refreshAlwaysOnTop()]);
  });

  onBeforeUnmount(() => {
    unsubscribe?.();
  });

  return {
    isDesktop,
    isMac,
    showControls,
    isMaximized,
    isAlwaysOnTop,
    minimize: () => run("minimize", () => backend.window.minimize()),
    toggleMaximize: () => run("toggle maximize", () => backend.window.toggleMaximize()),
    toggleAlwaysOnTop,
    close: () => run("close", () => backend.window.close()),
  };
}

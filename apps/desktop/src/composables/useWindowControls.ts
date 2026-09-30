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
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  close(): Promise<void>;
}

/** The OS name reported by the backend, such as `windows` or `macos`. */
export type PlatformName = () => string;

export function useWindowControls(platform: PlatformName): WindowControls {
  const isDesktop = isDesktopRuntime();
  const isMac = computed(() => platform().toLowerCase() === "macos");
  const showControls = computed(() => isDesktop && !isMac.value);
  const isMaximized = ref(false);

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
    await refreshMaximized();
  });

  onBeforeUnmount(() => {
    unsubscribe?.();
  });

  return {
    isDesktop,
    isMac,
    showControls,
    isMaximized,
    minimize: () => run("minimize", () => backend.window.minimize()),
    toggleMaximize: () => run("toggle maximize", () => backend.window.toggleMaximize()),
    close: () => run("close", () => backend.window.close()),
  };
}

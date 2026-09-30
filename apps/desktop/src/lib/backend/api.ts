/**
 * The only backend entry point feature code may use.
 *
 * Every call goes through {@link forward}, which answers a call made without a backend with a
 * real envelope instead of an undefined-function error, and normalizes anything the other side
 * rejects with. When a second transport arrives it is selected inside this module; callers
 * never learn which one answered.
 */

import { isDesktopRuntime } from "@/lib/runtime";
import type { AppInfo, AppSettings } from "@/types/app";

import { ShellError, normalizeShellError, noTransportError } from "./errors";
import * as desktop from "./tauri";

async function forward<T>(operation: string, run: () => Promise<T>): Promise<T> {
  if (!isDesktopRuntime()) {
    throw noTransportError(operation);
  }
  try {
    return await run();
  } catch (error) {
    throw normalizeShellError(error, operation);
  }
}

export const backend = {
  /** Name, version, platform and descriptor counts of the running application. */
  appInfo: (): Promise<AppInfo> => forward("app_info", () => desktop.appInfo()),

  /** Language and theme: the only things kept between runs so far, and never secrets. */
  settings: {
    get: (): Promise<AppSettings> => forward("settings_get", () => desktop.settingsGet()),
    set: (settings: AppSettings): Promise<void> => forward("settings_set", () => desktop.settingsSet(settings)),
  },

  /** The window the interface is drawn in. Meaningless without the desktop shell. */
  window: {
    minimize: (): Promise<void> => forward("window.minimize", () => desktop.windowMinimize()),
    toggleMaximize: (): Promise<void> => forward("window.toggleMaximize", () => desktop.windowToggleMaximize()),
    isMaximized: (): Promise<boolean> => forward("window.isMaximized", () => desktop.windowIsMaximized()),
    close: (): Promise<void> => forward("window.close", () => desktop.windowClose()),

    /**
     * Subscribes to window resizes so the maximize button can show the right glyph.
     * In a browser there is no window to watch, so there is nothing to unsubscribe from.
     */
    onResized: (listener: () => void): Promise<() => void> =>
      isDesktopRuntime() ? desktop.onWindowResized(listener) : Promise.resolve(() => {}),
  },
};

export { ShellError };

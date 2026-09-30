/**
 * The only backend entry point feature code may use.
 *
 * Every call goes through {@link forward}, which answers a call made without a backend
 * with a real envelope instead of an undefined-function error, and normalizes anything
 * the other side rejects with. When a second transport arrives it is selected inside
 * this module; callers never learn which one answered.
 */

import { isDesktopRuntime } from "@/lib/runtime";

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
  appInfo: (): Promise<import("@/types/app").AppInfo> => forward("app_info", () => desktop.appInfo()),
};

export { ShellError };

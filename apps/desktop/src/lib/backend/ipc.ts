/**
 * The lowest transport layer: the desktop shell's Tauri IPC or the browser service's HTTP and
 * WebSocket. `tauri.ts` is written against this module, so the command wrappers and the facade
 * above them are the same code in both hosts.
 *
 * The only place that may import `@tauri-apps/*` is this file. The browser branch never calls the
 * Tauri implementations, and the window/webview shims below answer the handful of calls the
 * wrappers make for things a browser tab does not have.
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { getCurrentWebview as tauriGetCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow as tauriGetCurrentWindow } from "@tauri-apps/api/window";

import { isDesktopRuntime } from "@/lib/runtime";
import { webInvoke, webListen } from "./web";

const desktop = isDesktopRuntime();

const browserWindow = {
  minimize: async () => {},
  toggleMaximize: async () => {},
  isMaximized: async () => false,
  close: async () => {},
  isAlwaysOnTop: async () => false,
  setAlwaysOnTop: async (_alwaysOnTop: boolean) => {},
  onResized: async (_handler: () => void) => () => {},
};

const browserWebview = {
  setZoom: async (_scale: number) => {},
};

export const invoke: typeof tauriInvoke = desktop ? tauriInvoke : (webInvoke as typeof tauriInvoke);

export const listen: typeof tauriListen = desktop ? tauriListen : (webListen as typeof tauriListen);

export const getCurrentWindow: typeof tauriGetCurrentWindow = desktop
  ? tauriGetCurrentWindow
  : () => browserWindow as unknown as ReturnType<typeof tauriGetCurrentWindow>;

export const getCurrentWebview: typeof tauriGetCurrentWebview = desktop
  ? tauriGetCurrentWebview
  : () => browserWebview as unknown as ReturnType<typeof tauriGetCurrentWebview>;

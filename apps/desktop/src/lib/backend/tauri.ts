/**
 * Typed wrappers over the Rust command surface and the window the interface is drawn in:
 * one function per `#[tauri::command]`, plus the window calls the title bar needs.
 *
 * Nothing outside this folder imports from `@tauri-apps/*` —
 * `scripts/tests/frontendBoundary.test.mjs` enforces that, because a second transport (a local
 * HTTP backend, for instance) must be addable without hunting the whole code base for
 * transport-specific calls.
 */

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { AppInfo, AppSettings } from "@/types/app";

// -- Application -------------------------------------------------------------

export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

// -- Settings ----------------------------------------------------------------

/** Reads the non-secret preferences. A missing file yields the defaults. */
export function settingsGet(): Promise<AppSettings> {
  return invoke<AppSettings>("settings_get");
}

/** Writes them back, and applies what the window itself needs. */
export function settingsSet(settings: AppSettings): Promise<void> {
  return invoke<void>("settings_set", { settings });
}

// -- Window ------------------------------------------------------------------
//
// The window is undecorated on Windows and Linux, so these are the buttons the interface draws
// itself. Dragging and double-click-to-maximize are handled by the `data-tauri-drag-region`
// attribute, not from here.

export function windowMinimize(): Promise<void> {
  return getCurrentWindow().minimize();
}

export function windowToggleMaximize(): Promise<void> {
  return getCurrentWindow().toggleMaximize();
}

export function windowIsMaximized(): Promise<boolean> {
  return getCurrentWindow().isMaximized();
}

export function windowClose(): Promise<void> {
  return getCurrentWindow().close();
}

/** Calls `listener` whenever the window is resized, and resolves with the unsubscribe. */
export async function onWindowResized(listener: () => void): Promise<() => void> {
  return getCurrentWindow().onResized(() => listener());
}

/**
 * Typed wrappers over the Rust command surface: one function per `#[tauri::command]`.
 *
 * Nothing outside this folder imports from `@tauri-apps/api` — `scripts/tests/frontendBoundary.test.mjs`
 * enforces that, because a second transport (a local HTTP backend, for instance) must be
 * addable without hunting the whole code base for transport-specific calls.
 */

import { invoke } from "@tauri-apps/api/core";

import type { AppInfo } from "@/types/app";

export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

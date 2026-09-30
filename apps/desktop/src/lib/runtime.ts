declare global {
  interface Window {
    /** Injected by the desktop shell. Absent when the interface runs in a browser. */
    __TAURI_INTERNALS__?: unknown;
  }
}

/** True when the interface runs inside the desktop shell rather than a plain browser. */
export function isDesktopRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

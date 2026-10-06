/**
 * Hand-written mirror of the Rust wire types.
 *
 * `packages/app-tests/contracts.test.ts` compares every field here with its Rust counterpart,
 * in both directions, so a rename on one side fails the test suite rather than a release.
 */

/** Facts about the running application, from `sevenshell_types::AppInfo`. */
export interface AppInfo {
  readonly name: string;
  readonly version: string;
  readonly os: string;
  readonly arch: string;
  readonly debugBuild: boolean;
  readonly protocolCount: number;
  readonly descriptorSchemaVersion: number;
}

/** How the interface picks its colours, from `sevenshell_types::ThemeMode`. */
export type ThemeMode = "system" | "dark" | "light";

/** What the colours resolve to once "system" has been answered. */
export type ResolvedTheme = "dark" | "light";

/** How the file browser is laid out, from `sevenshell_types::FilesLayout`. */
export type FilesLayout = "tree" | "list";

/** How a session opens, from `sevenshell_types::SessionLayout`. */
export type SessionLayout = "terminal" | "split";

/** Non-secret preferences, from `sevenshell_types::AppSettings`. */
export interface AppSettings {
  readonly themeMode: ThemeMode;
  /** `system` means "follow the interface theme" for the terminal, not the operating system. */
  readonly terminalTheme: ThemeMode;
  readonly filesLayout: FilesLayout;
  /** Whether opening a session opens a terminal on its own, or one with its file panel. */
  readonly sessionLayout: SessionLayout;
  /** How tall the file panel under a terminal is, in pixels. Absent means nobody has moved it. */
  readonly terminalPanelHeight?: number;
  /** How wide the session sidebar is, in pixels. Absent means nobody has moved the divider. */
  readonly sidebarWidth?: number;
  /** Whether the session sidebar is on screen. On unless somebody puts it away. */
  readonly sidebarOpen: boolean;
  /**
   * Whether the host monitor runs beside the workspace. On unless somebody turns it off: hidden
   * means nothing is sampled either, not merely that nothing is drawn.
   */
  readonly monitorEnabled: boolean;
  /**
   * Whether a directory travels as one archive instead of file by file. Off unless somebody turns
   * it on: this changes what a transfer leaves on disk, so it is never decided for them.
   */
  readonly packedTransfer: boolean;
  /** Absent means nobody has chosen a language, so the system's applies. */
  readonly locale?: string;
}

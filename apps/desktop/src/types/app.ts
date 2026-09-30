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

/** Non-secret preferences, from `sevenshell_types::AppSettings`. */
export interface AppSettings {
  readonly themeMode: ThemeMode;
  /** Absent means nobody has chosen a language, so the system's applies. */
  readonly locale?: string;
}

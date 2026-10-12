/**
 * The update channel, as the interface reads it off the wire.
 *
 * Mirrors of the types in `crates/sevenshell-types/src/update.rs`, and `packages/app-tests` compares
 * the two: a field renamed on one side only is a page that draws nothing where the answer was.
 *
 * Nothing here carries a download address or a signature. The interface is told which version is
 * published and what it changed; the bytes and their verification stay in the shell, which is also
 * what keeps a person's decision — "install that one" — about the release they were shown.
 */

/** A published release, as the channel describes it. */
export interface AvailableRelease {
  readonly version: string;
  /** The release notes as published, plain text. Absent when the channel described none. */
  readonly notes?: string;
  /** When it was published, in seconds since the Unix epoch. */
  readonly publishedAt?: number;
}

/** What a look at the update channel found. */
export type UpdateCheck =
  | { readonly kind: "upToDate"; readonly currentVersion: string }
  | { readonly kind: "available"; readonly currentVersion: string; readonly release: AvailableRelease }
  /**
   * This build is not an installed application — a development run — so there is nothing to update.
   * Not a failure: the question is the one that does not apply, and it is answered as such.
   */
  | { readonly kind: "notInstalled"; readonly currentVersion: string };

/** How far an installation has got, as it runs. */
export type UpdateProgress =
  | { readonly kind: "downloading"; readonly downloaded: number; readonly total?: number }
  | { readonly kind: "installing" };

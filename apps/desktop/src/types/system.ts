/**
 * Hand-written mirror of `sevenshell_types::system`.
 *
 * `packages/app-tests/contracts.test.ts` compares every field here with its Rust counterpart, so a
 * rename on one side fails the test suite rather than a release.
 *
 * Every field is nullable for the same reason it is on the sample types next door: the answers come
 * from commands run on the far side, and "this host did not say" has to stay distinguishable from a
 * name the row could draw. What the interface does with the answer — which mark, which tooltip, or
 * nothing at all — is decided here; the other side of the wire has no opinion about it.
 */

/** What one host said it is. */
export interface HostSystem {
  /** The name a person reads: the distribution's own, the kernel's, or what `ver` answered. */
  readonly name: string | null;
  /** The kernel's own name — the answer to `uname -s`: `Linux`, `Darwin`, `MINGW64_NT-10.0`. */
  readonly kernel: string | null;
  /** What the host calls its system, in lower case: `ID=` from `/etc/os-release`, or `windows`. */
  readonly distro: string | null;
  /** What that system says it is like (`ID_LIKE=`), in the order the host published it. */
  readonly distroLike: readonly string[];
}

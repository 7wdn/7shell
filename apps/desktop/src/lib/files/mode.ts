/**
 * The permission part of an SFTP mode, in the shape a person types it.
 *
 * `SftpEntry.mode` also carries the file's type bits, and those are not what `chmod` takes: a
 * directory's `0o40755` is `0755` to the person changing it, and sending the type bits back would be
 * a request to change the kind of thing at that path.
 */

/** The permission bits as octal text, with the leading zero and at least three digits (`0644`). */
export function octalMode(mode: number | undefined): string {
  return ((mode ?? 0) & 0o7777).toString(8).padStart(4, "0");
}

/** A typed octal mode, or null when it is not permission bits this build will send. */
export function parseOctalMode(text: string): number | null {
  const trimmed = text.trim();
  if (!/^[0-7]{1,4}$/.test(trimmed)) {
    return null;
  }
  const value = Number.parseInt(trimmed, 8);
  return value <= 0o7777 ? value : null;
}

/** The three rows of the permission matrix, by how far their bits are shifted. */
export const PERMISSION_ROLES = [
  { shift: 6, key: "owner" },
  { shift: 3, key: "group" },
  { shift: 0, key: "others" },
] as const;

/** The three columns: read, write, execute. */
export const PERMISSION_COLUMNS = [
  { bit: 4, key: "read" },
  { bit: 2, key: "write" },
  { bit: 1, key: "execute" },
] as const;

/** Whether the typed mode has one bit set. An invalid draft is read as no bits, like the input shows. */
export function hasPermission(mode: string, shift: number, bit: number): boolean {
  const value = parseOctalMode(mode);
  return value !== null && (value & (bit << shift)) !== 0;
}

/**
 * Sets or clears one bit of the typed mode.
 *
 * The setuid/setgid/sticky bits are carried through untouched: the matrix is the nine ordinary
 * bits, and a round trip through it must not silently drop the other three.
 */
export function togglePermission(mode: string, shift: number, bit: number, on: boolean): string {
  const value = parseOctalMode(mode) ?? 0;
  const next = on ? value | (bit << shift) : value & ~(bit << shift);
  return octalMode(next);
}

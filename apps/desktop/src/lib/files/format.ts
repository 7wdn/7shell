/**
 * Remote paths and numbers, as a person reads them.
 *
 * The interface formats; the backend decides. A size of zero is still a size, a missing timestamp
 * is an absence rather than the epoch, and a path segment is joined the way the SFTP side joins it
 * — the one place the interface builds a remote path itself is naming a new or renamed entry, and
 * a doubled separator there would ask the server for a different file than the one intended.
 */

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

/** Bytes the way a file manager writes them: decimal units, one decimal below ten. */
export function formatSize(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const digits = unit > 0 && value < 10 ? 1 : 0;
  return `${value.toFixed(digits)} ${UNITS[unit] ?? "B"}`;
}

/** Seconds since the epoch, in the machine's own date and time format. */
export function formatModified(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString();
}

/**
 * The permission bits the way `ls -l` writes them, for example `rwxr-xr-x`.
 *
 * The setuid, setgid and sticky bits are written where `ls` writes them — `rws` rather than `rwx`
 * — because a column that dropped them would make a setuid binary and an ordinary one look the
 * same, and telling those apart is what somebody reads a permission column for.
 */
export function formatMode(mode: number | undefined): string {
  if (mode === undefined) {
    return "—";
  }

  const letters = ["r", "w", "x"];
  const written: string[] = [];
  for (let place = 8; place >= 0; place -= 1) {
    written.push(mode & (1 << place) ? (letters[2 - (place % 3)] ?? "-") : "-");
  }

  // The three bits above the nine are the execute letter of their own group, said differently.
  if (mode & 0o4000) {
    written[2] = written[2] === "x" ? "s" : "S";
  }
  if (mode & 0o2000) {
    written[5] = written[5] === "x" ? "s" : "S";
  }
  if (mode & 0o1000) {
    written[8] = written[8] === "x" ? "t" : "T";
  }

  return written.join("");
}

/** The last segment of a path: what a person recognises a file by. */
export function baseName(path: string): string {
  const segments = path.split("/").filter((segment) => segment !== "");
  return segments[segments.length - 1] ?? path;
}

/**
 * The extension a name carries, lower case, or null when it carries none.
 *
 * The *last* dot, because that is the one a file manager groups by: `archive.tar.gz` is a `.gz`
 * here, and every tarball in a folder reads as one type rather than as three. A name that begins
 * with the dot carries no extension at all — `~/.bashrc` is not a "bashrc file" — and neither does
 * one that ends with it.
 */
export function extensionOf(name: string): string | null {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) {
    return null;
  }
  return name.slice(dot + 1).toLowerCase();
}

/** The directory a path sits in, or null when there is none to go up to. */
export function parentPath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed.startsWith("/")) {
    return null;
  }
  const cut = trimmed.lastIndexOf("/");
  return cut <= 0 ? "/" : trimmed.slice(0, cut);
}

/** One name appended to a directory, never with a doubled separator. */
export function joinPath(directory: string, name: string): string {
  return directory.endsWith("/") ? `${directory}${name}` : `${directory}/${name}`;
}

/**
 * Every directory from the root down to the parent of `path`.
 *
 * This is what has to be open for a path to be visible in the tree, so it is the whole reason the
 * tree and the listing do not drift apart: navigate anywhere on the right, and the left opens
 * exactly the ancestors that make that place reachable.
 */
export function ancestorsOf(path: string): string[] {
  const segments = path.split("/").filter(Boolean);
  const trail = ["/"];
  let current = "";

  // The last segment is the entry itself; its ancestors are everything before it.
  for (const segment of segments.slice(0, -1)) {
    current += `/${segment}`;
    trail.push(current);
  }

  return trail;
}

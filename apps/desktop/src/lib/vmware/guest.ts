/**
 * The readable name behind a `.vmx` guest identifier.
 *
 * Workstation stores a machine's guest family as an id (`debian12-64`, `windows9-64`,
 * `darwin22-64`). The id is precise about details nobody needs in a card; what the card answers is
 * "what kind of system is this", and that is the family. An id this list does not know is left
 * alone rather than guessed at, which is also why the raw id is kept as a tooltip beside it.
 */
const FAMILIES: readonly (readonly [needle: string, label: string])[] = [
  ["windows", "Windows"],
  ["darwin", "macOS"],
  ["macos", "macOS"],
  ["ubuntu", "Ubuntu"],
  ["debian", "Debian"],
  ["redhat", "Red Hat Enterprise Linux"],
  ["rhel", "Red Hat Enterprise Linux"],
  ["centos", "CentOS"],
  ["rocky", "Rocky Linux"],
  ["alma", "AlmaLinux"],
  ["oracle", "Oracle Linux"],
  ["opensuse", "openSUSE"],
  ["suse", "SUSE Linux"],
  ["fedora", "Fedora"],
  ["amazon", "Amazon Linux"],
  ["photon", "Photon OS"],
  ["freebsd", "FreeBSD"],
  ["netware", "NetWare"],
  ["other", "Other"],
];

/** The family name for one guest id, or `null` when this build has no name for it. */
export function guestOsLabel(id: string | undefined | null): string | null {
  const value = (id ?? "").trim().toLowerCase();
  if (value === "") {
    return null;
  }
  return FAMILIES.find(([needle]) => value.includes(needle))?.[1] ?? null;
}

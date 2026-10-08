/**
 * The commands the interface knows by name.
 *
 * Deliberately small and deliberately shallow: one line about each, and no attempt to be a manual.
 * A list that tries to be complete is a list nobody reads while typing. An entry whose one line is
 * missing from the locale files still completes — the name is the useful part — and shows the word
 * for its kind where the description would have been.
 *
 * Names are the thing itself, so they are not translated and not spelled differently anywhere: the
 * descriptions live in the locale files under `commands.completion.command.<name>`.
 */

export type CatalogKind = "command" | "builtin";

export interface CatalogEntry {
  readonly name: string;
  readonly kind: CatalogKind;
}

/** The shell's own words: they belong to whatever shell is on the far end, not to a program. */
const BUILTINS: readonly string[] = [
  "alias",
  "cd",
  "echo",
  "exit",
  "export",
  "history",
  "jobs",
  "pwd",
  "printf",
  "source",
  "type",
  "umask",
  "unset",
  "which",
];

/** Programs, in the loose order of how often a line reaches for them. */
const COMMANDS: readonly string[] = [
  "ls",
  "cat",
  "less",
  "tail",
  "head",
  "grep",
  "find",
  "xargs",
  "sed",
  "awk",
  "wc",
  "sort",
  "uniq",
  "cut",
  "tr",
  "diff",
  "stat",
  "du",
  "df",
  "cp",
  "mv",
  "rm",
  "mkdir",
  "rmdir",
  "touch",
  "ln",
  "chmod",
  "chown",
  "tar",
  "gzip",
  "unzip",
  "rsync",
  "scp",
  "curl",
  "wget",
  "ssh",
  "ssh-keygen",
  "sudo",
  "systemctl",
  "journalctl",
  "service",
  "ps",
  "top",
  "free",
  "uptime",
  "whoami",
  "id",
  "uname",
  "hostname",
  "date",
  "kill",
  "pkill",
  "killall",
  "nohup",
  "tmux",
  "screen",
  "crontab",
  "mount",
  "umount",
  "lsblk",
  "lsof",
  "ss",
  "netstat",
  "ip",
  "ping",
  "traceroute",
  "dig",
  "nslookup",
  "iptables",
  "ufw",
  "apt",
  "dnf",
  "yum",
  "docker",
  "kubectl",
  "git",
  "make",
  "tee",
  "watch",
  "base64",
  "md5sum",
  "sha256sum",
  "openssl",
  "useradd",
  "usermod",
  "passwd",
  "su",
  "chattr",
  "getfacl",
  "setfacl",
  "seq",
];

export const COMMAND_CATALOG: readonly CatalogEntry[] = [
  ...BUILTINS.map((name): CatalogEntry => ({ name, kind: "builtin" })),
  ...COMMANDS.map((name): CatalogEntry => ({ name, kind: "command" })),
];

/** Where the one-line description of a command lives in the locale files. */
export function descriptionKeyFor(name: string): string {
  return `commands.completion.command.${name}`;
}

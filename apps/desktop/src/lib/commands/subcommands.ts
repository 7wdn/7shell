/**
 * The words that come after a command somebody already knows.
 *
 * A short table, written by hand, of what each command is usually followed by: `systemctl` is a word
 * nobody has to guess at, and neither is `git status`. It is not a manual and not derived from the
 * commands on the host — the point is a list short enough to read while typing, and a wrong word in
 * it would be worse than an absent one.
 *
 * Flags are here as words too (`grep -r`, `journalctl -u`): what comes after a command is what these
 * are, and the engine has no opinion about which of them start with a dash.
 *
 * The one-line description of a word lives in the locale files under
 * `commands.completion.subcommand.<command>.<word>`; a word without one shows the word for its kind,
 * which is the same fallback the catalogue has.
 */

interface SubcommandGroup {
  readonly command: string;
  /** In the order they are worth reading: the everyday ones first. */
  readonly words: readonly string[];
}

const GROUPS: readonly SubcommandGroup[] = [
  {
    command: "git",
    words: [
      "status",
      "pull",
      "push",
      "commit",
      "checkout",
      "log",
      "diff",
      "stash",
      "fetch",
      "branch",
      "add",
      "reset",
      "merge",
      "rebase",
      "remote",
    ],
  },
  {
    command: "systemctl",
    words: [
      "status",
      "start",
      "stop",
      "restart",
      "reload",
      "enable",
      "disable",
      "daemon-reload",
      "list-units",
      "is-active",
      "show",
    ],
  },
  {
    command: "docker",
    words: [
      "ps",
      "logs",
      "exec",
      "run",
      "images",
      "stop",
      "start",
      "restart",
      "rm",
      "rmi",
      "pull",
      "build",
      "compose",
      "inspect",
    ],
  },
  {
    command: "kubectl",
    words: ["get", "describe", "logs", "apply", "delete", "exec", "port-forward", "rollout", "scale", "config", "top"],
  },
  {
    command: "apt",
    words: ["update", "upgrade", "install", "remove", "search", "list", "autoremove", "policy"],
  },
  {
    command: "dnf",
    words: ["install", "update", "remove", "search", "list", "info", "upgrade", "clean"],
  },
  {
    command: "journalctl",
    words: ["-u", "-f", "--since", "--until", "-n", "-b", "-p", "--no-pager"],
  },
  {
    command: "grep",
    words: ["-r", "-i", "-n", "-v", "-E", "-A", "-B", "-l", "-c", "-w", "--color"],
  },
  {
    command: "ls",
    words: ["-l", "-a", "-h", "-R", "-t", "-S", "-1", "--color"],
  },
  {
    command: "curl",
    words: ["-X", "-H", "-d", "-o", "-O", "-L", "-k", "-v", "-s", "-I", "-F"],
  },
  {
    command: "ps",
    words: ["aux", "-ef", "-eo", "--sort"],
  },
  {
    command: "tar",
    words: ["-xzf", "-czf", "-xzvf", "-czvf", "-tzf", "-cjf"],
  },
];

export function subcommandsOf(command: string): readonly string[] {
  return GROUPS.find((group) => group.command === command.toLowerCase())?.words ?? [];
}

/** Where the one line about a word lives in the locale files. */
export function subcommandKeyFor(command: string, word: string): string {
  return `commands.completion.subcommand.${command.toLowerCase()}.${word}`;
}

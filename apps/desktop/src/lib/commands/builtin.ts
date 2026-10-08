/**
 * The commands this build ships.
 *
 * Written as data and stored like anything else the library holds: a fresh install has something to
 * type, and what it has can be edited, switched off or deleted like a command somebody wrote —
 * because it *is* one. That also means an improved one here does not travel into a library that
 * already has the old one, which is the price of not having two kinds of command.
 *
 * The names are what the line does rather than a sentence about it: a name is what somebody scans
 * for, and `tar czvf` reads the same in every language the interface speaks.
 */
export interface BuiltinCommand {
  readonly name: string;
  /** The short text that brings it up in the command line. */
  readonly prefix: string;
  /** The line, with `#1`…`#5` where values go. */
  readonly command: string;
}

export const BUILTIN_COMMANDS: readonly BuiltinCommand[] = [
  { name: "ps aux | grep", prefix: "psg", command: "ps aux | grep #1" },
  { name: "systemctl status", prefix: "svc", command: "systemctl status #1" },
  { name: "systemctl restart", prefix: "svcr", command: "systemctl restart #1" },
  { name: "journalctl -u -f", prefix: "jctl", command: "journalctl -u #1 -f" },
  { name: "tail -f", prefix: "logf", command: "tail -f #1" },
  { name: "grep -rn", prefix: "grn", command: "grep -rn #1 ." },
  { name: "find -name", prefix: "fnm", command: "find / -name #1 2>/dev/null" },
  { name: "df -h", prefix: "dfh", command: "df -h" },
  { name: "du -sh", prefix: "dush", command: "du -sh #1" },
  { name: "free -h", prefix: "mem", command: "free -h" },
  { name: "uptime", prefix: "up", command: "uptime" },
  { name: "whoami; id", prefix: "who", command: "whoami; id" },
  { name: "ss -tlnp", prefix: "listen", command: "ss -tlnp" },
  { name: "lsof -i", prefix: "lsof", command: "lsof -i :#1" },
  { name: "ping -c 4", prefix: "png", command: "ping -c 4 #1" },
  { name: "tar xzvf", prefix: "tarx", command: "tar xzvf #1" },
  { name: "tar czvf", prefix: "tarc", command: "tar czvf #1.tar.gz #1" },
  { name: "docker logs -f", prefix: "dlog", command: "docker logs -f --tail 100 #1" },
  { name: "docker exec -it", prefix: "dexec", command: "docker exec -it #1 sh" },
  { name: "git log --oneline", prefix: "glog", command: "git log --oneline -20" },
];

/**
 * The mark a saved connection carries when the connection kind is the better answer.
 *
 * A host's own system is what an SSH row shows once the machine has said what it is. A Docker
 * session on an Ubuntu host is still about Docker, and a local PowerShell session is about the
 * shell rather than about Windows; those rows say so with a mark of their own. Everything else
 * falls through to the host mark, and then to the plain terminal.
 *
 * Only kinds this build actually draws are returned. WSLC still has no mark: borrowing Docker's
 * would turn "what is this row" into a guess. WSL is the one kind that can answer with a
 * distribution's mark, because the name Windows registered the distribution under is an answer
 * even though the PTY has no exec channel for the ordinary host probe.
 */
import type { SystemMark } from "@/lib/system/icons";
import type { Session } from "@/types/session";

const POWERSHELL_SHELLS = new Set(["pwsh", "pwsh.exe", "powershell", "powershell.exe"]);

/**
 * WSL distributions whose mark this build can infer from the registered name.
 *
 * This is not the host probe: a WSL terminal is a PTY rather than an SSH session, so the probe has
 * no exec channel to ask. What it has is the name Windows registered the distribution under —
 * `Ubuntu-22.04`, `Debian`, `Alpine` — which names the family closely enough for a mark. An
 * unrecognised name keeps the WSL mark rather than borrowing a distribution's.
 */
const WSL_DISTRO_MARKS: readonly (readonly [needle: string, mark: SystemMark])[] = [
  ["alma", "alma"],
  ["alpine", "alpine"],
  ["arch", "arch"],
  ["centos", "centos"],
  ["debian", "debian"],
  ["deepin", "deepin"],
  ["fedora", "fedora"],
  ["linuxmint", "mint"],
  ["mint", "mint"],
  ["opensuse", "opensuse"],
  ["rhel", "redhat"],
  ["redhat", "redhat"],
  ["rocky", "rocky"],
  ["suse", "opensuse"],
  ["ubuntu", "ubuntu"],
];

function wslMark(session: Session): SystemMark {
  const name = (session.wsl?.distro ?? "").trim().toLowerCase();
  return WSL_DISTRO_MARKS.find(([needle]) => name.includes(needle))?.[1] ?? "wsl";
}

/** The mark of the machine this window is running on, for a local session that named no shell. */
export function platformMark(os: string | undefined): SystemMark | null {
  switch ((os ?? "").toLowerCase()) {
    case "windows":
      return "windows";
    case "macos":
    case "darwin":
      return "macos";
    case "linux":
      return "linux";
    default:
      return null;
  }
}

export function connectionMark(session: Session, platform?: string): SystemMark | null {
  switch (session.protocol) {
    case "rdp":
      return "windows";
    case "kubernetes":
      return "kubernetes";
    case "container":
      // The serialized default (Docker on this machine) is left out of the document, the same way
      // every other default is; the rest of the application reads an absent runtime as Docker.
      switch (session.container?.runtime ?? "docker") {
        case "docker":
          return "docker";
        case "podman":
          return "podman";
        case "nerdctl":
          return "nerdctl";
        default:
          return null;
      }
    case "wsl":
      return wslMark(session);
    case "local": {
      const shell = (session.local?.shell ?? "").toLowerCase();
      if (POWERSHELL_SHELLS.has(shell)) {
        return "powershell";
      }
      if (shell === "cmd" || shell === "cmd.exe") {
        return "cmd";
      }
      if (shell === "git-bash" || shell === "git-bash.exe") {
        return "git-bash";
      }
      // "System default" names no shell at all. The machine underneath is still an answer, and a
      // local row with no mark at all is the one thing this function exists to avoid.
      return platformMark(platform);
    }
    default:
      return null;
  }
}

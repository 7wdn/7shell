import { describe, expect, it } from "vitest";

import { connectionMark } from "@/lib/sessions/marks";
import { systemMarkup } from "@/lib/system/icons";
import type { Session } from "@/types/session";

function session(overrides: Partial<Session>): Session {
  return { protocol: "ssh", ...overrides } as unknown as Session;
}

describe("the mark a connection kind carries", () => {
  it("draws the engine a container session actually uses", () => {
    const marks: readonly (readonly [runtime: "docker" | "podman" | "nerdctl" | "wslc", mark: string | null])[] = [
      ["docker", "docker"],
      ["podman", "podman"],
      ["nerdctl", "nerdctl"],
      ["wslc", null],
    ];
    for (const [runtime, mark] of marks) {
      expect(
        connectionMark(session({ protocol: "container", container: { runtime, transport: "local" } })),
        `${runtime} borrows nothing it is not`,
      ).toBe(mark);
    }
    // The serialized default is absent from the document; the rest of the application reads that
    // as Docker, and so does the mark.
    expect(connectionMark(session({ protocol: "container" }))).toBe("docker");
  });

  it("draws a cluster and a desktop without asking the host", () => {
    expect(connectionMark(session({ protocol: "kubernetes" }))).toBe("kubernetes");
    expect(connectionMark(session({ protocol: "rdp" }))).toBe("windows");
  });

  it("infers a WSL distribution's mark from the name Windows registered", () => {
    const cases: readonly (readonly [distro: string, mark: string])[] = [
      ["Ubuntu-22.04", "ubuntu"],
      ["Debian", "debian"],
      ["Alpine", "alpine"],
      ["openSUSE-Leap-15.5", "opensuse"],
      ["OracleLinux_8_7", "wsl"],
      ["", "wsl"],
    ];
    for (const [distro, mark] of cases) {
      expect(
        connectionMark(session({ protocol: "wsl", wsl: { distro, args: "" } })),
        `${distro || "an unnamed distribution"} is drawn as ${mark}`,
      ).toBe(mark);
    }
  });

  it("draws the shell a local session starts", () => {
    const cases: readonly (readonly [shell: string, mark: string | null])[] = [
      ["pwsh", "powershell"],
      ["powershell.exe", "powershell"],
      ["cmd", "cmd"],
      ["git-bash", "git-bash"],
      ["bash", null],
    ];
    for (const [shell, mark] of cases) {
      expect(connectionMark(session({ protocol: "local", local: { shell, args: "" } }))).toBe(mark);
    }
  });

  it("gives a local session that named no shell the machine's own mark", () => {
    expect(connectionMark(session({ protocol: "local", local: { shell: "", args: "" } }), "windows")).toBe("windows");
    expect(connectionMark(session({ protocol: "local", local: { shell: "bash", args: "" } }), "macos")).toBe("macos");
    expect(connectionMark(session({ protocol: "local", local: { shell: "bash", args: "" } }), "linux")).toBe("linux");
    expect(connectionMark(session({ protocol: "local", local: { shell: "", args: "" } }))).toBeNull();
  });

  it("leaves the other protocols to the host mark", () => {
    for (const protocol of ["ssh", "telnet", "serial", "ftp", "webdav", "rawTcp"] as const) {
      expect(connectionMark(session({ protocol }))).toBeNull();
    }
  });

  it("only returns marks this build has a drawing for", () => {
    const marks = [
      connectionMark(session({ protocol: "rdp" })),
      connectionMark(session({ protocol: "kubernetes" })),
      connectionMark(session({ protocol: "wsl", wsl: { distro: "Ubuntu-22.04", args: "" } })),
      connectionMark(session({ protocol: "wsl", wsl: { distro: "Unknown", args: "" } })),
      connectionMark(session({ protocol: "container", container: { runtime: "docker", transport: "local" } })),
      connectionMark(session({ protocol: "container", container: { runtime: "podman", transport: "local" } })),
      connectionMark(session({ protocol: "container", container: { runtime: "nerdctl", transport: "local" } })),
      connectionMark(session({ protocol: "local", local: { shell: "pwsh", args: "" } })),
      connectionMark(session({ protocol: "local", local: { shell: "cmd", args: "" } })),
      connectionMark(session({ protocol: "local", local: { shell: "git-bash", args: "" } })),
    ];
    for (const mark of marks) {
      expect(mark).not.toBeNull();
      expect(systemMarkup(mark), `${mark} has no file in assets/os`).toContain("<svg");
    }
  });
});

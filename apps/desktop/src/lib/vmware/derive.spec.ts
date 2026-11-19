import { describe, expect, it } from "vitest";

import { vmGuestName, vmGuestSession, vmSessionFromScan } from "@/lib/vmware/derive";
import type { Session } from "@/types/session";

function vmSession(overrides: Partial<Session> = {}): Session {
  return {
    id: "vm-1",
    name: "uos_test",
    groupId: "lab",
    protocol: "vmware",
    host: "",
    port: 0,
    user: "",
    auth: { method: "password", hasSecret: false },
    terminal: {
      terminalType: "xterm-256color",
      keepaliveSeconds: 30,
      encoding: "utf-8",
      backspace: "del",
      forwardAgent: false,
      record: false,
    },
    vmware: { vmId: "564db5f23515de032cd9338ba8d831b7", sshUser: "z", sshPort: 2222, rdpPort: 3390 },
    updatedAt: 1,
    ...overrides,
  };
}

describe("the sessions a VM workspace makes", () => {
  it("builds an imported VM session from a scan answer, with no address in it", () => {
    const session = vmSessionFromScan({
      id: "564db5f23515de032cd9338ba8d831b7",
      name: "uos_test",
      guestOs: "debian12-64",
      running: true,
    });
    expect(session.protocol).toBe("vmware");
    expect(session.name).toBe("uos_test");
    expect(session.host).toBe("");
    expect(session.groupId).toBeUndefined();
    expect(session.vmware).toMatchObject({ vmId: "564db5f23515de032cd9338ba8d831b7", sshPort: 22, rdpPort: 3389 });
  });

  it("puts an imported session in the group the import named", () => {
    const session = vmSessionFromScan(
      { id: "564db5f23515de032cd9338ba8d831b7", name: "uos_test", running: false },
      "group-1",
    );
    expect(session.groupId).toBe("group-1");
  });

  it("builds an SSH session from the VM's settings and the resolved address", () => {
    const guest = vmGuestSession(vmSession(), "ssh", "192.168.254.130");
    expect(guest.name).toBe(vmGuestName("uos_test", "ssh"));
    expect(guest.protocol).toBe("ssh");
    expect(guest.host).toBe("192.168.254.130");
    expect(guest.port).toBe(2222);
    expect(guest.user).toBe("z");
    expect(guest.groupId).toBe("lab");
    expect(guest.vmware).toBeUndefined();
  });

  it("builds a desktop session on the RDP port with the desktop defaults", () => {
    const guest = vmGuestSession(vmSession(), "rdp", "192.168.254.130");
    expect(guest.protocol).toBe("rdp");
    expect(guest.port).toBe(3390);
    expect(guest.user).toBe("");
    expect(guest.desktop).toMatchObject({ width: 1280, height: 1024, redirectDrives: true });
  });

  it("keeps what somebody changed on the existing session and only follows the address", () => {
    const existing: Session = {
      ...vmSession(),
      id: "ssh-1",
      name: "UOS shell",
      protocol: "ssh",
      host: "192.168.254.99",
      port: 22,
      user: "old",
      auth: { method: "publicKey", hasSecret: true },
      groupId: "custom",
    };
    const guest = vmGuestSession(vmSession(), "ssh", "192.168.254.130", existing);
    expect(guest.id).toBe("ssh-1");
    expect(guest.name).toBe("UOS shell");
    expect(guest.groupId).toBe("custom");
    expect(guest.auth).toEqual({ method: "publicKey", hasSecret: true });
    expect(guest.host).toBe("192.168.254.130");
    expect(guest.port).toBe(2222);
    expect(guest.user).toBe("z");
  });
});

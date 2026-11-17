import { describe, expect, it } from "vitest";

import { guestOsLabel } from "@/lib/vmware/guest";

describe("the guest family behind a .vmx identifier", () => {
  it("names the families Workstation's ids carry", () => {
    expect(guestOsLabel("ubuntu-64")).toBe("Ubuntu");
    expect(guestOsLabel("debian12-64")).toBe("Debian");
    expect(guestOsLabel("windows9-64")).toBe("Windows");
    expect(guestOsLabel("darwin22-64")).toBe("macOS");
    expect(guestOsLabel("rhel9-64")).toBe("Red Hat Enterprise Linux");
    expect(guestOsLabel("opensuse15-64")).toBe("openSUSE");
  });

  it("leaves an identifier it does not know alone", () => {
    expect(guestOsLabel("plan9-64")).toBeNull();
    expect(guestOsLabel("")).toBeNull();
    expect(guestOsLabel(undefined)).toBeNull();
  });
});

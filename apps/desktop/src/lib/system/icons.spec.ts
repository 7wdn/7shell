/**
 * Which mark a host gets, and that every mark the table names has a drawing behind it.
 *
 * The table is a list of judgement calls, so what is worth pinning down is the order they are
 * applied in — the host's own id, then the family it says it belongs to, then the kernel — and the
 * two shapes that would otherwise be silent mistakes: a distribution this build has no mark for
 * being drawn as some other distribution, and a name in the table with no file behind it.
 */
import { describe, expect, it } from "vitest";

import { AVAILABLE_MARKS, systemLabel, systemMark, systemMarkup } from "./icons";
import type { HostSystem } from "@/types/system";

/** An identity with only the fields a case cares about, which is what the wire sends anyway. */
function system(fields: Partial<HostSystem>): HostSystem {
  return { name: null, kernel: null, distro: null, distroLike: [], ...fields };
}

const UBUNTU = system({
  name: "Ubuntu 24.04.1 LTS",
  kernel: "Linux",
  distro: "ubuntu",
  distroLike: ["debian"],
});

describe("the mark a host gets", () => {
  it("draws the mark of the id the host published", () => {
    expect(systemMark(UBUNTU)).toBe("ubuntu");
    expect(systemMark(system({ kernel: "Linux", distro: "almalinux" }))).toBe("alma");
    expect(systemMark(system({ kernel: "Linux", distro: "rocky" }))).toBe("rocky");
    expect(systemMark(system({ kernel: "Linux", distro: "linuxmint" }))).toBe("mint");
  });

  it("draws a derived distribution as the family it says it belongs to", () => {
    // Kali publishes no id this build draws, and says it is Debian; that is the answer the row is
    // looking for, and it is the host's own claim rather than this build's guess.
    expect(systemMark(system({ kernel: "Linux", distro: "kali", distroLike: ["debian"] }))).toBe("debian");
    expect(systemMark(system({ kernel: "Linux", distro: "pop", distroLike: ["ubuntu", "debian"] }))).toBe("ubuntu");
    expect(systemMark(system({ kernel: "Linux", distro: "manjaro", distroLike: ["arch"] }))).toBe("arch");
  });

  it("keeps the order the host published, rather than picking a favourite", () => {
    // A host listing two families is narrowing down in its own order; a table that reordered them
    // would draw something the host did not say.
    expect(systemMark(system({ kernel: "Linux", distro: "zorin", distroLike: ["ubuntu", "debian"] }))).toBe("ubuntu");
    expect(systemMark(system({ kernel: "Linux", distro: "zorin", distroLike: ["debian", "ubuntu"] }))).toBe("debian");
  });

  it("never draws a distribution the host did not claim", () => {
    // OpenBSD is a system with no mark here. Borrowing FreeBSD's daemon — or the generic Linux
    // penguin — would be this build answering a question the host did not.
    expect(systemMark(system({ name: "OpenBSD", kernel: "OpenBSD" }))).toBeNull();
    expect(systemMark(system({ kernel: "Linux", distro: "void" }))).toBe("linux");
  });

  it("falls back to the kernel when there is no distribution to read", () => {
    expect(systemMark(system({ kernel: "Darwin" }))).toBe("macos");
    expect(systemMark(system({ kernel: "FreeBSD" }))).toBe("freebsd");
    expect(systemMark(system({ kernel: "Linux" }))).toBe("linux");
  });

  it("reads a Windows host from either of the two ways it introduces itself", () => {
    // `ver`, which the probe reads as an id, and the kernel name of a Windows host whose shell is
    // a POSIX one — neither of which keeps an `/etc/os-release`.
    expect(systemMark(system({ name: "Microsoft Windows [Version 10.0.19045.3803]", distro: "windows" }))).toBe(
      "windows",
    );
    expect(systemMark(system({ kernel: "MINGW64_NT-10.0", name: "MINGW64_NT-10.0" }))).toBe("windows");
    expect(systemMark(system({ kernel: "CYGWIN_NT-10.0" }))).toBe("windows");
  });

  it("draws nothing for a host that said nothing", () => {
    expect(systemMark(null)).toBeNull();
    expect(systemMark(system({}))).toBeNull();
  });

  it("has a drawing behind every mark it hands out", () => {
    const marks = [
      systemMark(UBUNTU),
      systemMark(system({ kernel: "Linux", distro: "debian" })),
      systemMark(system({ kernel: "Linux", distro: "centos" })),
      systemMark(system({ kernel: "Linux", distro: "rhel" })),
      systemMark(system({ kernel: "Linux", distro: "fedora" })),
      systemMark(system({ kernel: "Linux", distro: "alpine" })),
      systemMark(system({ kernel: "Linux", distro: "arch" })),
      systemMark(system({ kernel: "Linux", distro: "opensuse-leap" })),
      systemMark(system({ kernel: "Linux", distro: "deepin" })),
      systemMark(system({ kernel: "Linux" })),
      systemMark(system({ kernel: "Darwin" })),
      systemMark(system({ kernel: "FreeBSD" })),
      systemMark(system({ distro: "windows" })),
    ];

    for (const mark of marks) {
      expect(mark, "a system the table draws").not.toBeNull();
      expect(AVAILABLE_MARKS).toContain(mark);
      expect(systemMarkup(mark), `${mark} has no file in assets/os`).not.toBeNull();
    }

    // And the other direction: a file nothing can ever draw is a file somebody forgot to wire up.
    for (const file of AVAILABLE_MARKS) {
      expect(systemMarkup(file), `${file}.svg is loaded`).toContain("<svg");
    }
  });
});

describe("what a system is called", () => {
  it("prefers the host's own name for itself", () => {
    expect(systemLabel(UBUNTU)).toBe("Ubuntu 24.04.1 LTS");
    expect(systemLabel(system({ kernel: "FreeBSD" }))).toBe("FreeBSD");
    expect(systemLabel(system({ distro: "windows", name: "Microsoft Windows [Version 10.0.19045.3803]" }))).toBe(
      "Microsoft Windows [Version 10.0.19045.3803]",
    );
  });

  it("falls back to what it called its system, and to nothing when it said nothing", () => {
    expect(systemLabel(system({ distro: "ubuntu" }))).toBe("ubuntu");
    expect(systemLabel(system({}))).toBeNull();
    expect(systemLabel(null)).toBeNull();
  });
});

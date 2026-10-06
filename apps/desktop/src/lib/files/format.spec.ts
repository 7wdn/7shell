/**
 * The path helpers.
 *
 * `ancestorsOf` is the one with a rule worth pinning: it decides which branches the tree opens, and
 * getting it wrong is visible as a tree that does not show where the listing is.
 */
import { describe, expect, it } from "vitest";

import { ancestorsOf, extensionOf, familyOf, formatMode } from "@/lib/files/format";

describe("ancestorsOf", () => {
  it("lists every directory that has to be open for a path to be visible", () => {
    expect(ancestorsOf("/var/log/nginx")).toEqual(["/", "/var", "/var/log"]);
  });

  it("leaves out the entry itself, which is what the last segment is", () => {
    expect(ancestorsOf("/etc/hosts")).toEqual(["/", "/etc"]);
  });

  it("answers with the root for a path directly under it", () => {
    expect(ancestorsOf("/home")).toEqual(["/"]);
    expect(ancestorsOf("/")).toEqual(["/"]);
  });

  it("ignores a trailing separator rather than inventing an empty segment", () => {
    expect(ancestorsOf("/var/log/")).toEqual(["/", "/var"]);
  });
});

describe("extensionOf", () => {
  it("reads what follows the last dot, which is what a file manager groups by", () => {
    expect(extensionOf("NVM.Desktop_4.4.0-1.x86_64.rpm")).toBe("rpm");
    expect(extensionOf("archive.tar.gz"), "one type for every tarball, not three").toBe("gz");
  });

  it("reads a name without one as having none", () => {
    expect(extensionOf("Makefile")).toBeNull();
    expect(extensionOf("ss-e2e-10719-done")).toBeNull();
  });

  it("does not read a leading dot as an extension", () => {
    expect(extensionOf(".bashrc")).toBeNull();
    expect(extensionOf(".config.json"), "the last dot wins, and it has one").toBe("json");
  });

  it("does not read a trailing dot as an extension either", () => {
    expect(extensionOf("something.")).toBeNull();
  });

  it("reads a type the same whatever case it was written in", () => {
    expect(extensionOf("README.TXT")).toBe("txt");
  });
});

describe("familyOf", () => {
  it("puts every way of packing a file into one type", () => {
    for (const extension of ["zip", "rar", "7z", "tar", "gz", "tgz", "xz", "zst", "bz2"]) {
      expect(familyOf(extension), `${extension} is an archive`).toBe("archive");
    }
  });

  it("knows the families a folder of work and a folder of downloads are made of", () => {
    expect(familyOf("png")).toBe("image");
    expect(familyOf("mp4")).toBe("video");
    expect(familyOf("mp3")).toBe("audio");
    expect(familyOf("pdf")).toBe("document");
    expect(familyOf("txt")).toBe("text");
  });

  it("leaves a type it does not know to be shown as itself", () => {
    expect(familyOf("rpm")).toBeNull();
    expect(familyOf("deb")).toBeNull();
    expect(familyOf("part")).toBeNull();
  });
});

describe("formatMode", () => {
  it("writes the nine bits the way ls does", () => {
    expect(formatMode(0o755)).toBe("rwxr-xr-x");
    expect(formatMode(0o644)).toBe("rw-r--r--");
    expect(formatMode(0)).toBe("---------");
  });

  it("reads the bits the same whether or not the file type came with them", () => {
    // Servers differ here: some send a plain 0o644, some the whole st_mode of a regular file.
    expect(formatMode(0o100644)).toBe(formatMode(0o644));
  });

  it("says a mode the server did not send is absent rather than empty", () => {
    expect(formatMode(undefined)).toBe("—");
  });

  it("keeps setuid, setgid and the sticky bit, which are why anybody reads this column", () => {
    expect(formatMode(0o4755)).toBe("rwsr-xr-x");
    expect(formatMode(0o2755)).toBe("rwxr-sr-x");
    expect(formatMode(0o1777)).toBe("rwxrwxrwt");
  });

  it("capitalises the special bit when there is no execute bit under it", () => {
    expect(formatMode(0o4644)).toBe("rwSr--r--");
    expect(formatMode(0o1666)).toBe("rw-rw-rwT");
  });
});

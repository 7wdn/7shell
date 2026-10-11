/**
 * Which mark stands for which host, and the mark's own drawing.
 *
 * The backend answers with what the host called itself (`HostSystem`). What to *draw* for that is
 * this build's opinion, and this is the only place it lives: a host names itself in a vocabulary no
 * icon set has ever matched, so the mapping is a list of judgement calls rather than a lookup the
 * other side of the wire could have done for us.
 *
 * Three rules, applied in this order:
 *
 * 1. **The id the host published wins.** `ID=ubuntu` draws the Ubuntu mark.
 * 2. **A family the host names beats a generic picture.** A distribution this build has no mark for
 *    is drawn as the family it says it belongs to (`ID_LIKE="ubuntu debian"` → the Ubuntu mark,
 *    which is what a Mint row would show if Mint had no mark of its own). The row is answering "what
 *    kind of machine is this", and a family is a truer answer than a penguin. It is never drawn as
 *    an unrelated distribution, and never as one the host did not claim.
 * 3. **The kernel is the last word.** `Darwin` is macOS, `FreeBSD` is FreeBSD, and Linux whose
 *    distribution is unknown is the generic Tux — not a guess at which distribution it might be.
 *
 * A host that said nothing gets no mark, and its row keeps the plain terminal it had before any of
 * this existed. That is the absence of an answer rather than a failure, which is why nothing here
 * invents a picture for it.
 */

import type { HostSystem } from "@/types/system";

/** The marks this build ships, named after the files in `assets/os`. */
export type SystemMark =
  | "alma"
  | "alpine"
  | "arch"
  | "centos"
  | "debian"
  | "deepin"
  | "fedora"
  | "freebsd"
  | "linux"
  | "macos"
  | "mint"
  | "opensuse"
  | "redhat"
  | "rocky"
  | "ubuntu"
  | "windows";

/**
 * The distributions this build draws, keyed by the id they publish in `/etc/os-release`.
 *
 * Only ids, and only the ones a person would recognise as their own system: a derived distribution
 * belongs in the table when its mark exists (Linux Mint has one), and otherwise resolves through
 * `ID_LIKE` to its family. `windows` is here because the probe puts it there: it is the one system
 * that keeps no `/etc/os-release` and answers no `uname`, and `ver` is its id.
 */
const BY_DISTRO: Readonly<Partial<Record<string, SystemMark>>> = {
  alpine: "alpine",
  almalinux: "alma",
  arch: "arch",
  archlinux: "arch",
  cachyos: "arch",
  centos: "centos",
  debian: "debian",
  deepin: "deepin",
  endeavouros: "arch",
  fedora: "fedora",
  linuxmint: "mint",
  manjaro: "arch",
  mint: "mint",
  opensuse: "opensuse",
  "opensuse-leap": "opensuse",
  "opensuse-tumbleweed": "opensuse",
  redhat: "redhat",
  rhel: "redhat",
  rocky: "rocky",
  sles: "opensuse",
  suse: "opensuse",
  ubuntu: "ubuntu",
  windows: "windows",
};

/**
 * Kernel names that are a system of their own rather than a distribution.
 *
 * A Windows host whose shell is a POSIX one (`MINGW64_NT-10.0`, `CYGWIN_NT-10.0`, `MSYS_NT-10.0`)
 * says so here and nowhere else: it keeps no `/etc/os-release`, so there is no id to read.
 */
const BY_KERNEL: Readonly<Partial<Record<string, SystemMark>>> = {
  Darwin: "macos",
  FreeBSD: "freebsd",
};

/** The kernel prefixes that mean "this is Windows with a POSIX shell on top". */
const WINDOWS_KERNELS: readonly string[] = ["MINGW", "CYGWIN", "MSYS", "Windows"];

/** Which mark stands for what a host said, or `null` when this build has none for it. */
export function systemMark(system: HostSystem | null): SystemMark | null {
  if (!system) return null;

  if (system.distro) {
    const own = BY_DISTRO[system.distro];
    if (own) return own;

    // The order the host published is the order of preference: the first family it named that this
    // build draws. A host that lists several is narrowing down, not ranking alternatives.
    for (const like of system.distroLike) {
      const family = BY_DISTRO[like];
      if (family) return family;
    }
  }

  if (system.kernel) {
    const named = BY_KERNEL[system.kernel];
    if (named) return named;
    if (WINDOWS_KERNELS.some((prefix) => system.kernel?.startsWith(prefix))) return "windows";
  }

  // Linux and nothing else known: the generic mark, which is an answer rather than a guess.
  return system.kernel === "Linux" ? "linux" : null;
}

/** What to call the system in a sentence: whatever the host said about itself, in its own words. */
export function systemLabel(system: HostSystem | null): string | null {
  if (!system) return null;
  return system.name ?? system.distro ?? system.kernel ?? null;
}

/**
 * The drawings, read out of `assets/os` at build time.
 *
 * Raw rather than as URLs: a mark has to be inline to be sized by the row it sits in and to take a
 * class the way a letter does, and sixteen files of a few kilobytes each is not a bundle worth
 * splitting over.
 */
const FILES = import.meta.glob("../../assets/os/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Readonly<Record<string, string>>;

/** The drawings by mark name, taken from the file name each one was loaded from. */
const MARKUP: Readonly<Record<string, string>> = Object.fromEntries(
  Object.entries(FILES).flatMap(([path, raw]) => {
    const markup = rowMarkup(raw);
    return markup ? [[path.slice(path.lastIndexOf("/") + 1, -".svg".length), markup]] : [];
  }),
);

/** Every mark that actually has a drawing, for the test that keeps the table honest. */
export const AVAILABLE_MARKS: readonly string[] = Object.keys(MARKUP).sort();

/**
 * One drawing, fit to sit in a row.
 *
 * The files are downloads as much as they are drawings: an XML prolog, a doctype, a fixed pixel
 * size and a handful of editor attributes come with them, and none of that belongs inside a 13px
 * text line. They are taken out here rather than edited out of the files, so that dropping in
 * another download with the same name keeps working — the file stays whatever the icon library
 * handed over, and what a row needs is decided where rows are drawn.
 */
export function systemMarkup(mark: SystemMark | string | null): string | null {
  if (!mark) return null;
  return MARKUP[mark] ?? null;
}

/** The `<svg>` element out of a downloaded file, with what a row cannot use taken off it. */
function rowMarkup(raw: string): string | null {
  const start = raw.indexOf("<svg");
  const end = raw.lastIndexOf("</svg>");
  if (start < 0 || end < start) return null;

  const openEnd = raw.indexOf(">", start);
  const opening = raw
    .slice(start, openEnd + 1)
    .replace(/\s(?:width|height|class|style|t|p-id)="[^"]*"/g, "")
    .replace(/^<svg/, '<svg aria-hidden="true" focusable="false"');

  return `${opening}${raw.slice(openEnd + 1, end)}</svg>`;
}

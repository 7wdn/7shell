/**
 * Reads the GLIBC symbol versions an ELF binary asks for.
 *
 * A binary built against a newer glibc runs on anything at or above that version and fails on
 * anything below it, so the highest `GLIBC_2.x` string in the file is the floor a distribution
 * has to meet. The string table is the only place this is written down without a full ELF parser.
 */
const GLIBC_VERSION = /GLIBC_2\.(\d+)/g;

export function maxGlibcVersion(contents) {
  let highest = -1;
  let text = "";
  for (const match of contents.toString("latin1").matchAll(GLIBC_VERSION)) {
    const minor = Number(match[1]);
    if (minor > highest) {
      highest = minor;
      text = `GLIBC_2.${minor}`;
    }
  }
  return highest < 0 ? null : { minor: highest, text };
}

export function assertGlibcFloor(contents, allowedMinor, label) {
  const found = maxGlibcVersion(contents);
  if (!found) {
    throw new Error(`${label} does not name a GLIBC version; refusing to call its floor satisfied`);
  }
  if (found.minor > allowedMinor) {
    throw new Error(`${label} needs ${found.text}, above the allowed GLIBC_2.${allowedMinor}`);
  }
  return found.text;
}

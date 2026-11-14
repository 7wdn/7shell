// The compatibility bridge is only useful if its glibc floor is checked from the binary itself.
// These cases keep the reader honest about the two things that have bitten otherwise: a missing
// version string (which must not read as "passes") and a version newer than the allowed one.

import assert from "node:assert/strict";
import { assertGlibcFloor, maxGlibcVersion } from "../lib/linux-floor.mjs";

assert.deepEqual(maxGlibcVersion(Buffer.from("xx GLIBC_2.17 yy GLIBC_2.28 zz")), {
  minor: 28,
  text: "GLIBC_2.28",
});
assert.deepEqual(maxGlibcVersion(Buffer.from("GLIBC_2.34 GLIBC_2.2.5")), {
  minor: 34,
  text: "GLIBC_2.34",
});
assert.equal(maxGlibcVersion(Buffer.from("no versions here")), null);
assert.throws(() => assertGlibcFloor(Buffer.from("GLIBC_2.34"), 28, "the bridge"), /the bridge needs GLIBC_2\.34/);
assert.throws(() => assertGlibcFloor(Buffer.from("nothing"), 28, "the bridge"), /does not name a GLIBC version/);
assert.equal(assertGlibcFloor(Buffer.from("GLIBC_2.17 GLIBC_2.28"), 28, "the bridge"), "GLIBC_2.28");

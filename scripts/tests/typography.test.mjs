// The type scale, on the page it was worked out for.
//
// Four sizes and one weight, each with a job: a name is `text-sm`/medium, a sentence is `text-sm`,
// a supporting sentence is `text-xs`, and machine text — a code, a fingerprint, a version — is
// `text-[11px]` in the monospace face. Nothing is smaller than that, and nothing is bolder than
// medium. It is written out in `docs/ui-conventions.md`.
//
// The settings page is where it is checked first, because it is where the drift was: a page that
// grew a 10px line here and a `font-semibold` there stops having a scale and starts having
// accidents, and an accidental size is one nobody can point at a reason for.
//
// What it looks at is the class list in the file, which is the mechanism: a test document lays
// nothing out, so a size that was never applied is not something an assertion could read back.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

const SETTINGS = path.join(REPO_ROOT, "apps", "desktop", "src", "components", "settings");

/** Every component of the settings page, and the shared pieces it draws. */
function sources() {
  return readdirSync(SETTINGS)
    .filter((name) => name.endsWith(".vue"))
    .map((name) => ({ name, source: readFileSync(path.join(SETTINGS, name), "utf8") }));
}

/** Fails the run if the scan read nothing: a scan that finds no file is a broken check. */
function files() {
  const found = sources();
  assert.ok(found.length > 0, "no settings component was read at all, which is a broken check");
  return found;
}

test("nothing on the settings page is drawn below the smallest step", () => {
  const offenders = [];
  for (const { name, source } of files()) {
    for (const match of source.matchAll(/text-\[(\d+(?:\.\d+)?)px\]/g)) {
      if (Number(match[1]) < 11) {
        offenders.push(`${name}: ${match[0]}`);
      }
    }
  }

  assert.deepEqual(offenders, [], `the smallest step is 11px, for machine text: ${offenders.join(", ")}`);
});

test("nothing on the settings page is louder than medium", () => {
  const offenders = [];
  for (const { name, source } of files()) {
    for (const match of source.matchAll(/font-(bold|semibold|extrabold|black)\b/g)) {
      offenders.push(`${name}: ${match[0]}`);
    }
  }

  assert.deepEqual(
    offenders,
    [],
    `a name is medium and a sentence is not bold, so there is nothing below to shout with: ${offenders.join(", ")}`,
  );
});

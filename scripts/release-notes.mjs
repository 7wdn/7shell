#!/usr/bin/env node
// Prints the changelog section for one version, for the release workflow to use as the body of a
// GitHub release.
//
//   node scripts/release-notes.mjs 0.1.1          the section, ready to publish
//   node scripts/release-notes.mjs v0.1.1         the same: the tag is what carries the `v`
//   node scripts/release-notes.mjs 0.1.1 --check  answer whether the section exists, write no body
//
// The version heading is left out of what is printed: a release is already named after its tag, and
// a body that opens by repeating its own title reads like something no person wrote. What is
// printed is the section's own content, ending in exactly one newline.
//
// A version with no section, or with an empty one, is an error in both modes. `--check` exists so a
// release built from a tag whose notes were never written fails before the tag, and the plain mode
// refuses to print nothing rather than let a pipeline publish the blank it got.

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

/**
 * The repository root, worked out here rather than imported from `lib/descriptors.mjs`.
 *
 * The release workflow runs this script with nothing but Node — no `pnpm install`, no development
 * dependencies — so a `CHANGELOG.md` path is not worth an import that drags YAML in with it: the
 * publish step then needs `node_modules` to exist, which it does on a machine where the repository
 * has been installed and does not on a runner. It took a release run to find that out;
 * `scripts/tests/releaseTooling.test.mjs` is what stops it happening again.
 */
const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const CHANGELOG = path.join(REPO_ROOT, "CHANGELOG.md");

/** A version as it is written in an argument or a tag: `0.1.1` from either spelling. */
function normalizeVersion(version) {
  return String(version)
    .trim()
    .replace(/^v(?=\d)/, "");
}

/** The version a `##` heading names, or `null` when the line is not a version heading. */
function headingVersion(line) {
  const match = /^##[ \t]+\[?(v?\d[^\s\]]*)\]?(?:[ \t]+-[ \t]+.*)?[ \t]*$/.exec(line);
  return match ? normalizeVersion(match[1]) : null;
}

/**
 * The body of `version`'s section, or `null` when the changelog has no such section.
 *
 * A section runs from its `##` heading up to the next `##` heading, so the `###` headings inside it
 * (Added and the like) and everything under them travel with it. Anything written below the last
 * section — a link definition, a note about the file — is deliberately not part of it.
 */
export function releaseNotesFor(changelog, version) {
  const lines = changelog.split(/\r?\n/);
  const wanted = normalizeVersion(version);
  const start = lines.findIndex((line) => headingVersion(line) === wanted);
  if (start === -1) {
    return null;
  }

  const body = [];
  for (let index = start + 1; index < lines.length; index += 1) {
    // A `##` heading starts the next version, and a link definition line belongs to the file rather
    // than to the version above it: without this, the last section would carry the whole reference
    // list into whatever a release publishes.
    if (/^##[ \t]/.test(lines[index]) || /^\[[^\]]+\]:[ \t]/.test(lines[index])) {
      break;
    }
    body.push(lines[index]);
  }
  return body.join("\n");
}

function usage(message) {
  console.error(message);
  console.error("Usage: node scripts/release-notes.mjs <version> [--check]");
  process.exit(2);
}

async function main() {
  const args = process.argv.slice(2);
  const unknown = args.filter((argument) => argument.startsWith("--") && argument !== "--check");
  if (unknown.length > 0) {
    usage(`Unknown option: ${unknown[0]}`);
  }

  const versions = args.filter((argument) => !argument.startsWith("--"));
  if (versions.length !== 1) {
    usage(versions.length === 0 ? "Give the version whose notes to print." : "Give exactly one version.");
  }

  const checkOnly = args.includes("--check");
  const version = normalizeVersion(versions[0]);
  const changelog = await readFile(CHANGELOG, "utf8");
  const notes = releaseNotesFor(changelog, version);

  if (notes === null) {
    // The line to act on is the first one: a CI log folds the rest away.
    console.error(`CHANGELOG.md has no section for ${version}, so there are no release notes to publish.`);
    console.error("Write the section before tagging: a release whose body is empty looks published and says nothing.");
    process.exit(1);
  }

  const body = notes.trim();
  if (body === "") {
    console.error(`CHANGELOG.md has a section for ${version}, and it is empty.`);
    console.error("An empty body publishes the same nothing a missing section would; write the notes first.");
    process.exit(1);
  }

  if (checkOnly) {
    console.log(`release notes: ${version} is written in CHANGELOG.md`);
  } else {
    process.stdout.write(`${body}\n`);
  }
}

// Only a direct run reads a changelog and exits: the guard test imports `releaseNotesFor` and must
// not have the command's own argument parsing run underneath it.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}

#!/usr/bin/env node
// Prints the changelog section for one version, for the release workflow to use as the body of a
// GitHub release and as the notes in the update manifest.
//
//   node scripts/release-notes.mjs 0.1.1                 both language parts, for the update notes
//   node scripts/release-notes.mjs 0.1.1 --lang zh       the Chinese part, for the GitHub body
//   node scripts/release-notes.mjs 0.1.1 --lang en       the English part
//   node scripts/release-notes.mjs 0.1.1 --check         answer whether the section is publishable
//   node scripts/release-notes.mjs v0.1.1                the same: the tag is what carries the `v`
//
// The version heading is left out of what is printed: a release is already named after its tag, and
// a body that opens by repeating its own title reads like something no person wrote. What is
// printed is the section's own content, ending in exactly one newline.
//
// From 0.4.0 a section carries `### English` and `### 中文`. The GitHub body asks for the Chinese
// part alone; the update manifest takes both, because the application picks the one its interface
// is speaking. A version with no section, an empty one, or (from 0.4.0) one missing a language is
// an error in both modes. `--check` exists so a release built from a tag whose notes were never
// written fails before the tag, and the plain mode refuses to print nothing rather than let a
// pipeline publish the blank it got.

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

/**
 * The first release whose section must carry both languages.
 *
 * Older sections stay as they were written; requiring a Chinese translation of a release already
 * published would turn a rebuild of an old tag into a content edit. From this version on, the
 * GitHub body is Chinese and the update manifest carries both, so both have to exist.
 */
const BILINGUAL_FROM = [0, 4, 0];

/** The headings a bilingual version section is split by. They are H3: an H2 would end the version. */
const LANGUAGE_HEADINGS = {
  en: /^###[ \t]+English[ \t]*$/,
  zh: /^###[ \t]+中文[ \t]*$/,
};

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

/** Whether this version's section is required to have both language parts. */
function requiresBilingual(version) {
  const parts = normalizeVersion(version)
    .split(".")
    .map((part) => Number.parseInt(part, 10));
  if (parts.some((part) => Number.isNaN(part))) {
    return false;
  }
  for (let index = 0; index < BILINGUAL_FROM.length; index += 1) {
    const own = parts[index] ?? 0;
    const floor = BILINGUAL_FROM[index];
    if (own !== floor) {
      return own > floor;
    }
  }
  return true;
}

/** Whether a version section has any language heading at all. */
function hasLanguageHeadings(notes) {
  return Object.values(LANGUAGE_HEADINGS).some((pattern) => notes.split(/\r?\n/).some((line) => pattern.test(line)));
}

/**
 * The content under one language heading, without the heading itself.
 *
 * `###` ends the part, and so does `##`: the second is already outside a version section, but the
 * caller may pass a hand-built body. `####` headings inside the part stay. A missing heading is
 * `null`, which is different from a heading with nothing under it.
 */
function languagePart(notes, language) {
  const pattern = LANGUAGE_HEADINGS[language];
  const lines = notes.split(/\r?\n/);
  const start = lines.findIndex((line) => pattern.test(line));
  if (start === -1) {
    return null;
  }

  const part = [];
  for (let index = start + 1; index < lines.length; index += 1) {
    if (/^#{2,3}[ \t]+/.test(lines[index])) {
      break;
    }
    part.push(lines[index]);
  }
  return part.join("\n").trim();
}

/** The body for one language, or for both as the changelog wrote it.
 *
 * A section written before the bilingual convention has no language headings; it is returned as it
 * stands for either language, which is what lets an old tag stay rebuildable.
 */
export function releaseNotesInLanguage(notes, language) {
  if (language === "both") {
    return notes.trim();
  }
  const part = languagePart(notes, language);
  if (part !== null) {
    return part;
  }
  return hasLanguageHeadings(notes) ? null : notes.trim();
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
  console.error("Usage: node scripts/release-notes.mjs <version> [--lang both|en|zh] [--check]");
  process.exit(2);
}

async function main() {
  const args = process.argv.slice(2);
  let checkOnly = false;
  let language = "both";
  const versions = [];

  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === "--check") {
      checkOnly = true;
      continue;
    }
    if (argument === "--lang") {
      const value = args[index + 1];
      if (!value || value.startsWith("--")) {
        usage("--lang needs one of both, en or zh.");
      }
      language = value;
      index += 1;
      continue;
    }
    if (argument.startsWith("--lang=")) {
      language = argument.slice("--lang=".length);
      continue;
    }
    if (argument.startsWith("--")) {
      usage(`Unknown option: ${argument}`);
    }
    versions.push(argument);
  }

  if (language !== "both" && language !== "en" && language !== "zh") {
    usage(`Unknown language: ${language}`);
  }
  if (versions.length !== 1) {
    usage(versions.length === 0 ? "Give the version whose notes to print." : "Give exactly one version.");
  }

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

  const english = languagePart(notes, "en");
  const chinese = languagePart(notes, "zh");
  const bilingual = requiresBilingual(version) || hasLanguageHeadings(notes);
  const missingLanguage = english === null || english === "" || chinese === null || chinese === "";

  if (checkOnly) {
    if (bilingual && missingLanguage) {
      console.error(`CHANGELOG.md's ${version} section must carry both "### English" and "### 中文" with content.`);
      console.error(
        "The GitHub body is Chinese and the update notes keep both languages; one alone cannot serve both.",
      );
      process.exit(1);
    }
    console.log(`release notes: ${version} is written in CHANGELOG.md`);
    return;
  }

  if (language === "zh" && requiresBilingual(version) && (chinese === null || chinese === "")) {
    console.error(`CHANGELOG.md has no "### 中文" part for ${version}, so there is no Chinese GitHub body.`);
    console.error("Write the Chinese part before tagging: the GitHub release body is the Chinese section.");
    process.exit(1);
  }

  const selected = releaseNotesInLanguage(notes, language);
  if (selected === null || selected.trim() === "") {
    console.error(`CHANGELOG.md has no ${language === "zh" ? '"### 中文"' : '"### English"'} part for ${version}.`);
    console.error("Write that language part, or print both with --lang both.");
    process.exit(1);
  }

  if (language === "zh" && !bilingual) {
    console.warn(`CHANGELOG.md's ${version} section has no language headings; printing its single-language body.`);
  }
  process.stdout.write(`${selected.trim()}\n`);
}

// Only a direct run reads a changelog and exits: the guard test imports `releaseNotesFor` and must
// not have the command's own argument parsing run underneath it.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}

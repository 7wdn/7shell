// The changelog's release-notes extractor.
//
// `release-notes.mjs` turns one section of CHANGELOG.md into the body of a GitHub release, which is
// the most public thing this repository builds. Two failures there are silent and worth a guard: a
// section that bleeds into the next version's text, and a version with no section that leaves with
// a zero exit code, because either one publishes something that looks like notes and is not.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";
import { releaseNotesFor } from "../release-notes.mjs";

const SCRIPT = path.join(REPO_ROOT, "scripts", "release-notes.mjs");
const CHANGELOG = path.join(REPO_ROOT, "CHANGELOG.md");
const VERSION_FILE = path.join(REPO_ROOT, "version.json");

/**
 * The version this repository is being prepared to release.
 *
 * Read from the single source of truth rather than written here: a literal goes stale on the first
 * bump, and the guard worth having is "the version that is about to be released has notes in the
 * changelog" — the same question the release workflow asks before it builds anything. With a literal
 * this stops asking about anything at all the moment the version moves.
 */
const RELEASED = JSON.parse(readFileSync(VERSION_FILE, "utf8")).app;

/**
 * Two versions, so "the section that was asked for" can be told apart from "the one after it",
 * plus a link definition under the last section — the shape a Keep a Changelog file really has, and
 * the text a section must not swallow.
 */
const FIXTURE = `# Changelog

## [1.0.0] - 2026-02-01

The newer release.

### Added

- the newer change

## [0.9.0] - 2026-01-01

The older release.

### Added

- the older change

[1.0.0]: https://example.invalid/1.0.0
`;

function run(...args) {
  return spawnSync(process.execPath, [SCRIPT, ...args], { encoding: "utf8" });
}

test("a version's notes are its own and stop before the next version", () => {
  const newer = releaseNotesFor(FIXTURE, "1.0.0");
  const older = releaseNotesFor(FIXTURE, "0.9.0");

  assert.ok(newer.includes("the newer change"), "the section asked for was not returned");
  assert.ok(!newer.includes("the older change"), "the notes ran on into the next version's section");
  assert.ok(older.includes("the older change"), "the section asked for was not returned");
  assert.ok(!older.includes("the newer change"), "the notes ran on into the previous version's section");
  assert.ok(!older.includes("https://example.invalid"), "text below the last section is not part of any version");
});

test("a version with no section has no notes", () => {
  assert.equal(releaseNotesFor(FIXTURE, "2.0.0"), null);
  assert.equal(releaseNotesFor(FIXTURE, "Unreleased"), null);
});

test("a leading v belongs to the tag and is accepted", () => {
  const tagged = releaseNotesFor(FIXTURE, "v0.9.0");

  assert.equal(tagged, releaseNotesFor(FIXTURE, "0.9.0"));
  // Equal alone would hold for two `null`s, so the v form has to find the section, not merely
  // agree with the other spelling about finding nothing.
  assert.ok(tagged.includes("the older change"), "the v was not taken off the version");
});

test("the command prints the section without its version heading", () => {
  const plain = run(RELEASED);
  const tagged = run(`v${RELEASED}`);

  assert.equal(plain.status, 0, `printing the notes failed: ${plain.stderr}`);
  assert.equal(tagged.status, 0, `the tag spelling failed: ${tagged.stderr}`);
  assert.equal(tagged.stdout, plain.stdout, "a tag's v changed what is printed");
  // Any changelog heading, and not `### Added` in particular: a release that only fixes things has
  // no section of additions, and what this is here for is that the body carries the changelog's own
  // headings rather than its prose alone.
  assert.ok(/^### /m.test(plain.stdout), "the printed body has no changelog section headings");
  assert.ok(!plain.stdout.includes(`## [${RELEASED}]`), "the version heading must stay out of the release body");
});

test("--check fails loudly for a version the changelog does not have", () => {
  const result = run("9.9.9", "--check");

  assert.notEqual(result.status, 0, "a version with no notes exited successfully");
  assert.equal(result.stdout, "", "a failing check still wrote a release body");
  assert.ok(result.stderr.includes("9.9.9"), `the failure does not name the version: ${result.stderr}`);
});

test("--check succeeds for the section this release is cut from", () => {
  const result = run(RELEASED, "--check");

  assert.equal(result.status, 0, `the released version has no notes: ${result.stderr}`);
  assert.ok(readFileSync(CHANGELOG, "utf8").length > 0, "the changelog itself must be readable");
});

test("no version at all is a usage error, not a silent empty body", () => {
  const result = run("--check");

  assert.equal(result.status, 2, "a missing argument must not look like a version with no notes");
  assert.equal(result.stdout, "", "a usage error wrote to stdout");
});

// The update manifest, which is the file that decides what an installed build downloads next.
//
// Two things are worth a test rather than a careful read. Which key an archive is published under —
// `windows-x86_64-nsis` against `windows-x86_64` — is the difference between an update that is
// offered and one that is never found, because the application looks itself up by those strings. And
// the failure paths matter more than the success path here: a manifest assembled from two different
// releases, or from an archive nobody signed, is worse than no manifest at all, since the only way
// to notice is a user installing the wrong thing.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { ManifestError, manifestKeys, mergeManifest, platformEntries } from "../update-manifest.mjs";

const SCRIPT = fileURLToPath(new URL("../update-manifest.mjs", import.meta.url));

/** Runs the script the way the workflow does, and fails with its own words when it refuses. */
function run(args) {
  const result = spawnSync(process.execPath, [SCRIPT, ...args], { encoding: "utf8" });
  assert.equal(result.status, 0, `update-manifest.mjs ${args.join(" ")} failed: ${result.stderr}`);
}

/** A directory that holds a pretend build's output. */
async function bundle() {
  return mkdtemp(path.join(tmpdir(), "updater-bundle-"));
}

async function writeArtifact(directory, name, signature) {
  await mkdir(directory, { recursive: true });
  await writeFile(path.join(directory, name), "bytes", "utf8");
  if (signature !== null) {
    await writeFile(path.join(directory, `${name}.sig`), signature, "utf8");
  }
}

const TAG = "v0.1.0-rc.1";

test("a Windows build publishes both installers, and the plain key names the recommended one", async () => {
  const root = await bundle();
  // The names and the signature placement a real build produced on 2026-10-12: the `.sig` is attached
  // to the installer itself rather than to a packed copy of it.
  await writeArtifact(path.join(root, "nsis"), "7Shell_0.1.0-rc.1_x64-setup.exe", "nsis-signature");
  await writeArtifact(path.join(root, "msi"), "7Shell_0.1.0-rc.1_x64_en-US.msi", "msi-signature");

  const entry = await platformEntries({
    bundleDir: root,
    platform: "windows-x86_64",
    tag: TAG,
    repository: "7wdn/7shell",
  });
  const keys = manifestKeys(entry);

  assert.deepEqual(Object.keys(keys).sort(), ["windows-x86_64", "windows-x86_64-msi", "windows-x86_64-nsis"]);
  assert.equal(
    keys["windows-x86_64"].url,
    `https://github.com/7wdn/7shell/releases/download/${TAG}/7Shell_0.1.0-rc.1_x64-setup.exe`,
    "the plain key points at the installer Tauri recommends for updates",
  );
  assert.equal(keys["windows-x86_64-nsis"].signature, "nsis-signature");
  assert.equal(keys["windows-x86_64-msi"].signature, "msi-signature");
});

test("an installer with no signature beside it is not published", async () => {
  const root = await bundle();
  // A build with no signing key writes the installers and nothing else: a release people can download
  // by hand, and not one the update channel may point at.
  await writeArtifact(path.join(root, "nsis"), "7Shell_0.1.0-rc.1_x64-setup.exe", null);
  await writeArtifact(path.join(root, "msi"), "7Shell_0.1.0-rc.1_x64_en-US.msi", "signature");

  const entry = await platformEntries({
    bundleDir: root,
    platform: "windows-x86_64",
    tag: TAG,
    repository: "7wdn/7shell",
  });

  assert.equal(entry.entries.length, 1);
  assert.equal(entry.entries[0].installer, "msi");
});

test("a build with nothing signed stops the release instead of publishing nothing", async () => {
  const root = await bundle();
  await writeArtifact(path.join(root, "nsis"), "7Shell_0.1.0-rc.1_x64-setup.exe", null);

  await assert.rejects(
    () => platformEntries({ bundleDir: root, platform: "windows-x86_64", tag: TAG, repository: "7wdn/7shell" }),
    ManifestError,
    "a release whose update archives were never signed has no update channel, and saying so beats an empty manifest",
  );
});

test("an archive whose name this build cannot place stops the release", async () => {
  const root = await bundle();
  await writeArtifact(path.join(root, "linux"), "7Shell_0.1.0-rc.1_amd64.something-new.gz", "signature");

  await assert.rejects(
    () => platformEntries({ bundleDir: root, platform: "linux-x86_64", tag: TAG, repository: "7wdn/7shell" }),
    /cannot place/,
    "a bundler that renamed its archives is a decision to make, not a key to guess",
  );
});

test("a macOS build publishes the application archive under its own target", async () => {
  const root = await bundle();
  await writeArtifact(path.join(root, "macos"), "7Shell.app.tar.gz", "mac-signature");

  const entry = await platformEntries({
    bundleDir: root,
    platform: "darwin-aarch64",
    tag: TAG,
    repository: "7wdn/7shell",
  });
  const keys = manifestKeys(entry);

  assert.deepEqual(Object.keys(keys).sort(), ["darwin-aarch64", "darwin-aarch64-app"]);
  assert.equal(keys["darwin-aarch64"].signature, "mac-signature");
});

test("the merged manifest names the release, its notes and every platform built for it", async () => {
  const releases = await mkdtemp(path.join(tmpdir(), "updater-releases-"));
  await writeFile(
    path.join(releases, "windows.json"),
    JSON.stringify({
      platform: "windows-x86_64",
      tag: TAG,
      entries: [{ installer: "nsis", name: "a.nsis.zip", signature: "sig", url: "https://example.invalid/a.nsis.zip" }],
    }),
    "utf8",
  );
  await writeFile(
    path.join(releases, "macos.json"),
    JSON.stringify({
      platform: "darwin-aarch64",
      tag: TAG,
      entries: [
        { installer: "app", name: "a.app.tar.gz", signature: "sig", url: "https://example.invalid/a.app.tar.gz" },
      ],
    }),
    "utf8",
  );

  const manifest = await mergeManifest({
    releasesDir: releases,
    version: "0.1.0-rc.1",
    tag: TAG,
    notes: "what changed",
    date: "2026-10-12T00:00:00Z",
  });

  assert.equal(manifest.version, "0.1.0-rc.1");
  assert.equal(manifest.notes, "what changed");
  assert.equal(manifest.pub_date, "2026-10-12T00:00:00Z");
  assert.deepEqual(Object.keys(manifest.platforms).sort(), [
    "darwin-aarch64",
    "darwin-aarch64-app",
    "windows-x86_64",
    "windows-x86_64-nsis",
  ]);
});

test("a platform file from another tag is refused", async () => {
  const releases = await mkdtemp(path.join(tmpdir(), "updater-releases-"));
  await writeFile(
    path.join(releases, "stale.json"),
    JSON.stringify({
      platform: "windows-x86_64",
      tag: "v0.1.0-rc.2",
      entries: [{ installer: "nsis", name: "a.nsis.zip", signature: "sig", url: "https://example.invalid/a" }],
    }),
    "utf8",
  );

  await assert.rejects(
    () => mergeManifest({ releasesDir: releases, version: "0.1.0-rc.1", tag: TAG, notes: "", date: "d" }),
    /built from v0.1.0-rc.2/,
    "half a manifest pointing at another release is an update nobody can reason about",
  );
});

test("two platform files claiming the same key are refused", async () => {
  const releases = await mkdtemp(path.join(tmpdir(), "updater-releases-"));
  const platform = JSON.stringify({
    platform: "windows-x86_64",
    tag: TAG,
    entries: [{ installer: "nsis", name: "a.nsis.zip", signature: "sig", url: "https://example.invalid/a" }],
  });
  await writeFile(path.join(releases, "one.json"), platform, "utf8");
  await writeFile(path.join(releases, "two.json"), platform, "utf8");

  await assert.rejects(
    () => mergeManifest({ releasesDir: releases, version: "0.1.0-rc.1", tag: TAG, notes: "", date: "d" }),
    /claimed twice/,
  );
});

test("the workflow's own command lines produce files, flag for flag", async () => {
  // Run the way the release workflow runs it rather than as an imported function: a flag renamed
  // between the two is a release step that fails after the build has already been paid for, and the
  // only thing that catches it is the command line itself being exercised.
  const root = await bundle();
  await writeArtifact(path.join(root, "nsis"), "7Shell_0.1.0-rc.1_x64-setup.exe", "signature");

  const releases = await mkdtemp(path.join(tmpdir(), "updater-releases-"));
  const platformFile = path.join(releases, "windows-x86_64.json");
  const body = path.join(await mkdtemp(path.join(tmpdir(), "updater-notes-")), "body.md");
  await writeFile(body, "what changed\n", "utf8");
  const manifestFile = path.join(releases, "latest.json");

  run([
    "platform",
    "--bundle",
    root,
    "--platform",
    "windows-x86_64",
    "--tag",
    TAG,
    "--repository",
    "7wdn/7shell",
    "--out",
    platformFile,
  ]);

  run([
    "merge",
    "--releases",
    releases,
    "--version",
    "0.1.0-rc.1",
    "--tag",
    TAG,
    "--repository",
    "7wdn/7shell",
    "--notes",
    body,
    "--date",
    "2026-10-12T00:00:00Z",
    "--out",
    manifestFile,
  ]);

  const manifest = JSON.parse(await readFile(manifestFile, "utf8"));
  assert.equal(manifest.version, "0.1.0-rc.1");
  assert.equal(manifest.notes, "what changed");
  assert.equal(manifest.pub_date, "2026-10-12T00:00:00Z");
  assert.equal(manifest.platforms["windows-x86_64"].signature, "signature");
  assert.match(manifest.platforms["windows-x86_64"].url, /^https:\/\/github\.com\/7wdn\/7shell\/releases\/download\//);
});

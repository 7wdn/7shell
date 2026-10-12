// The manifests three package managers install from.
//
// What is checked here is the part that cannot be checked after publication: a manifest naming the
// wrong file, the wrong version or a hash that does not match the bytes is a package manager
// installing something nobody verified, and the registries only find out on submission — after the
// release has already told people to use them. So the fixtures below are shaped like a release's own
// file list, and the assertions read the manifest back the way a package manager would.

import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import YAML from "yaml";

import {
  PackagingError,
  hashAssets,
  homebrewCask,
  macOSImages,
  scoopManifest,
  writeManifests,
} from "../packaging-manifests.mjs";

const VERSION = "0.1.0-rc.1";
const TAG = "v0.1.0-rc.1";

/** A release's file list, hashed as if it had just been downloaded. */
function assets() {
  const file = (name, sha256) => ({
    name,
    bytes: 1024,
    sha256,
    url: `https://github.com/7wdn/7shell/releases/download/${TAG}/${name}`,
  });
  return [
    file("7Shell_0.1.0-rc.1_x64-setup.exe", "aa".repeat(32)),
    file("7Shell_0.1.0-rc.1_x64_en-US.msi", "bb".repeat(32)),
    file("7Shell_0.1.0-rc.1_aarch64.dmg", "cc".repeat(32)),
    file("7Shell_0.1.0-rc.1_x64.dmg", "dd".repeat(32)),
    file("7Shell_0.1.0-rc.1_amd64.AppImage", "ee".repeat(32)),
    file("7Shell_0.1.0-rc.1_amd64.deb", "ff".repeat(32)),
  ];
}

test("a candidate version is installable by Scoop and Homebrew, and not by Winget", async () => {
  const output = await mkdtemp(path.join(tmpdir(), "packaging-"));
  const { written, winget } = await writeManifests({
    assets: assets(),
    version: VERSION,
    tag: TAG,
    repository: "7wdn/7shell",
    date: "2026-10-12",
    outputDir: output,
  });

  assert.equal(winget, false, "winget publishes releases, so a candidate is not a version it carries");
  const names = written.map((file) => path.basename(file)).sort();
  assert.deepEqual(names, ["7shell.json", "7shell.rb"]);
});

test("a released version is installable by Winget too, with the installer's own hash", async () => {
  const output = await mkdtemp(path.join(tmpdir(), "packaging-"));
  const { winget } = await writeManifests({
    assets: assets(),
    version: "0.1.0",
    tag: "v0.1.0",
    repository: "7wdn/7shell",
    date: "2026-11-01",
    outputDir: output,
  });
  assert.equal(winget, true);

  const installer = YAML.parse(await readFile(path.join(output, "winget", "SevenShell.7Shell.installer.yaml"), "utf8"));
  const version = YAML.parse(await readFile(path.join(output, "winget", "SevenShell.7Shell.yaml"), "utf8"));
  const locale = YAML.parse(await readFile(path.join(output, "winget", "SevenShell.7Shell.locale.en-US.yaml"), "utf8"));

  assert.equal(version.PackageVersion, "0.1.0");
  assert.equal(installer.PackageVersion, "0.1.0");
  assert.equal(installer.InstallerType, "nullsoft");
  assert.equal(installer.ReleaseDate, "2026-11-01");
  assert.equal(installer.Installers.length, 1);
  assert.equal(installer.Installers[0].Architecture, "x64");
  assert.match(installer.Installers[0].InstallerUrl, /_x64-setup\.exe$/, "the installer a person would download");
  // Winget stores hashes upper case, and a manifest whose hash disagrees with the file is refused by
  // its own validation — which is the last line of defence before a bad package reaches users.
  assert.equal(installer.Installers[0].InstallerSha256, "AA".repeat(32));
  assert.equal(locale.License, "Apache-2.0");
  assert.equal(locale.ManifestType, "defaultLocale");
});

test("the Homebrew cask carries a hash and a URL for each architecture", () => {
  const { "7shell.rb": cask } = homebrewCask({
    version: VERSION,
    tag: TAG,
    repository: "7wdn/7shell",
    assets: assets(),
  });

  assert.match(cask, /on_arm do\n\s+sha256 "cc+\n?\s*"\n\s+url ".*aarch64\.dmg"/);
  assert.match(cask, /on_intel do\n\s+sha256 "dd+"\n\s+url ".*_x64\.dmg"/);
  assert.match(cask, new RegExp(`version "${VERSION.replaceAll(".", "\\.")}"`));
  assert.match(cask, /app "7Shell\.app"/);
});

test("the Scoop manifest installs the installer's own output and hashes what it downloads", () => {
  const { "7shell.json": contents } = scoopManifest({
    version: VERSION,
    tag: TAG,
    repository: "7wdn/7shell",
    assets: assets(),
  });
  const manifest = JSON.parse(contents);

  assert.equal(manifest.version, VERSION);
  assert.equal(
    manifest.architecture["64bit"].url,
    `https://github.com/7wdn/7shell/releases/download/${TAG}/7Shell_0.1.0-rc.1_x64-setup.exe`,
  );
  assert.equal(manifest.architecture["64bit"].hash, "aa".repeat(32));
  // Scoop runs the installer silently and then takes over the executable it placed: shimming the
  // installer itself would be a command called "7shell" that installs the application instead.
  assert.match(manifest.installer.script.join("\n"), /-ArgumentList '\/S'/);
  assert.match(manifest.bin[0][0], /7Shell\.exe$/);
  assert.equal(manifest.checkver.github, "https://github.com/7wdn/7shell");
});

test("a release with no Intel disk image stops instead of writing a cask that half works", () => {
  assert.throws(
    () =>
      macOSImages([
        { name: "7Shell_0.1.0-rc.1_aarch64.dmg", sha256: "cc", url: "https://example.invalid/arm.dmg", bytes: 1 },
      ]),
    PackagingError,
    "a cask whose Intel half names nothing is a cask that fails on the machine it is installed on",
  );
});

test("hashes are computed from the bytes, one entry per file", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "packaging-assets-"));
  await writeFile(path.join(directory, "a.exe"), "first");
  await writeFile(path.join(directory, "b.dmg"), "second");

  const hashed = await hashAssets({ directory, tag: TAG, repository: "7wdn/7shell" });

  assert.deepEqual(
    hashed.map((asset) => asset.name),
    ["a.exe", "b.dmg"],
  );
  // The real SHA-256 of the file's contents: a manifest publishes this number, and a package manager
  // checks the download against it.
  assert.equal(hashed[0].sha256, "a7937b64b8caa58f03721bb6bacf5c78cb235febe0e70b1b84cd99541461a08e");
  assert.equal(hashed[1].sha256, "16367aacb67a4a017c8da8ab95682ccb390863780f7114dda0a0e0c55644c7c4");
  assert.equal(hashed[0].bytes, 5);
  assert.equal(hashed[0].url, `https://github.com/7wdn/7shell/releases/download/${TAG}/a.exe`);
});

test("a release directory with nothing in it stops instead of writing an empty manifest", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "packaging-empty-"));

  await assert.rejects(
    () => hashAssets({ directory, tag: TAG, repository: "7wdn/7shell" }),
    PackagingError,
    "an empty release would publish manifests with no file behind them",
  );
});

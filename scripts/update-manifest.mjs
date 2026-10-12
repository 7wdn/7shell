#!/usr/bin/env node
// Builds the update manifest the application reads, from the artifacts a release produced.
//
//   node scripts/update-manifest.mjs platform \
//     --bundle src-tauri/target/release/bundle --platform windows-x86_64 \
//     --tag v0.1.0-rc.1 --repository 7wdn/7shell --out platform.json
//
//   node scripts/update-manifest.mjs merge \
//     --releases <dir of platform files> --version 0.1.0-rc.1 --tag v0.1.0-rc.1 \
//     --repository 7wdn/7shell --notes release-body.md --date 2026-10-12T00:00:00Z \
//     --out latest.json
//
// Two steps rather than one, because the two halves are known in different places. The platform a
// build produced is known to the job that produced it — it says so on its command line — and the
// file names are whatever the bundler wrote, which is read back here instead of being predicted.
// A single step would have to guess both from a directory listing, and a guess about which archive
// belongs to which machine is a guess that ends with an update installed on the wrong one.
//
// The signature is not checked here. Signing happens in the build, and verification happens on the
// machine that installs; this file only carries the bytes across.

import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

/**
 * What each platform is called in the manifest.
 *
 * The names are the updater's own: `{os}-{arch}`, exactly what a running build looks itself up as.
 * `darwin` rather than `macos` is not a typo here — it is the string the plugin compares, and a
 * manifest that said `macos-aarch64` would be a manifest nothing ever matches.
 */
export const PLATFORMS = ["windows-x86_64", "darwin-aarch64", "darwin-x86_64", "linux-x86_64"];

/**
 * How a bundler's output is told apart, by the end of its file name.
 *
 * The suffixes are the ones the bundles are written with, and they have to be matched exactly: an
 * archive that only looks similar is an archive this script does not know how to name, and the right
 * answer to that is to stop rather than to publish an update channel pointing at it.
 *
 * Windows is the one worth reading twice: Tauri signs the *installers themselves* (the `.exe` and the
 * `.msi`), not a `tar` of them, so the file an update downloads is the same file a person downloads —
 * one artefact with two jobs. macOS needs its `.app` directory packed to be downloadable at all, and
 * Linux signs the AppImage. `.AppImage.tar.gz` is listed as well because that is what the same
 * bundler wrote in earlier versions; whichever one a build produces is the one that ends up signed,
 * and a build that produced both is refused below rather than guessed at.
 */
const INSTALLERS = [
  { installer: "app", suffix: ".app.tar.gz" },
  { installer: "appimage", suffix: ".AppImage.tar.gz" },
  { installer: "appimage", suffix: ".AppImage" },
  { installer: "nsis", suffix: "-setup.exe" },
  { installer: "msi", suffix: ".msi" },
];

/**
 * Which installer the plain platform key points at when a platform has more than one.
 *
 * The plugin looks for `{os}-{arch}-{installer}` first and falls back to `{os}-{arch}`, and the
 * fallback is what a build whose installer kind cannot be detected reads. NSIS is the installer
 * Tauri recommends for updates and the one the release attaches for people who have no preference,
 * so it is what the plain key has to name.
 */
const PREFERRED = ["nsis", "msi", "appimage", "app"];

export class ManifestError extends Error {}

/** Every file in a directory tree, with its path relative to `root`. */
async function walk(root, directory = root) {
  const found = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      found.push(...(await walk(root, full)));
      continue;
    }
    found.push(path.relative(root, full).split(path.sep).join("/"));
  }
  return found;
}

/** The updater's name for an artifact, or `null` when this script cannot name it. */
export function classify(file) {
  const match = INSTALLERS.find((candidate) => file.endsWith(candidate.suffix));
  return match ? match.installer : null;
}

/** The key in the manifest for one platform and one installer kind. */
export function keyFor(platform, installer) {
  return `${platform}-${installer}`;
}

/**
 * Reads one build's own output and says where each updater archive can be downloaded.
 *
 * The pairs are found by their signatures: an updater archive is one that has a `.sig` file beside
 * it, which is what makes "an archive nobody signed" impossible to publish by accident — it simply
 * is not seen.
 */
export async function platformEntries({ bundleDir, platform, tag, repository }) {
  if (!PLATFORMS.includes(platform)) {
    throw new ManifestError(`unknown platform \`${platform}\` (expected one of ${PLATFORMS.join(", ")})`);
  }

  const files = await walk(bundleDir);
  const signatures = files.filter((file) => file.endsWith(".sig"));
  if (signatures.length === 0) {
    throw new ManifestError(
      `${bundleDir} holds no signed update archive: the build produced nothing an update could install`,
    );
  }

  const entries = [];
  for (const signature of signatures) {
    const artifact = signature.slice(0, -".sig".length);
    const installer = classify(artifact);
    if (!installer) {
      throw new ManifestError(
        `${artifact} has a signature but a name this script cannot place among ${INSTALLERS.map((i) => i.suffix).join(", ")}`,
      );
    }
    if (!files.includes(artifact)) {
      throw new ManifestError(`${signature} has no archive beside it, so nothing was signed`);
    }
    entries.push({
      installer,
      name: path.basename(artifact),
      signature: (await readFile(path.join(bundleDir, artifact + ".sig"), "utf8")).trim(),
      url: `https://github.com/${repository}/releases/download/${tag}/${path.basename(artifact)}`,
    });
  }

  // One platform, one build: two archives of the same kind would mean the job built the same thing
  // twice, and the manifest cannot say which of them a person would get.
  const seen = new Set();
  for (const entry of entries) {
    if (seen.has(entry.installer)) {
      throw new ManifestError(`${bundleDir} holds two \`${entry.installer}\` update archives, so neither is the one`);
    }
    seen.add(entry.installer);
  }

  return { platform, tag, entries };
}

/** The manifest a platform file contributes: the plain key, plus one key per installer kind. */
export function manifestKeys(entry) {
  const keys = {};
  const preferred = PREFERRED.find((installer) => entry.entries.some((item) => item.installer === installer));
  if (!preferred) {
    throw new ManifestError(`${entry.platform} has no installer this build knows how to prefer`);
  }

  for (const item of entry.entries) {
    keys[keyFor(entry.platform, item.installer)] = { url: item.url, signature: item.signature };
  }
  const chosen = entry.entries.find((item) => item.installer === preferred);
  keys[entry.platform] = { url: chosen.url, signature: chosen.signature };
  return keys;
}

/**
 * Collects every platform's file into the one manifest the endpoint serves.
 *
 * A platform file that was built from a different tag is refused rather than merged: the manifest
 * names one release, and half of it pointing at another tag is how a person on Windows gets a macOS
 * build's version number.
 */
export async function mergeManifest({ releasesDir, version, tag, notes, date }) {
  const files = (await walk(releasesDir)).filter((file) => file.endsWith(".json"));
  if (files.length === 0) {
    throw new ManifestError(`${releasesDir} holds no platform file, so nothing would be published`);
  }

  const platforms = {};
  for (const file of files.sort()) {
    const entry = JSON.parse(await readFile(path.join(releasesDir, file), "utf8"));
    if (entry.tag !== tag) {
      throw new ManifestError(`${file} was built from ${entry.tag}, and this manifest is for ${tag}`);
    }
    for (const [key, value] of Object.entries(manifestKeys(entry))) {
      if (platforms[key]) {
        throw new ManifestError(`${key} is claimed twice, in ${file} and in another platform file`);
      }
      platforms[key] = value;
    }
  }

  return { version, notes, pub_date: date, platforms };
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!argument.startsWith("--")) {
      throw new ManifestError(`unexpected argument \`${argument}\``);
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new ManifestError(`\`${argument}\` needs a value`);
    }
    options[argument.slice(2)] = value;
    index += 1;
  }
  return options;
}

function required(options, name) {
  if (!options[name]) {
    throw new ManifestError(`\`--${name}\` is required`);
  }
  return options[name];
}

async function write(file, value) {
  await mkdir(path.dirname(file), { recursive: true });
  // Two spaces and a newline: a released manifest is read by people when an update misbehaves, and a
  // single line of JSON is a file nobody can check anything in.
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

async function main() {
  const [mode, ...rest] = process.argv.slice(2);
  const options = parseArguments(rest);

  if (mode === "platform") {
    const entry = await platformEntries({
      bundleDir: required(options, "bundle"),
      platform: required(options, "platform"),
      tag: required(options, "tag"),
      repository: required(options, "repository"),
    });
    await write(required(options, "out"), entry);
    console.log(
      `update manifest: ${entry.platform} contributes ${entry.entries.map((item) => item.installer).join(", ")}`,
    );
    return;
  }

  if (mode === "merge") {
    const notes = options.notes ? await readFile(options.notes, "utf8") : "";
    const manifest = await mergeManifest({
      releasesDir: required(options, "releases"),
      version: required(options, "version"),
      tag: required(options, "tag"),
      notes: notes.trim(),
      date: required(options, "date"),
    });
    await write(required(options, "out"), manifest);
    console.log(`update manifest: ${Object.keys(manifest.platforms).sort().join(", ")}`);
    return;
  }

  throw new ManifestError("usage: update-manifest.mjs platform|merge [--flag value ...]");
}

// Only a direct run parses arguments: the guard test imports the functions above and must not have
// the command's own `main` run underneath it, where a usage error would be the test's first failure.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}

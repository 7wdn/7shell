#!/usr/bin/env node
// Turns a release's own files into the manifests three package managers install from.
//
//   node scripts/packaging-manifests.mjs assets \
//     --dir <downloaded artifacts> --tag v0.1.0-rc.1 --repository 7wdn/7shell --out assets.json
//
//   node scripts/packaging-manifests.mjs manifests \
//     --assets assets.json --version 0.1.0-rc.1 --tag v0.1.0-rc.1 \
//     --repository 7wdn/7shell --date 2026-10-12 --out packaging/out
//
// Two steps, and the first one is why this is not a template with holes in it: the hash of a file
// that was actually built is the one thing a manifest cannot be written without, and a hash copied
// from somewhere else is a package manager installing a file nobody verified. So the files are
// hashed here, from the release's own downloads, and every manifest is written from that list.
//
// What this does *not* do is submit anything. Winget, Scoop and Homebrew each take a manifest
// through their own review — a pull request against a registry this project does not own — and that
// step needs credentials only a maintainer has. `packaging/README.md` says what to run for each.

import { createHash } from "node:crypto";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

/**
 * The repository root, worked out here rather than imported from `lib/descriptors.mjs`.
 *
 * The release workflow runs this script with nothing but Node: no `pnpm install`, no development
 * dependencies, because the job that publishes a release should need nothing that is not already on
 * the runner. That other file reads YAML, so importing it for this one string made publishing depend
 * on `node_modules` existing — invisible on a machine where the repository has been installed, and
 * fatal on the runner. `scripts/tests/releaseTooling.test.mjs` keeps the three scripts that job runs
 * free of anything but `node:`.
 */
const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** The package's own identity, as the registries spell it. */
const PACKAGE_ID = "SevenShell.7Shell";
const CASK_NAME = "7shell";
const PRODUCT = "7Shell";
const LICENSE = "Apache-2.0";
const DESCRIPTION =
  "Open-source SSH and operations console: an encrypted vault, host keys verified before anything is sent, terminals, a file workbench, jump hosts and a host monitor.";

export class PackagingError extends Error {}

function require_(value, what) {
  if (!value) {
    throw new PackagingError(`${what} is missing from the release's files`);
  }
  return value;
}

/**
 * The Windows installer a package manager should install.
 *
 * The NSIS installer is the one both Scoop and Winget are given: it is a per-user install with a
 * silent switch, which is what both of them need, while the MSI is the one administrators deploy.
 */
export function windowsInstaller(assets) {
  return require_(
    assets.find((asset) => asset.name.endsWith(".exe")) ?? assets.find((asset) => asset.name.endsWith(".msi")),
    "a Windows installer (.exe or .msi)",
  );
}

/** The macOS disk images, one per architecture. */
export function macOSImages(assets) {
  const images = assets.filter((asset) => asset.name.endsWith(".dmg"));
  return {
    arm: require_(
      images.find((asset) => /aarch64|arm64/.test(asset.name)),
      "an Apple-silicon disk image (*aarch64*.dmg or *arm64*.dmg)",
    ),
    intel: require_(
      images.find((asset) => /(x64|x86_64|amd64)/.test(asset.name)),
      "an Intel disk image (*x64*.dmg)",
    ),
  };
}

/** Every file in a directory, one level deep, with its size and its real hash. */
export async function hashAssets({ directory, tag, repository }) {
  const assets = [];
  for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) =>
    a.name.localeCompare(b.name),
  )) {
    if (!entry.isFile()) {
      continue;
    }
    const bytes = await readFile(path.join(directory, entry.name));
    assets.push({
      name: entry.name,
      bytes: bytes.byteLength,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      url: `https://github.com/${repository}/releases/download/${tag}/${entry.name}`,
    });
  }
  if (assets.length === 0) {
    throw new PackagingError(`${directory} holds no file, so there is nothing to write a manifest about`);
  }
  return assets;
}

function yamlString(value) {
  // Quoted always: a description with a colon in it is a description that turns a manifest into
  // something else, and there is no value here worth the risk of writing unquoted.
  return `"${String(value).replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

export function wingetManifests({ version, tag, repository, date, assets }) {
  const installer = windowsInstaller(assets);
  const url = `https://github.com/${repository}`;

  return {
    [`${PACKAGE_ID}.yaml`]: [
      `PackageIdentifier: ${PACKAGE_ID}`,
      `PackageVersion: ${version}`,
      "DefaultLocale: en-US",
      "ManifestType: version",
      "ManifestVersion: 1.6.0",
      "",
    ].join("\n"),
    [`${PACKAGE_ID}.locale.en-US.yaml`]: [
      `PackageIdentifier: ${PACKAGE_ID}`,
      `PackageVersion: ${version}`,
      "PackageLocale: en-US",
      `Publisher: ${yamlString("SevenShell contributors")}`,
      `PublisherUrl: ${url}`,
      `PackageName: ${yamlString(PRODUCT)}`,
      `PackageUrl: ${url}`,
      `License: ${LICENSE}`,
      `LicenseUrl: ${url}/blob/main/LICENSE`,
      `ShortDescription: ${yamlString("Open-source SSH and operations console")}`,
      `Description: ${yamlString(DESCRIPTION)}`,
      `Moniker: ${yamlString("7shell")}`,
      "Tags:",
      ...["ssh", "terminal", "sftp", "remote", "sessions"].map((tagName) => `  - ${tagName}`),
      `ReleaseNotesUrl: ${url}/releases/tag/${tag}`,
      "ManifestType: defaultLocale",
      "ManifestVersion: 1.6.0",
      "",
    ].join("\n"),
    [`${PACKAGE_ID}.installer.yaml`]: [
      `PackageIdentifier: ${PACKAGE_ID}`,
      `PackageVersion: ${version}`,
      // The NSIS installer Tauri writes: silent with /S, per-user by default, and it upgrades in
      // place rather than installing beside itself.
      "InstallerType: nullsoft",
      "Scope: user",
      "InstallModes:",
      "  - interactive",
      "  - silent",
      "  - silentWithProgress",
      "UpgradeBehavior: install",
      `ReleaseDate: ${date}`,
      "Installers:",
      "  - Architecture: x64",
      `    InstallerUrl: ${installer.url}`,
      `    InstallerSha256: ${installer.sha256.toUpperCase()}`,
      "ManifestType: installer",
      "ManifestVersion: 1.6.0",
      "",
    ].join("\n"),
  };
}

export function scoopManifest({ version, tag, repository, assets }) {
  const installer = windowsInstaller(assets);
  const url = `https://github.com/${repository}`;
  const manifest = {
    version,
    description: DESCRIPTION,
    homepage: url,
    license: LICENSE,
    architecture: {
      // Scoop installs the installer's own output and shims the executable it placed, which is the
      // same thing a person downloading the installer gets — not a private unpacking of it.
      "64bit": {
        url: installer.url,
        hash: installer.sha256,
      },
    },
    installer: {
      script: [`Start-Process -FilePath "$dir\\$fname" -ArgumentList '/S' -Wait`, `Remove-Item "$dir\\$fname"`],
    },
    bin: [["$env:LOCALAPPDATA\\7Shell\\7Shell.exe", "7shell"]],
    checkver: { github: `https://github.com/${repository}`, regex: `/releases/tag/v([\\d.]+(?:-[\\w.]+)?)` },
    autoupdate: { architecture: { "64bit": { url: `${url}/releases/download/v$version/${installer.name}` } } },
  };
  return { [`${CASK_NAME}.json`]: `${JSON.stringify(manifest, null, 2)}\n` };
}

export function homebrewCask({ version, tag, repository, assets }) {
  const { arm, intel } = macOSImages(assets);
  const url = `https://github.com/${repository}`;
  const lines = [
    `cask "${CASK_NAME}" do`,
    `  version "${version}"`,
    "",
    "  on_arm do",
    `    sha256 "${arm.sha256}"`,
    `    url "${arm.url}"`,
    "  end",
    "  on_intel do",
    `    sha256 "${intel.sha256}"`,
    `    url "${intel.url}"`,
    "  end",
    "",
    `  name "${PRODUCT}"`,
    `  desc "Open-source SSH and operations console"`,
    `  homepage "${url}"`,
    "",
    "  # The disk image holds the application beside a shortcut to /Applications; the cask install and",
    "  # the drag-install a person does by hand are the same thing.",
    `  app "${PRODUCT}.app"`,
    "end",
    "",
  ];
  return { [`${CASK_NAME}.rb`]: lines.join("\n") };
}

/**
 * Writes every manifest a release can be installed by.
 *
 * Winget is written only for a version winget's own pipeline can carry: it publishes releases rather
 * than candidates, and a version with a prerelease label on it is a version it rejects on the way in.
 * The other two carry one happily, which is what makes a candidate installable at all before the
 * first stable release exists.
 */
export async function writeManifests({ assets, version, tag, repository, date, outputDir }) {
  const written = [];
  const stable = !version.includes("-");

  if (stable) {
    for (const [name, contents] of Object.entries(wingetManifests({ version, tag, repository, date, assets }))) {
      const file = path.join(outputDir, "winget", name);
      await mkdir(path.dirname(file), { recursive: true });
      await writeFile(file, contents, "utf8");
      written.push(file);
    }
  }

  for (const [directory, generated] of [
    ["scoop", scoopManifest({ version, tag, repository, assets })],
    ["homebrew", homebrewCask({ version, tag, repository, assets })],
  ]) {
    for (const [name, contents] of Object.entries(generated)) {
      const file = path.join(outputDir, directory, name);
      await mkdir(path.dirname(file), { recursive: true });
      await writeFile(file, contents, "utf8");
      written.push(file);
    }
  }

  return { written, winget: stable };
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!argument.startsWith("--")) {
      throw new PackagingError(`unexpected argument \`${argument}\``);
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new PackagingError(`\`${argument}\` needs a value`);
    }
    options[argument.slice(2)] = value;
    index += 1;
  }
  return options;
}

function required(options, name) {
  if (!options[name]) {
    throw new PackagingError(`\`--${name}\` is required`);
  }
  return options[name];
}

async function main() {
  const [mode, ...rest] = process.argv.slice(2);
  const options = parseArguments(rest);

  if (mode === "assets") {
    const assets = await hashAssets({
      directory: required(options, "dir"),
      tag: required(options, "tag"),
      repository: required(options, "repository"),
    });
    const out = path.resolve(REPO_ROOT, required(options, "out"));
    await mkdir(path.dirname(out), { recursive: true });
    await writeFile(out, `${JSON.stringify(assets, null, 2)}\n`, "utf8");
    console.log(`packaging: hashed ${assets.length} release file(s)`);
    return;
  }

  if (mode === "manifests") {
    const assets = JSON.parse(await readFile(path.resolve(REPO_ROOT, required(options, "assets")), "utf8"));
    const { written, winget } = await writeManifests({
      assets,
      version: required(options, "version"),
      tag: required(options, "tag"),
      repository: required(options, "repository"),
      date: required(options, "date"),
      outputDir: path.resolve(REPO_ROOT, required(options, "out")),
    });
    for (const file of written) {
      console.log(`packaging: wrote ${path.relative(REPO_ROOT, file).split(path.sep).join("/")}`);
    }
    if (!winget) {
      console.log("packaging: winget is written for a release, not for a candidate; nothing was written for it");
    }
    return;
  }

  throw new PackagingError("usage: packaging-manifests.mjs assets|manifests [--flag value ...]");
}

// Only a direct run reads arguments: the guard test imports the writers above and must not have the
// command's own `main` run underneath it.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}

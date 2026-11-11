#!/usr/bin/env node
// Verifies that a Windows build actually ships the MCP bridge next to the application.
//
// `tauri.conf.json` declares it as an `externalBin`; the bundler is what copies it. A green
// `cargo check` says nothing about that step, so this script starts from the files a release left
// behind: the release bridge, the generated NSIS script, the generated WiX source the MSI is built
// from, and — on Windows — an administrative install of the MSI into a temporary directory.
//
// The MSI's WiX puts the bridge in the optional `Environment` feature (the same feature that
// carries PATH). An administrative install (`msiexec /a`) extracts only the features it is asked
// for, so this script asks for `ADDLOCAL=Environment` and then compares the file that lands on disk
// with the staged binary. The generated WiX is checked as well: it is what a build without a
// Windows host can still inspect.
//
// Run it after `pnpm tauri build --bundles nsis,msi`. Node built-ins only, so it also works in the
// release job's bare environment.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BINARY = process.platform === "win32" ? "sshell-mcp.exe" : "sshell-mcp";

function fail(message) {
  console.error(`installer-sidecar: ${message}`);
  process.exit(1);
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function stagedSidecar() {
  const directory = path.join(ROOT, "src-tauri", "binaries");
  if (!existsSync(directory)) {
    fail("src-tauri/binaries is missing; run `pnpm build:sidecar` first");
  }
  const candidates = readdirSync(directory).filter((name) => name.startsWith("sshell-mcp-"));
  if (candidates.length !== 1) {
    fail(`expected one staged bridge, found ${candidates.length}; run \`pnpm build:sidecar\``);
  }
  return path.join(directory, candidates[0]);
}

function checkReleaseBinary(staged) {
  const release = path.join(ROOT, "target", "release", BINARY);
  if (!existsSync(release)) {
    fail("target/release has no bridge; run `pnpm tauri build --bundles nsis,msi`");
  }
  if (sha256(release) !== sha256(staged)) {
    fail("the release bridge differs from the staged one; run `pnpm tauri build` again");
  }
  return sha256(release);
}

function checkNsis() {
  const script = path.join(ROOT, "target", "release", "nsis", "x64", "installer.nsi");
  if (!existsSync(script)) {
    return "skipped (no NSIS script)";
  }
  const text = readFileSync(script, "utf8");
  if (!text.includes("/oname=sshell-mcp.exe")) {
    fail("the generated NSIS script does not install the bridge as sshell-mcp.exe");
  }
  return "ok (File /oname=sshell-mcp.exe)";
}

function checkMsi(stagedHash) {
  const wix = path.join(ROOT, "target", "release", "wix", "x64", "main.wxs");
  if (!existsSync(wix)) {
    return "skipped (no WiX source)";
  }
  const text = readFileSync(wix, "utf8");
  if (!/Id="sshell_mcp\.exe"/.test(text)) {
    fail("the generated WiX has no sidecar component");
  }
  if (!/ComponentRef Id="sshell_mcp\.exe"/.test(text)) {
    fail("the generated WiX does not reference the sidecar component");
  }
  if (process.platform !== "win32") {
    return "ok (WiX component + ComponentRef; extraction is Windows-only)";
  }

  const bundle = path.join(ROOT, "target", "release", "bundle", "msi");
  // Old builds keep their installers in the same directory; naming the version is what keeps this
  // check about the package this tree would ship rather than whichever file the directory lists
  // first.
  const version = JSON.parse(readFileSync(path.join(ROOT, "src-tauri", "tauri.conf.json"), "utf8")).version;
  const installer = existsSync(bundle)
    ? readdirSync(bundle).find((name) => name.toLowerCase().endsWith(".msi") && name.includes(`_${version}_`))
    : undefined;
  if (!installer) {
    fail(`no MSI for version ${version}; run \`pnpm tauri build --bundles msi\``);
  }

  const directory = mkdtempSync(path.join(os.tmpdir(), "7shell-msi-"));
  try {
    const result = spawnSync(
      "msiexec.exe",
      ["/a", path.join(bundle, installer), "/qn", `TARGETDIR=${directory}`, "ADDLOCAL=Environment"],
      { encoding: "utf8", windowsHide: true },
    );
    if (result.status !== 0) {
      fail(`msiexec could not lay the MSI down for inspection (exit ${result.status})`);
    }

    const extracted = findFile(directory, BINARY);
    if (!extracted) {
      fail(`the administrative install left no ${BINARY} behind`);
    }
    const hash = sha256(extracted);
    if (hash !== stagedHash) {
      fail("the sidecar inside the MSI differs from the staged bridge");
    }
    return `ok (WiX component + ComponentRef; administrative install carries it, sha256 ${hash.slice(0, 12)}…)`;
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

function findFile(directory, name) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      const found = findFile(full, name);
      if (found) {
        return found;
      }
    } else if (entry.name.toLowerCase() === name.toLowerCase()) {
      return full;
    }
  }
  return undefined;
}

const staged = stagedSidecar();
const hash = checkReleaseBinary(staged);
console.log(`installer-sidecar: release bridge sha256 ${hash}`);
console.log(`installer-sidecar: staged bridge ${path.relative(ROOT, staged)}`);
console.log(`installer-sidecar: NSIS ${checkNsis()}`);
console.log(`installer-sidecar: MSI ${checkMsi(hash)}`);

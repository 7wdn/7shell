#!/usr/bin/env node
// Verifies that a Windows build actually ships the MCP bridge next to the application.
//
// `tauri.conf.json` declares it as an `externalBin`; the bundler is what copies it. A green
// `cargo check` says nothing about that step, so this script starts from the files a release left
// behind: the release bridge, the generated NSIS script, and the generated WiX source the MSI is
// built from.
//
// The MSI's WiX puts the bridge in an optional feature (the same feature that carries PATH), so an
// administrative install (`msiexec /a`) does not extract it unless every feature is selected. The
// generated WiX is the authoritative statement that the component is part of the MSI; the NSIS
// script names the installed file `sshell-mcp.exe` explicitly.
//
// Run it after `pnpm tauri build --bundles nsis,msi`. Node built-ins only, so it also works in the
// release job's bare environment.

import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
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

function checkMsi() {
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
  return "ok (WiX component + ComponentRef)";
}

const staged = stagedSidecar();
const hash = checkReleaseBinary(staged);
console.log(`installer-sidecar: release bridge sha256 ${hash}`);
console.log(`installer-sidecar: staged bridge ${path.relative(ROOT, staged)}`);
console.log(`installer-sidecar: NSIS ${checkNsis()}`);
console.log(`installer-sidecar: MSI ${checkMsi()}`);

#!/usr/bin/env node
// Builds the MCP bridge and puts the seed where Tauri's `externalBin` expects it.
//
// Tauri looks for `src-tauri/binaries/sshell-mcp-seed-<target-triple>(.exe)` and copies it beside
// the application in the bundle. That copy is a seed, not the program a client starts: the host
// copies it to the machine-local bridge directory at startup, so an application update can replace
// the seed while an MCP client is running the other copy. The bridge is a second Rust binary in
// this workspace, so it has to be built first; doing it here keeps the release command one command.
//
// Node built-ins only: the release job runs this before `pnpm install` has necessarily provided
// anything, and a build step that needs a package is a build step that fails on a clean runner.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const debug = process.argv.includes("--debug");
const profile = debug ? "debug" : "release";
const binary = process.platform === "win32" ? "sshell-mcp.exe" : "sshell-mcp";

function hostTriple() {
  const output = execFileSync("rustc", ["-vV"], { cwd: ROOT, encoding: "utf8" });
  const match = /^host:\s*(\S+)$/m.exec(output);
  if (!match) {
    throw new Error("rustc -vV did not name a host target");
  }
  return match[1];
}

const host = hostTriple();
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;

const build = ["build", "-p", "sevenshell-mcp"];
if (profile === "release") {
  build.push("--release");
}
if (triple !== host) {
  build.push("--target", triple);
}
execFileSync("cargo", build, { cwd: ROOT, stdio: "inherit" });

const targetDirectory = process.env.CARGO_TARGET_DIR
  ? path.resolve(ROOT, process.env.CARGO_TARGET_DIR)
  : path.join(ROOT, "target");
const built = path.join(targetDirectory, triple === host ? "" : triple, profile, binary);
const directory = path.join(ROOT, "src-tauri", "binaries");
const destination = path.join(directory, `sshell-mcp-seed-${triple}${path.extname(binary)}`);
// A build from before the seed existed leaves the old name behind; `externalBin` no longer names it
// and a second staged bridge would make the installer check ambiguous.
const legacy = path.join(directory, `sshell-mcp-${triple}${path.extname(binary)}`);

mkdirSync(directory, { recursive: true });
copyFileSync(built, destination);
try {
  rmSync(legacy, { force: true });
} catch {
  // The old name only matters to this checkout; a file that cannot be removed is not a build error.
}
console.log(`mcp sidecar: ${path.relative(ROOT, built)} -> ${path.relative(ROOT, destination)}`);

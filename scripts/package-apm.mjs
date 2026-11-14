#!/usr/bin/env node
// Turns the ordinary Linux deb into an APM package and checks what the converter produced.
//
// APM packages run a deb inside a maintained base environment. The base is what supplies glibc,
// WebKitGTK and OpenSSL at versions newer than UOS 20 or Kylin V10 carry, so the desktop does not
// need a second build against glibc 2.28 — it needs this wrapper and a bridge the host can start.
// The bridge seed is replaced before conversion: see build-legacy-seed.mjs.
//
// Runs only on Linux with APM and the base environment already installed, because
// `amber-pm-convert` uses both (it installs the deb into an overlay of the base, then copies the
// changed files into the package). The Node part is reading back what it produced.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BASE = "amber-pm-bookworm";
const PACKAGE = "7shell";
const args = process.argv.slice(2);

function value(name) {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : undefined;
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

function run(command, commandArgs, options = {}) {
  const result = spawnSync(command, commandArgs, { encoding: "utf8", ...options });
  if (result.error) {
    fail(`${command} could not be started: ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(`${command} ${commandArgs.join(" ")} failed (status ${result.status})\n${result.stderr ?? ""}`);
  }
  return result.stdout;
}

function findAllFiles(directory, name) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      found.push(...findAllFiles(full, name));
    } else if (entry.name === name) {
      found.push(full);
    }
  }
  return found;
}

function findNamed(directory, predicate) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      const nested = findNamed(full, predicate);
      if (nested) return nested;
    } else if (predicate(entry.name)) {
      return full;
    }
  }
  return null;
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

const deb = path.resolve(value("deb") ?? "");
const seed = path.resolve(value("seed") ?? "");
const version = value("version");
const out = path.resolve(value("out") ?? path.join(ROOT, "dist-apm"));
if (!value("deb") || !existsSync(deb)) fail("--deb must name the Linux deb built by the release job");
if (!value("seed") || !existsSync(seed)) fail("--seed must name the legacy MCP bridge built by build-legacy-seed.mjs");
if (!version) fail("--version is required");
if (process.getuid?.() !== 0) {
  console.warn("warning: not running as root; amber-pm-convert calls sudo and may ask for a password");
}

for (const command of ["dpkg-deb", "amber-pm-convert"]) {
  const found = spawnSync("sh", ["-c", `command -v ${command}`], { encoding: "utf8" });
  if (found.status !== 0) fail(`${command} is required on PATH`);
}
const baseEnvironment = `/var/lib/apm/apm/files/ace-env/var/lib/apm/${BASE}/files/ace-env`;
if (!existsSync(baseEnvironment)) {
  fail(
    `${BASE} is not installed; install the APM base package first ` +
      `(https://d.spark-app.store/${process.arch === "arm64" ? "arm64" : "amd64"}-apm/)`,
  );
}

const work = mkdtempSync(path.join(tmpdir(), "7shell-apm-"));
const keep = args.includes("--keep");
try {
  const root = path.join(work, "root");
  run("dpkg-deb", ["-R", deb, root]);
  const packagedSeed = path.join(root, "usr", "bin", "sshell-mcp-seed");
  if (!existsSync(packagedSeed)) fail(`the deb has no ${packagedSeed}`);
  copyFileSync(seed, packagedSeed);
  chmodSync(packagedSeed, 0o755);

  const patched = path.join(work, "patched.deb");
  run("dpkg-deb", ["--root-owner-group", "-b", root, patched]);

  const converted = spawnSync(
    "amber-pm-convert",
    ["--base", BASE, "--pkgname", PACKAGE, "--version", version, patched],
    { cwd: work, stdio: "inherit" },
  );
  if (converted.status !== 0) {
    fail(`amber-pm-convert failed (status ${converted.status}); the work directory is ${work}`);
  }

  const arch = run("dpkg-deb", ["-f", patched, "Architecture"]).trim();
  const produced = path.join(work, `${PACKAGE}_${version}_${arch}.deb`);
  if (!existsSync(produced)) {
    fail(`amber-pm-convert did not produce ${produced}; the work directory is ${work}`);
  }

  const field = (name) => run("dpkg-deb", ["-f", produced, name]).trim();
  if (field("Package") !== PACKAGE) fail(`the APM package is named ${field("Package")}, not ${PACKAGE}`);
  const depends = field("Depends");
  if (
    !depends
      .split(",")
      .map((part) => part.trim())
      .includes(BASE)
  ) {
    fail(`the APM package does not depend on ${BASE}: ${depends}`);
  }

  const verify = path.join(work, "verify");
  run("dpkg-deb", ["-x", produced, verify]);
  const payload = path.join(verify, "var", "lib", "apm", PACKAGE, "files");
  if (!existsSync(payload)) fail(`the APM package has no ${payload}`);
  const shippedSeeds = findAllFiles(payload, "sshell-mcp-seed");
  if (shippedSeeds.length === 0) fail("the APM package does not carry sshell-mcp-seed");
  const legacyHash = sha256(seed);
  for (const shippedSeed of shippedSeeds) {
    if (sha256(shippedSeed) !== legacyHash) {
      fail(`the APM package carries a different seed at ${shippedSeed}`);
    }
  }

  // The tray loads this with dlopen and panics at startup when it is absent; the base does not
  // carry it, so the APM package has to — which is what the deb's new dependency makes the converter
  // install into the payload. Checking for it here keeps a dropped dependency from becoming a
  // package that installs and then dies before the window appears.
  if (findAllFiles(payload, "libayatana-appindicator3.so.1").length === 0) {
    fail("the APM package does not carry libayatana-appindicator3; the tray would panic at startup");
  }

  const entries = path.join(verify, "var", "lib", "apm", PACKAGE, "entries");
  const desktop = existsSync(entries) ? findNamed(entries, (name) => name.endsWith(".desktop")) : null;
  if (!desktop) fail("the APM package has no desktop entry");
  if (!new RegExp(`^Exec=apm run ${PACKAGE} `, "m").test(readFileSync(desktop, "utf8"))) {
    fail(`the desktop entry is not started through APM: ${desktop}`);
  }

  mkdirSync(out, { recursive: true });
  const asset = path.join(out, `7Shell_${version}_${arch}-apm.deb`);
  copyFileSync(produced, asset);
  const size = (statSync(asset).size / (1024 * 1024)).toFixed(1);
  console.log(`APM package: ${asset} (${size} MiB, depends ${depends})`);
  if (keep) console.log(`work directory kept: ${work}`);
} finally {
  if (!keep) rmSync(work, { recursive: true, force: true });
}

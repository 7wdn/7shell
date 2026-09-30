#!/usr/bin/env node
// Keeps every released version number derived from version.json.
//
//   node scripts/reconcile-versions.mjs           write the manifests
//   node scripts/reconcile-versions.mjs --check    fail if any manifest drifts
//
// A version that has to be edited in four files by hand is a version that will disagree
// with itself in a release build.

import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

import { REPO_ROOT } from "./lib/descriptors.mjs";
import { replaceJsonVersion, replaceWorkspaceVersion } from "./lib/versions.mjs";

const VERSION_FILE = path.join(REPO_ROOT, "version.json");
const checkOnly = process.argv.includes("--check");

function label(file) {
  return path.relative(REPO_ROOT, file).split(path.sep).join("/");
}

async function main() {
  const versionSource = await readFile(VERSION_FILE, "utf8");
  const version = JSON.parse(versionSource).app;
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+/.test(version)) {
    throw new Error(`version.json has no usable "app" version (found ${JSON.stringify(version)})`);
  }

  const targets = [
    { file: path.join(REPO_ROOT, "package.json"), apply: replaceJsonVersion },
    { file: path.join(REPO_ROOT, "src-tauri", "tauri.conf.json"), apply: replaceJsonVersion },
    { file: path.join(REPO_ROOT, "Cargo.toml"), apply: replaceWorkspaceVersion },
  ];

  const drifted = [];
  for (const target of targets) {
    const current = await readFile(target.file, "utf8");
    const expected = target.apply(current, version);
    if (current === expected) {
      continue;
    }
    if (checkOnly) {
      drifted.push(target.file);
    } else {
      await writeFile(target.file, expected, "utf8");
      console.log(`updated ${label(target.file)} -> ${version}`);
    }
  }

  if (drifted.length > 0) {
    console.error(`These manifests disagree with version.json (${version}):`);
    for (const file of drifted) {
      console.error(`  - ${label(file)}`);
    }
    console.error("Run `pnpm sync:versions` and commit the result.");
    process.exit(1);
  }

  console.log(checkOnly ? `versions: all manifests agree on ${version}` : `versions: ${version}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});

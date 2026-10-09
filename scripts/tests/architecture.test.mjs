// Dependency direction across the Rust workspace.
//
// A crate may only depend on crates with a lower rank, and only hosts may depend on
// Tauri. This is the rule that keeps the operations callable from a future command line
// or agent interface: the moment `sevenshell-core` learns about a webview, that door
// closes. Rank a new crate deliberately and add it here.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

/** Lower runs first; a dependency must always point downwards. */
const RANKS = new Map([
  ["sevenshell-error", 0],
  ["sevenshell-types", 0],
  ["sevenshell-platform", 1],
  ["sevenshell-profiles", 2],
  ["sevenshell-vault", 2],
  ["sevenshell-policy", 2],
  ["sevenshell-ssh", 3],
  ["sevenshell-sync", 3],
  ["sevenshell-session", 4],
  ["sevenshell-sftp", 4],
  ["sevenshell-core", 5],
  ["sevenshell-api", 6],
  ["sevenshell-cli", 7],
  ["sevenshell-mcp", 7],
  ["sevenshell-web", 7],
  ["sevenshell-desktop", 8],
]);

/** Crates allowed to know that Tauri exists. */
const TAURI_HOSTS = new Set(["sevenshell-desktop"]);

function manifestPaths() {
  const paths = [];
  const cratesDir = path.join(REPO_ROOT, "crates");
  for (const entry of readdirSync(cratesDir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      paths.push(path.join(cratesDir, entry.name, "Cargo.toml"));
    }
  }
  paths.push(path.join(REPO_ROOT, "src-tauri", "Cargo.toml"));
  return paths;
}

function readManifest(file) {
  const source = readFileSync(file, "utf8");
  const name = /^name = "([^"]+)"/m.exec(source)?.[1] ?? "";
  const keys = [...source.matchAll(/^\s*([A-Za-z0-9_-]+)\s*=/gm)].map((match) => match[1]);
  return {
    file: path.relative(REPO_ROOT, file).split(path.sep).join("/"),
    name,
    internal: [...new Set(keys.filter((key) => key.startsWith("sevenshell-")))],
    tauri: [...new Set(keys.filter((key) => key === "tauri" || key.startsWith("tauri-")))],
  };
}

const manifests = manifestPaths().map(readManifest);

test("every crate is ranked", () => {
  for (const manifest of manifestNames()) {
    assert.ok(RANKS.has(manifest), `${manifest} is not listed in RANKS — give it a rank deliberately`);
  }
});

function manifestNames() {
  return manifests.map((manifest) => manifest.name).filter((name) => name.length > 0);
}

test("crates only depend on lower-ranked crates", () => {
  for (const manifest of manifests) {
    const rank = RANKS.get(manifest.name);
    assert.notEqual(rank, undefined, `${manifest.name} is not ranked`);

    for (const dependency of manifest.internal) {
      assert.notEqual(dependency, manifest.name, `${manifest.name} depends on itself`);
      const dependencyRank = RANKS.get(dependency);
      assert.notEqual(dependencyRank, undefined, `${manifest.name} depends on unranked ${dependency}`);
      assert.ok(
        dependencyRank < rank,
        `${manifest.name} (rank ${rank}) depends on ${dependency} (rank ${dependencyRank}); dependencies must point downwards (${manifest.file})`,
      );
    }
  }
});

test("only hosts know about Tauri", () => {
  for (const manifest of manifests) {
    if (manifest.tauri.length === 0) {
      continue;
    }
    assert.ok(
      TAURI_HOSTS.has(manifest.name),
      `${manifest.name} depends on ${manifest.tauri.join(", ")}; only ${[...TAURI_HOSTS].join(", ")} may (${manifest.file})`,
    );
  }
});

test("no crate depends on a host", () => {
  for (const manifest of manifests) {
    const rank = RANKS.get(manifest.name);
    for (const dependency of manifest.internal) {
      if (!TAURI_HOSTS.has(dependency)) {
        continue;
      }
      assert.ok(false, `${manifest.name} (rank ${rank}) depends on the host ${dependency} (${manifest.file})`);
    }
  }
});

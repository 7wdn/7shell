// The scripts the release workflow runs, and the environment it runs them in.
//
// The publish job installs nothing: no `pnpm install`, no development dependencies, only the Node the
// runner already has. That is deliberate — publishing a release should need nothing that is not
// already there — and it is also invisible from a working copy, where `node_modules` exists because
// somebody installed the repository. The first release run that reached the publish step died on
// `Cannot find package 'yaml'`, imported three levels down for one path constant.
//
// So there are two checks, and they are different questions. Every import these scripts make must be
// a Node builtin, which is a fact about the source and can be read off it. And each one must actually
// run with no `node_modules` anywhere above it, in a directory that has never been installed — which
// is the environment the workflow puts them in and the only thing that proves the first check holds
// in practice.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

/** The scripts the publish job runs, by the names the workflow uses. */
const TOOLING = ["update-manifest.mjs", "packaging-manifests.mjs", "release-notes.mjs"];

/** Every module specifier a source file imports. */
function importSpecifiers(source) {
  const specifiers = [];
  for (const match of source.matchAll(/\bfrom\s+"([^"]+)"/g)) {
    specifiers.push(match[1]);
  }
  for (const match of source.matchAll(/^\s*import\s+"([^"]+)"/gm)) {
    specifiers.push(match[1]);
  }
  return specifiers;
}

test("the release tooling imports nothing but Node's own modules", async () => {
  const offenders = [];
  for (const name of TOOLING) {
    const source = await readFile(path.join(REPO_ROOT, "scripts", name), "utf8");
    const specifiers = importSpecifiers(source);
    assert.ok(specifiers.length > 0, `${name} imports nothing at all, so this check proves nothing`);
    for (const specifier of specifiers) {
      if (!specifier.startsWith("node:")) {
        offenders.push(`${name} imports ${specifier}`);
      }
    }
  }

  assert.deepEqual(
    offenders,
    [],
    `the publish job has no node_modules, so these would fail there: ${offenders.join(", ")}`,
  );
});

test("each one runs in a directory that has never been installed", async () => {
  // Outside the repository, so nothing named `node_modules` is anywhere above it.
  const room = await mkdtemp(path.join(tmpdir(), "release-tools-"));
  const tools = path.join(room, "scripts");
  await mkdir(tools, { recursive: true });

  // The layout the scripts expect: themselves in `<room>/scripts`, the files they read in `<room>`.
  for (const name of TOOLING) {
    await cp(path.join(REPO_ROOT, "scripts", name), path.join(tools, name));
  }
  await writeFile(
    path.join(room, "CHANGELOG.md"),
    "# Changelog\n\n## [1.0.0] - 2026-01-01\n\n### Added\n\n- something\n",
    "utf8",
  );
  await mkdir(path.join(room, "files"), { recursive: true });
  await writeFile(path.join(room, "files", "7Shell_1.0.0_x64-setup.exe"), "an installer", "utf8");
  // With a signature beside it, because that is the only thing the manifest will publish: without one
  // the script refuses, by design, and this test would be asserting the wrong failure.
  await writeFile(path.join(room, "files", "7Shell_1.0.0_x64-setup.exe.sig"), "a signature", "utf8");
  await mkdir(path.join(room, "releases"), { recursive: true });
  await writeFile(
    path.join(room, "releases", "windows-x86_64.json"),
    JSON.stringify({
      platform: "windows-x86_64",
      tag: "v1.0.0",
      entries: [{ installer: "nsis", name: "a.exe", signature: "sig", url: "https://example.invalid/a.exe" }],
    }),
    "utf8",
  );
  await writeFile(path.join(room, "notes.md"), "what changed\n", "utf8");

  /** Runs one of the copied scripts the way the workflow does, and reports how it went. */
  const run = (name, args) =>
    spawnSync(process.execPath, [path.join(tools, name), ...args], {
      cwd: room,
      encoding: "utf8",
      // Nothing about this machine's Node configuration is allowed to help: the runner has a plain one.
      env: { ...process.env, NODE_PATH: "", NODE_OPTIONS: "" },
    });

  const failed = [];
  const cases = [
    ["release-notes.mjs", ["1.0.0", "--check"]],
    ["release-notes.mjs", ["1.0.0"]],
    [
      "packaging-manifests.mjs",
      ["assets", "--dir", "files", "--tag", "v1.0.0", "--repository", "o/r", "--out", "assets.json"],
    ],
    [
      "update-manifest.mjs",
      [
        "platform",
        "--bundle",
        "files",
        "--platform",
        "windows-x86_64",
        "--tag",
        "v1.0.0",
        "--repository",
        "o/r",
        "--out",
        "platform.json",
      ],
    ],
    [
      "update-manifest.mjs",
      [
        "merge",
        "--releases",
        "releases",
        "--version",
        "1.0.0",
        "--tag",
        "v1.0.0",
        "--repository",
        "o/r",
        "--notes",
        "notes.md",
        "--date",
        "2026-01-01T00:00:00Z",
        "--out",
        "latest.json",
      ],
    ],
  ];

  for (const [name, args] of cases) {
    const result = run(name, args);
    if (result.status !== 0) {
      failed.push(
        `${name} ${args.join(" ")}: ${(result.stderr || result.stdout).trim().split("\n").slice(-3).join(" ")}`,
      );
    }
  }

  assert.deepEqual(failed, [], `these could not run without an installed repository:\n${failed.join("\n")}`);
});

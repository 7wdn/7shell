#!/usr/bin/env node
// Runs every gate in the order that fails fastest.
//
//   node scripts/run-check.mjs                 everything
//   node scripts/run-check.mjs --group=guards   only the cheap checks
//   node scripts/run-check.mjs --skip=rust      everything but the Rust gates
//   node scripts/run-check.mjs --only=types     one step
//
// The point is that a contributor (and CI) has a single command whose exit code means
// "this repository is consistent".

import { spawnSync } from "node:child_process";

import { REPO_ROOT } from "./lib/descriptors.mjs";

const STEPS = [
  { name: "descriptors", group: "guards", command: "node scripts/sync-descriptors.mjs --check" },
  { name: "i18n", group: "guards", command: "node scripts/check-i18n.mjs" },
  { name: "versions", group: "guards", command: "node scripts/reconcile-versions.mjs --check" },
  { name: "provenance", group: "guards", command: "node scripts/check-provenance.mjs" },
  { name: "guard tests", group: "guards", command: 'node --test "scripts/tests/*.test.mjs"' },
  {
    name: "format",
    group: "interface",
    // The globs are the script's own rather than repeated here: a copy of them in this file is a copy
    // that stops matching the day a new kind of source is added, and the gate would then be silent
    // about exactly the files nobody had formatted yet.
    command: "pnpm fmt:check",
  },
  {
    name: "lint",
    group: "interface",
    // The script's own command rather than the arguments written out here, for the reason the format
    // gate gives below: this one used to read `oxlint apps/desktop/src`, which is the same prefix as
    // `pnpm lint` and one directory short of it. The e2e suite went unlinted here while CI ran the
    // real command, and the difference showed up as a red `main` that was green on every machine that
    // only ever ran this file.
    command: "pnpm lint",
  },
  { name: "types", group: "interface", command: "pnpm exec vue-tsc --noEmit --project apps/desktop/tsconfig.json" },
  { name: "unit tests", group: "interface", command: "pnpm exec vitest run" },
  // The documentation site has a workflow of its own, but a build that only ever runs when a page is
  // pushed is a build that breaks quietly on the day somebody renames a file another page links to.
  { name: "docs build", group: "interface", command: "pnpm docs:build" },
  { name: "rust format", group: "rust", command: "cargo fmt --all --check" },
  { name: "rust lint", group: "rust", command: "cargo clippy --workspace --all-targets -- -D warnings" },
  { name: "rust tests", group: "rust", command: "cargo test --workspace" },
  {
    name: "supply chain",
    group: "rust",
    // The same command CI's supply chain job runs, over the same `deny.toml`: a policy file only ever
    // read by a workflow is a policy file nobody notices has stopped matching the tree, and the
    // licence and advisory rules are the ones that go stale quietly as dependencies move.
    // Needs `cargo install cargo-deny`.
    command: "cargo deny check licenses bans sources advisories",
  },
];

function parseArguments(argv) {
  const options = { group: null, skip: new Set(), only: null };
  for (const argument of argv) {
    const [key, value] = argument.replace(/^--/, "").split("=");
    if (key === "group" && value) {
      options.group = value;
    } else if (key === "skip" && value) {
      options.skip.add(value);
    } else if (key === "only" && value) {
      options.only = value;
    }
  }
  return options;
}

function selectedSteps(options) {
  return STEPS.filter((step) => {
    if (options.only) {
      return step.name === options.only;
    }
    if (options.group && step.group !== options.group) {
      return false;
    }
    return !options.skip.has(step.name) && !options.skip.has(step.group);
  });
}

function run(step) {
  const started = Date.now();
  const result = spawnSync(step.command, {
    cwd: REPO_ROOT,
    shell: true,
    stdio: "inherit",
  });
  return { status: result.status ?? 1, milliseconds: Date.now() - started };
}

function main() {
  const options = parseArguments(process.argv.slice(2));
  const steps = selectedSteps(options);
  if (steps.length === 0) {
    console.error("No steps selected. Use --group=guards|interface|rust, --skip=<name|group> or --only=<name>.");
    process.exit(2);
  }

  const results = [];
  for (const step of steps) {
    console.log(`\n=== ${step.name} ===`);
    const result = run(step);
    results.push({ name: step.name, ...result });
    if (result.status !== 0) {
      console.error(`\n${step.name} failed.`);
      break;
    }
  }

  console.log("\n=== summary ===");
  for (const result of results) {
    const outcome = result.status === 0 ? "ok  " : "FAIL";
    console.log(`${outcome} ${result.name} (${(result.milliseconds / 1000).toFixed(1)}s)`);
  }

  const failed = results.find((result) => result.status !== 0);
  const skipped = steps.length - results.length;
  if (failed) {
    console.error(`\nFAILED: ${failed.name}. ${skipped} later step(s) were not run.`);
    process.exit(1);
  }
  console.log(`\nAll ${results.length} step(s) passed.`);
}

main();

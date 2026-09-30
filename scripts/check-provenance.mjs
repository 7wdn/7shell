#!/usr/bin/env node
// Checks that this repository carries no trace of the material it was designed from.
//
// The forbidden words live in `provenance.rules.local.json`, which is ignored by the
// `*.local.json` rule and therefore never committed. This script contains no forbidden
// word itself, so it is safe to commit and safe to run in CI: with no local rules file it
// reports `skipped` and exits successfully. On a machine that has the rules file it is a
// real check, wired into the pre-commit hook and into `pnpm check`.
//
// What it inspects:
//   1. every tracked path and every tracked file's contents
//   2. the whole commit history (`git log -p --all`)
//   3. the local exclude file, so the ignored material stays ignored

import { execFileSync } from "node:child_process";
import { readFileSync, statSync } from "node:fs";
import path from "node:path";

import { REPO_ROOT } from "./lib/descriptors.mjs";

const RULES_FILE = path.join(REPO_ROOT, "provenance.rules.local.json");
const MAX_FILE_BYTES = 4 * 1024 * 1024;

function git(args) {
  return execFileSync("git", args, {
    cwd: REPO_ROOT,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function gitQuiet(args) {
  try {
    return git(args);
  } catch {
    return "";
  }
}

function loadRules() {
  let raw;
  try {
    raw = readFileSync(RULES_FILE, "utf8");
  } catch {
    return null;
  }
  const parsed = JSON.parse(raw);
  const terms = (parsed.terms ?? []).filter((term) => typeof term === "string" && term.length > 1);
  if (terms.length === 0) {
    throw new Error("provenance.rules.local.json lists no usable terms");
  }
  return {
    terms,
    caseInsensitive: parsed.caseInsensitive !== false,
    localExcludeExpectation: (parsed.localExcludeExpectation ?? []).filter((entry) => typeof entry === "string"),
  };
}

function buildMatcher(rules) {
  const needles = rules.terms.map((term) => (rules.caseInsensitive ? term.toLowerCase() : term));
  return (haystack) => {
    const text = rules.caseInsensitive ? haystack.toLowerCase() : haystack;
    return needles.filter((needle) => text.includes(needle));
  };
}

function trackedPaths() {
  const raw = git(["ls-files", "-z"]);
  return raw.split("\0").filter((entry) => entry.length > 0);
}

function looksBinary(file) {
  try {
    if (statSync(file).size > MAX_FILE_BYTES) {
      return true;
    }
    return readFileSync(file).subarray(0, 8192).includes(0);
  } catch {
    return true;
  }
}

function scanPaths(paths, matches) {
  const findings = [];
  for (const relative of paths) {
    const found = matches(relative);
    if (found.length > 0) {
      findings.push(`${relative}: path contains ${found.map((term) => `\`${term}\``).join(", ")}`);
    }
  }
  return findings;
}

function scanContents(paths, matches) {
  const findings = [];
  for (const relative of paths) {
    const full = path.join(REPO_ROOT, relative);
    if (looksBinary(full)) {
      continue;
    }
    const lines = readFileSync(full, "utf8").split(/\r?\n/);
    lines.forEach((line, index) => {
      const found = matches(line);
      if (found.length > 0) {
        findings.push(`${relative}:${index + 1}: contains ${found.map((term) => `\`${term}\``).join(", ")}`);
      }
    });
  }
  return findings;
}

function scanHistory(matches) {
  const findings = [];
  const log = gitQuiet(["log", "-p", "--all", "--no-color", "--pretty=format:COMMIT %H %s"]);
  if (log.trim() === "") {
    return findings;
  }

  let commit = "unknown";
  let lineNumber = 0;
  for (const line of log.split(/\r?\n/)) {
    lineNumber += 1;
    if (line.startsWith("COMMIT ")) {
      const [, hash, ...subject] = line.split(" ");
      commit = `${hash} ${subject.join(" ")}`.trim();
      continue;
    }
    const found = matches(line);
    if (found.length > 0) {
      findings.push(
        `history ${commit}: contains ${found.map((term) => `\`${term}\``).join(", ")} (patch line ${lineNumber})`,
      );
    }
  }
  return findings;
}

function scanLocalExclude(rules, matches) {
  const findings = [];
  let exclude = "";
  try {
    exclude = readFileSync(path.join(REPO_ROOT, ".git", "info", "exclude"), "utf8");
  } catch {
    return [".git/info/exclude is missing, so locally ignored material is not ignored"];
  }

  for (const expected of rules.localExcludeExpectation) {
    const present = exclude.split(/\r?\n/).some((line) => line.trim() === expected);
    if (!present) {
      findings.push(`.git/info/exclude no longer ignores \`${expected}\``);
    }
  }

  const hit = matches(exclude);
  if (hit.length === 0) {
    findings.push(".git/info/exclude contains no term from the rules, which means the material is not being ignored");
  }
  return findings;
}

function main() {
  const rules = loadRules();
  if (!rules) {
    console.log(`provenance: skipped (no ${path.basename(RULES_FILE)})`);
    return;
  }

  const matches = buildMatcher(rules);
  const paths = trackedPaths();

  const findings = [
    ...scanPaths(paths, matches),
    ...scanContents(paths, matches),
    ...scanHistory(matches),
    ...scanLocalExclude(rules, matches),
  ];

  if (findings.length > 0) {
    console.error(`Provenance check failed (${findings.length} finding(s)):`);
    for (const finding of findings) {
      console.error(`  - ${finding}`);
    }
    console.error("");
    console.error("This repository must not carry any trace of the material it was designed from.");
    console.error("Rewrite the offending text, and never edit the ignore rules to hide it.");
    process.exit(1);
  }

  console.log(`provenance: ${paths.length} tracked path(s) and the full history are clean`);
}

main();

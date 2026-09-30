#!/usr/bin/env node
// Fails when a message key is used but not translated.
//
// Three sources hand keys to the interface: the descriptors (labels), the Rust error
// catalog, and the interface itself. Every one of them has to exist in every locale, and
// all locales must agree on the key set — a missing translation is a bug, never a
// silent fall back to English.
//
// Locale files are JSON, one per domain, nested under the domain key, so this script
// needs no TypeScript parser.

import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

import { REPO_ROOT, loadDescriptors } from "./lib/descriptors.mjs";

const LOCALES_DIR = path.join(REPO_ROOT, "apps", "desktop", "src", "i18n", "locales");
const RUST_ROOTS = ["crates", "src-tauri"];
const TS_ROOT = path.join(REPO_ROOT, "apps", "desktop", "src");

function label(file) {
  return path.relative(REPO_ROOT, file).split(path.sep).join("/");
}

async function walk(directory, extensions) {
  const out = [];
  for (const entry of await readdir(directory, { withFileTypes: true }).catch(() => [])) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      out.push(...(await walk(full, extensions)));
      continue;
    }
    if (extensions.some((extension) => entry.name.endsWith(extension))) {
      out.push(full);
    }
  }
  return out;
}

function flatten(value, prefix, out) {
  for (const [key, entry] of Object.entries(value)) {
    const full = prefix ? `${prefix}.${key}` : key;
    if (entry !== null && typeof entry === "object") {
      flatten(entry, full, out);
    } else {
      out.add(full);
    }
  }
}

async function loadLocaleSources() {
  const languages = (await readdir(LOCALES_DIR, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();

  const locales = new Map();
  for (const language of languages) {
    const directory = path.join(LOCALES_DIR, language);
    const keys = new Set();
    const files = (await readdir(directory)).filter((name) => name.endsWith(".json"));
    for (const name of files) {
      const parsed = JSON.parse(await readFile(path.join(directory, name), "utf8"));
      flatten(parsed, "", keys);
    }
    locales.set(language, keys);
  }
  return locales;
}

/** Message keys the interface uses, by literal occurrence. */
async function collectSourceKeys() {
  const required = new Map();
  const pattern = /"errors\.[A-Za-z0-9_.]+"/g;

  for (const root of RUST_ROOTS) {
    for (const file of await walk(path.join(REPO_ROOT, root), [".rs"])) {
      const source = await readFile(file, "utf8");
      for (const match of source.match(pattern) ?? []) {
        required.set(match.slice(1, -1), label(file));
      }
    }
  }

  for (const file of await walk(TS_ROOT, [".ts", ".vue"])) {
    const source = await readFile(file, "utf8");
    for (const match of source.match(pattern) ?? []) {
      required.set(match.slice(1, -1), label(file));
    }
  }

  return required;
}

async function main() {
  const { capabilities, protocols, authMethods } = await loadDescriptors();
  const locales = await loadLocaleSources();
  const sourceKeys = await collectSourceKeys();

  const required = new Map(sourceKeys);
  for (const capability of capabilities) {
    required.set(capability.labelKey, "descriptors/capabilities.yaml");
  }
  for (const protocol of protocols) {
    required.set(protocol.labelKey, `descriptors/protocols/${protocol.id}.yaml`);
  }
  for (const method of authMethods) {
    required.set(method.labelKey, `descriptors/auth-methods/${method.id}.yaml`);
  }

  const problems = [];
  for (const [language, keys] of locales) {
    for (const [key, origin] of required) {
      if (!keys.has(key)) {
        problems.push(`${language}: missing \`${key}\` (used by ${origin})`);
      }
    }
  }

  // Locales must also be symmetric with each other: a key that exists in one language
  // and not another means a half-translated file, which reads as a bug to a user.
  const [referenceLanguage, referenceKeys] = [...locales.entries()][0] ?? ["en", new Set()];
  for (const [language, keys] of locales) {
    if (language === referenceLanguage) {
      continue;
    }
    for (const key of referenceKeys) {
      if (!keys.has(key)) {
        problems.push(`${language}: missing \`${key}\`, which ${referenceLanguage} defines`);
      }
    }
    for (const key of keys) {
      if (!referenceKeys.has(key)) {
        problems.push(`${referenceLanguage}: missing \`${key}\`, which ${language} defines`);
      }
    }
  }

  if (problems.length > 0) {
    console.error("Translation keys are out of step:");
    for (const problem of [...new Set(problems)]) {
      console.error(`  - ${problem}`);
    }
    process.exit(1);
  }

  console.log(`i18n: ${required.size} key(s) present in ${locales.size} locale(s)`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});

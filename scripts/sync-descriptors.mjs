#!/usr/bin/env node
// Turns descriptors/**/*.yaml into the committed outputs the Rust and TypeScript
// sides both read. `--check` verifies the committed outputs are current instead of
// writing, which is what CI and `pnpm check` run.

import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

import { DescriptorError, GENERATED_TS_DIR, REPO_ROOT, generatedOutputs, loadDescriptors } from "./lib/descriptors.mjs";

const checkOnly = process.argv.includes("--check");

function label(file) {
  return path.relative(REPO_ROOT, file).split(path.sep).join("/");
}

async function readIfPresent(file) {
  try {
    return await readFile(file, "utf8");
  } catch {
    return null;
  }
}

async function main() {
  const descriptors = await loadDescriptors();
  const outputs = generatedOutputs(descriptors);

  if (checkOnly) {
    const stale = [];
    for (const [file, expected] of outputs) {
      const actual = await readIfPresent(file);
      if (actual !== expected) {
        stale.push(file);
      }
    }
    if (stale.length > 0) {
      console.error("Descriptor outputs are stale. Run `pnpm generate:descriptors` and commit the result.");
      for (const file of stale) {
        console.error(`  - ${label(file)}`);
      }
      process.exit(1);
    }
    console.log(`descriptors: ${outputs.size} generated file(s) up to date`);
    return;
  }

  await mkdir(GENERATED_TS_DIR, { recursive: true });
  for (const [file, content] of outputs) {
    await writeFile(file, content, "utf8");
    console.log(`wrote ${label(file)}`);
  }
}

main().catch((error) => {
  if (error instanceof DescriptorError) {
    console.error(`descriptor error: ${error.message}`);
  } else {
    console.error(error);
  }
  process.exit(1);
});

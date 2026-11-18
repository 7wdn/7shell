#!/usr/bin/env node
/**
 * Starts the desktop application for development with its own vault and settings.
 *
 * A development build and an installed copy resolve the same configuration directory by default,
 * so whichever runs first writes a vault the other may not be able to read. A newer build writing
 * a protocol or schema an older installed binary does not know is exactly how an unlock screen ends
 * up claiming the password is wrong. This command keeps the two apart.
 *
 * An existing `SEVENSHELL_CONFIG_DIR` wins: the e2e launcher points this same command at its own
 * throwaway directory. The path below is derived from this file at run time, never written down, so
 * the repository carries no machine's layout.
 */
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DEV_CONFIG = path.join(REPO_ROOT, ".scratch", "dev-config");
mkdirSync(DEV_CONFIG, { recursive: true });

const environment = { ...process.env };
environment.SEVENSHELL_CONFIG_DIR ||= DEV_CONFIG;

// `pnpm dev:tauri -- --help` hands the double dash through as an argument; it is not part of what
// the Tauri command should see.
const args = process.argv.slice(2);
if (args[0] === "--") {
  args.shift();
}

// The pnpm that runs this script is already a JavaScript entry point, so a second pnpm can be
// started without a shell — a shell would concatenate the arguments and is deprecated for that.
// A direct `node scripts/dev-app.mjs` has no such entry and falls back to the command on PATH.
const pnpm = process.env.npm_execpath;
const child = pnpm
  ? spawn(process.execPath, [pnpm, "exec", "tauri", "dev", ...args], {
      cwd: REPO_ROOT,
      env: environment,
      stdio: "inherit",
    })
  : spawn("pnpm", ["exec", "tauri", "dev", ...args], {
      cwd: REPO_ROOT,
      env: environment,
      shell: process.platform === "win32",
      stdio: "inherit",
    });

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});

#!/usr/bin/env node
// Builds the MCP bridge against an older glibc than the desktop's own build.
//
// The desktop is packaged for Ubuntu 22.04 and newer, so its MCP bridge needs glibc 2.34 and the
// OpenSSL 3 the desktop bundle already carries. A client on UOS 20 or Kylin V10 starts the bridge
// on the host, whose glibc is 2.28 and whose OpenSSL is 1.1 — the bridge has to be built for that
// host, not for the compatibility environment the desktop itself runs in. `vendored-tls` compiles
// OpenSSL into the binary so the only shared dependency left is glibc; this script checks that the
// result really only asks for 2.28 and does not name libssl or libcrypto.

import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { assertGlibcFloor } from "./lib/linux-floor.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);

function value(name) {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : undefined;
}

const arch = value("arch") ?? (process.arch === "arm64" ? "arm64" : "amd64");
const triples = {
  amd64: "x86_64-unknown-linux-gnu",
  arm64: "aarch64-unknown-linux-gnu",
};
const triple = triples[arch];
if (!triple) {
  console.error(`unknown --arch ${arch}; use amd64 or arm64`);
  process.exit(2);
}

const out = path.resolve(value("out") ?? path.join(ROOT, "legacy-seed", `sshell-mcp-${arch}`));

const build = spawnSync(
  "cargo",
  ["zigbuild", "--release", "-p", "sevenshell-mcp", "--target", `${triple}.2.28`, "--features", "vendored-tls"],
  { cwd: ROOT, stdio: "inherit", env: process.env },
);
if (build.error?.code === "ENOENT") {
  console.error("cargo zigbuild is not installed: install Zig and `cargo install cargo-zigbuild`");
  process.exit(1);
}
if (build.status !== 0) {
  console.error(`cargo zigbuild failed with status ${build.status}`);
  process.exit(build.status ?? 1);
}

const targetDir = path.resolve(ROOT, process.env.CARGO_TARGET_DIR ?? "target");
const binary = path.join(targetDir, triple, "release", "sshell-mcp");
const floor = assertGlibcFloor(readFileSync(binary), 28, "the legacy MCP bridge");

const dynamic = spawnSync("readelf", ["-d", binary], { encoding: "utf8" });
if (dynamic.error) {
  console.error("readelf is required to prove the bridge has no dynamic OpenSSL");
  process.exit(1);
}
if (dynamic.status !== 0) {
  console.error(dynamic.stderr.trim() || "readelf failed");
  process.exit(1);
}
const tls = dynamic.stdout.split("\n").filter((line) => /\bNEEDED\b/.test(line) && /\blib(ssl|crypto)\.so/.test(line));
if (tls.length > 0) {
  console.error(`the bridge still links a dynamic OpenSSL:\n${tls.join("\n")}`);
  process.exit(1);
}

mkdirSync(path.dirname(out), { recursive: true });
copyFileSync(binary, out);
chmodSync(out, 0o755);
console.log(`legacy MCP bridge: ${out} (glibc floor ${floor}, no dynamic OpenSSL)`);

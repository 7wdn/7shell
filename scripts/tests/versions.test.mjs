import assert from "node:assert/strict";
import test from "node:test";

import {
  readJsonVersion,
  readWorkspaceVersion,
  replaceJsonVersion,
  replaceWorkspaceVersion,
} from "../lib/versions.mjs";

const CARGO_TOML = `[workspace]
resolver = "3"
members = ["src-tauri", "crates/*"]

[workspace.package]
# The released version. Only version.json is edited by hand.
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
sevenshell-error = { version = "0.1.0", path = "crates/sevenshell-error" }
sevenshell-types = { version = "0.1.0", path = "crates/sevenshell-types" }

[profile.release]
strip = true
`;

/** A manifest written with an unusual layout, to prove the check reads numbers, not whitespace. */
const WRAPPED_JSON = `{
  "name": "sevenshell",
  "version": "0.1.0",
  "engines": { "node": ">=22" }
}
`;

test("a JSON manifest keeps its other fields and their order", () => {
  const source = `{\n  "name": "sevenshell",\n  "version": "0.1.0",\n  "private": true\n}\n`;
  const updated = replaceJsonVersion(source, "1.2.3");

  assert.equal(updated, `{\n  "name": "sevenshell",\n  "version": "1.2.3",\n  "private": true\n}\n`);
});

test("the workspace version and the internal dependencies move together", () => {
  const updated = replaceWorkspaceVersion(CARGO_TOML, "2.0.0");

  assert.ok(updated.includes('version = "2.0.0"\nedition = "2024"'), "the workspace version was not replaced");
  assert.ok(updated.includes('sevenshell-error = { version = "2.0.0", path = "crates/sevenshell-error" }'));
  assert.ok(updated.includes('sevenshell-types = { version = "2.0.0", path = "crates/sevenshell-types" }'));
});

test("nothing else in Cargo.toml is touched", () => {
  const updated = replaceWorkspaceVersion(CARGO_TOML, "2.0.0");

  // The serde requirement, the feature list, the profile and every comment survive.
  assert.ok(updated.includes('serde = { version = "1", features = ["derive"] }'));
  assert.ok(updated.includes("[profile.release]\nstrip = true"));
  assert.ok(updated.includes("# The released version. Only version.json is edited by hand."));
  assert.equal(updated.replace(/2\.0\.0/g, "0.1.0"), CARGO_TOML, "only the version numbers changed");
});

test("rewriting to the version already present is a no-op", () => {
  assert.equal(replaceWorkspaceVersion(CARGO_TOML, "0.1.0"), CARGO_TOML);
});

test("a Cargo.toml without the workspace section is an error, not a silent no-op", () => {
  assert.throws(() => replaceWorkspaceVersion('[package]\nname = "x"\n', "1.0.0"), /workspace\.package/);
  assert.throws(() => replaceWorkspaceVersion('[workspace.package]\nedition = "2024"\n', "1.0.0"), /no version/);
});

test("the version is read back out of every manifest shape", () => {
  assert.equal(readJsonVersion(WRAPPED_JSON), "0.1.0");
  assert.equal(readWorkspaceVersion(CARGO_TOML), "0.1.0");

  // A manifest without the section, or without a version in it, reports nothing rather than
  // guessing — the caller then treats it as drift.
  assert.equal(readWorkspaceVersion('[package]\nname = "x"\n'), null);
  assert.equal(readWorkspaceVersion('[workspace.package]\nedition = "2024"\n'), null);
  assert.equal(readWorkspaceVersion("[workspace.package]\nversion = 1\n"), null);
});

test("a hand-wrapped manifest is still the right version", () => {
  // Formatting is not a release problem, so the check must not report one.
  assert.equal(readJsonVersion(WRAPPED_JSON), "0.1.0");
  assert.notEqual(replaceJsonVersion(WRAPPED_JSON, "0.1.0"), WRAPPED_JSON, "writing still normalises the layout");
  assert.equal(readJsonVersion(replaceJsonVersion(WRAPPED_JSON, "1.0.0")), "1.0.0");
});

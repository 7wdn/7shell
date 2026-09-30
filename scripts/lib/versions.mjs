// Version rewriting for the manifests that cannot read version.json themselves.
//
// Deliberately regex-based rather than a full parser: the rewrite has to preserve comments
// and formatting in files people edit by hand, and the exact patterns it touches are pinned
// by `scripts/tests/versions.test.mjs`. A silent miss here means a release whose version
// disagrees with itself.

/** Sets `version` in a JSON manifest, leaving the other fields and their order alone. */
export function replaceJsonVersion(source, version) {
  const parsed = JSON.parse(source);
  parsed.version = version;
  return `${JSON.stringify(parsed, null, 2)}\n`;
}

/**
 * Sets the workspace version in Cargo.toml, and moves the internal path dependencies with
 * it: their requirement has to be a real version rather than a wildcard (cargo-deny fails
 * on wildcards), which means they need the same number.
 */
export function replaceWorkspaceVersion(source, version) {
  const header = "[workspace.package]";
  const start = source.indexOf(header);
  if (start === -1) {
    throw new Error("Cargo.toml has no [workspace.package] section");
  }

  const body = source.slice(start);
  if (!/^version = "/m.test(body)) {
    throw new Error("Cargo.toml [workspace.package] has no version");
  }

  const withWorkspaceVersion = body.replace(/^version = "[^"]*"/m, `version = "${version}"`);
  const updated = withWorkspaceVersion.replace(/^(sevenshell-[a-z0-9-]+ = \{ version = )"[^"]*"/gm, `$1"${version}"`);
  return source.slice(0, start) + updated;
}

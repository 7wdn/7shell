# Release process

**Nothing has been released yet.** This is the process the first release will follow, written down
before it is needed so the first one is boring.

## Version lines

| Line | Tag | What it covers | Version lives in |
|---|---|---|---|
| Application | `v*` | The desktop application (Windows, macOS, Linux) | `version.json` → `app` |
| Packages | `packages-v*` | The command line and agent interface, once they exist | `version.json` → `packages` |

Only `version.json` is edited by hand. `pnpm sync:versions` writes the number into `package.json`,
`src-tauri/tauri.conf.json` and the Cargo workspace, and `pnpm verify:versions` (run by CI) fails
when any of them disagrees. A version number that has to be updated in four places by hand is a
version number that will eventually disagree with itself in a release build.

## Before a release

1. `pnpm check` green, on all three platforms in CI.
2. `CHANGELOG.md` updated. Entries are written for someone deciding whether to upgrade, not for
   someone reading the diff: what changed, what it means for them, what breaks.
3. The version bumped with `pnpm sync:versions` and `version.json` committed.
4. For the first release only: the README's status line, `SECURITY.md` and the notice in the
   interface footer all still say "no security review" — a release does not change that until an
   audit has actually happened.

## Cutting the release

1. Tag `v<version>` on the commit that bumped the version.
2. CI builds the installers (MSI and NSIS on Windows, DMG on macOS with notarization, AppImage,
   deb and rpm on Linux) and signs what can be signed.
3. The release notes come from the changelog and list the artefacts with their checksums.
4. Package manager manifests are updated: Homebrew, Winget, Scoop and Flatpak where they apply.
5. If the update channel is live, update the signed update metadata — and never publish update
   metadata that points at an unsigned or unbuilt artefact.

## What a release must not do

- **It must not claim more than it delivers.** The capability list in the interface is generated
  from the descriptors, and the descriptors only claim what is implemented — if a release note and
  that screen disagree, the screen is right and the note is a bug.
- **It must not ship a binary that was not built by CI from the tagged commit.** A local build is
  for testing; a release artefact is reproducible from a tag.
- **It must not skip the security statement.** Until an audit exists, every release says there is
  no audit.

## Signing and secrets

Signing keys and notarization credentials live only in CI secrets, never in the repository and
never on a contributor's machine as part of the build. A build that needs a secret to compile is a
build that cannot be verified by anyone else.

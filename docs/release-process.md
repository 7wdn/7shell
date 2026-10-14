# Release process

**Nothing has been released yet.** This is the process the first release will follow, written down
before it is needed so the first one is boring. Everything below is what the workflows and scripts
actually do; a step described here that no file implements is a bug in this document, not an
aspiration.

## Version lines

| Line | Tag | What it covers | Version lives in |
|---|---|---|---|
| Application | `v*` | The desktop application (Windows, macOS, Linux) | `version.json` → `app` |
| Packages | `packages-v*` | The command line and agent interface, once they exist | `version.json` → `packages` |

Only `version.json` is edited by hand. `pnpm sync:versions` writes the number into `package.json`,
`src-tauri/tauri.conf.json` and the Cargo workspace, and `pnpm verify:versions` (run by CI) fails
when any of them disagrees. A version number that has to be updated in four places by hand is a
version number that will eventually disagree with itself in a release build.

A version is **numbers and periods only** — `0.1.0`, then `0.1.1`, then `0.2.0`. Two reasons, and
both are mechanical rather than stylistic:

- **An MSI can carry at most a numeric pre-release identifier.** The Windows installer's version is
  four numbers, and `0.1.0-rc.1` is rejected by the bundler with "optional pre-release identifier in
  app version must be numeric-only". A version with a suffix is a version Windows cannot be built for.
- **Package managers carry releases, not candidates.** Winget's own pipeline refuses a pre-release
  version outright, and a Homebrew cask or a Scoop manifest that names one is a package that installs
  something the registry never reviewed.

So a **candidate is not a version, it is this release marked as one**: the artefacts are attached to a
draft, the draft is published as a prerelease, and a person promotes it when they are satisfied. The
update channel follows the promotion — installed builds are never offered a candidate.

## Before a release

1. `pnpm check` green, on all three platforms in CI.
2. `CHANGELOG.md` has a section for the version being released. `pnpm release:notes <version> --check`
   is the gate: a release whose notes were never written fails before anything is built, because a
   release with an empty body is a release nobody can decide about.
3. The version bumped with `pnpm sync:versions`, and `version.json` committed.

## Cutting the release

1. Tag `v<version>` on the commit that bumped the version, and push the tag. That is the whole
   trigger: `.github/workflows/release.yml` refuses to build when the tag, `version.json` and the
   changelog disagree, so the three cannot drift apart unnoticed.
2. Four builds run in parallel — Windows (MSI and NSIS), macOS on Apple silicon, macOS on Intel,
   Linux (AppImage, deb, rpm) — each on the runner for the platform it produces.
3. The release job hashes every installer, writes the update manifest, writes the package manager
   manifests, and attaches everything to a **draft** release. Nothing is on the releases page while
   that is happening, so nobody opens a release that is still being filled.
4. When everything is attached, the draft is published **as a prerelease** — the candidate. Its notes
   and its files are the real thing; what it is not is what installed builds are offered.
5. **Promoting** it is the other half: `workflow_dispatch` on the Release workflow with the tag and
   `promote` set, or by hand — `gh release edit v0.1.0 --prerelease=false --latest`. That is the moment
   the update channel starts offering it, and it is a decision rather than a build step: a channel that
   offered candidates would make users the ones who find out that a build was not ready. A release
   whose build carried no signed update manifest cannot fill the channel, and the step says so rather
   than publishing a manifest for an unsigned artefact.
6. Package manager manifests are attached to the release and are submitted by hand from there; see
   `packaging/README.md` for what each submission takes. Winget carries releases rather than
   candidates, so the first version it can carry is the first stable one.

## The update channel

An installed build asks one fixed URL:
`https://github.com/7wdn/7shell/releases/download/updates/latest.json`. That is a release of its own,
tagged `updates`, whose single asset is replaced when a release is **promoted** — a URL that changed
per version would be a URL baked into the builds people already have, and those are the ones that
matter.

The manifest is assembled from what each build signed, never from what a workflow assumed:
`scripts/update-manifest.mjs` reads back the archives the bundler wrote, and refuses to publish when
an archive has no signature beside it, when two platform files disagree about the tag, or when an
archive has a name it cannot place. The application verifies the signature of everything it downloads
against the public key committed in `src-tauri/tauri.conf.json` before installing it.

Two things worth knowing before the first release:

- **Without the signing key a release has no update manifest at all.** The workflow says so in its
  summary and publishes installers only; the promotion step then leaves the channel as it was, because
  a manifest pointing at files nobody signed would be worse than none.
- **The channel has never delivered an update until it has.** It needs two versions: install one, and
  it should be offered the next. Until that has happened, the manifest's shape is covered by tests and
  its behaviour is not.

## Signing and secrets

Signing keys and notarization credentials live only in CI secrets, never in the repository and never
on a contributor's machine as part of the build. A build that needs a secret to compile is a build
that cannot be verified by anyone else — which is why the release switches the update artifacts on
from the workflow rather than from `tauri.conf.json`, and why a build with no secrets at all still
produces installers.

| Secret | What it is for | State |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Signs the update archives. The matching public key is committed in `src-tauri/tauri.conf.json`. | Needed for the update channel |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The password that key is stored with. | Needed with the key |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | Signing and notarizing the macOS build. | Absent until an Apple Developer account exists |
| A Windows code-signing certificate | Not configured: Tauri signs with whatever certificate the environment provides, and there is none yet. | Absent |

Until those exist, the release says so in its own body: the installers are not code-signed, the macOS
build is not notarized, and both operating systems warn before running them. Losing the update key
means no further update can reach an installed build — a new key would have to be committed to
`tauri.conf.json`, and a fresh install would be the only way onto it.

## What a release must not do

- **It must not claim more than it delivers.** The capability list in the interface is generated from
  the descriptors, and the descriptors only claim what is implemented — if a release note and that
  screen disagree, the screen is right and the note is a bug.
- **It must not ship a binary that was not built by CI from the tagged commit.** A local build is for
  testing; a release artefact is reproducible from a tag.
- **It must not publish update metadata that points at an unsigned artefact.** The workflow enforces
  this by refusing to write a manifest for an archive with no signature beside it.
- **It must not skip the security statement.** Until an audit exists, every release says there is no
  audit.

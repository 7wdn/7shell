# Package manager manifests

Nothing in this directory is written by hand. `scripts/packaging-manifests.mjs` reads the files a
release actually produced — hashing them on the way in — and writes the three manifests below into
`packaging/out/`, which the release attaches as `packaging-manifests.zip`. A manifest with a version
or a hash typed by a person is a manifest that disagrees with the release it names.

```
node scripts/packaging-manifests.mjs assets \
  --dir <the release's downloaded files> --tag v0.1.0-rc.1 --repository 7wdn/7shell --out assets.json

node scripts/packaging-manifests.mjs manifests \
  --assets assets.json --version 0.1.0-rc.1 --tag v0.1.0-rc.1 \
  --repository 7wdn/7shell --date 2026-10-12 --out packaging/out
```

## What each one is, and what submitting it takes

| Manager | Written as | Submission |
|---|---|---|
| Winget | `winget/SevenShell.7Shell.{yaml,installer.yaml,locale.en-US.yaml}` | A pull request against `microsoft/winget-pkgs`. `wingetcreate update SevenShell.7Shell --version <v> --urls <installer> --submit` does it with a GitHub token; the registry's own CI then downloads the installer and checks the hash. Winget carries *releases*, so nothing is written for a candidate version — the first one it can carry is `0.1.0`. |
| Scoop | `scoop/7shell.json` | Copy into a bucket repository (`<owner>/scoop-<name>`, one this project would have to own) and push. `scoop install ./7shell.json` on a Windows machine is what verifies it. |
| Homebrew | `homebrew/7shell.rb` | Copy into a tap (`<owner>/homebrew-<name>`) and push. `brew install --cask ./7shell.rb` is what verifies it. |

None of these are automated end to end, and that is deliberate rather than unfinished: each one ends
in a review by somebody else, on infrastructure this project does not own, with credentials only a
maintainer holds. What *is* automated is the part that can be wrong silently — the version, the URL
and the hash — and `scripts/tests/packagingManifests.test.mjs` is where that is checked.

## Flatpak is not here yet

Flatpak is the one the plan lists that this does not generate, and the reason is that it is a
different kind of thing: a Flathub manifest builds the application from source inside a sandbox
rather than installing a file this project built, so it needs the source archive *and* a generated
`cargo-sources.json` (every crate the build downloads, with its hash) — a file produced by tooling
this repository does not run, and one that has to be regenerated whenever `Cargo.lock` changes. A
manifest generated here without it would be a file that looks like a Flatpak recipe and builds
nothing, which is worse than saying so. Doing it properly is a workstream of its own, on a machine
with `flatpak-builder` and a Flathub account.

# APM package (UOS 20 / Kylin V10)

UOS 20 and Kylin V10 are Debian 10 baselines: glibc 2.28, WebKitGTK 4.0 and OpenSSL 1.1. The
desktop needs WebKitGTK 4.1 and glibc 2.35, and no native package can change that — the oldest
practical base that carries WebKitGTK 4.1 is Debian 12. The `-apm.deb` asset therefore runs the
ordinary Linux deb inside [APM's](https://gitee.com/amber-ce/amber-pm) Debian 12 environment.

Nothing here is built by hand. The release workflow's `linux-apm` job produces the two assets; this
file describes what it does and how to reproduce it on a machine with APM installed.

## What is in the package

- The same application files as the native deb.
- An APM control file whose `Depends` names `amber-pm-bookworm`, the Debian 12 base environment.
- The tray library (`libayatana-appindicator3` and its dependencies): the base does not carry it,
  and the application loads it with `dlopen` at startup. The packaging script refuses a package
  without it rather than shipping one that dies before the window appears.
- A desktop entry that starts the application through `apm run`.
- A **different MCP bridge seed**: `sshell-mcp-seed` is built for glibc 2.28 with OpenSSL linked
  into the binary. An MCP client starts that seed on the host, outside the compatibility
  environment, so it cannot use the base environment's glibc or OpenSSL. The ordinary seed keeps
  the system TLS stack and is unchanged.

## Versions

| What | Version | Where |
|---|---|---|
| APM | `1.3.4.2` | the release job downloads `apm_<version>_<arch>.deb` from AmberPM's releases |
| Base environment | `amber-pm-bookworm 12.9.9` | downloaded from `d.spark-app.store` and checked against a pinned SHA-256 |
| Zig | `0.17.1` | `build-legacy-seed.mjs` needs `cargo zigbuild` and a `zig` on `PATH` |

The two SHA-256 values live in the `linux-apm` matrix in `.github/workflows/release.yml`; changing
a base version means changing the version and its hash together, in the same commit.

## Reproducing a build

```bash
# 1. Build the bridge the host will start: glibc 2.28, static OpenSSL.
cargo install cargo-zigbuild          # once, with Zig 0.17.1 on PATH
node scripts/build-legacy-seed.mjs --arch amd64 --out legacy-seed/sshell-mcp

# 2. Install APM and the base environment (Ubuntu 22.04/24.04):
sudo apt install ./apm_1.3.4.2_amd64.deb
# The base package is in the APM package index; install the same file the CI pins:
curl -L -o base.deb \
  https://d.spark-app.store/amd64-apm/depends/base-bookworm/amber-pm-bookworm_12.9.9_amd64.deb
sudo apt install ./base.deb

# 3. Convert the deb the Tauri build produced.
sudo node scripts/package-apm.mjs \
  --deb 7Shell_<version>_amd64.deb \
  --seed legacy-seed/sshell-mcp \
  --version <version> \
  --out dist-apm
```

`package-apm.mjs` replaces the seed in the deb, runs `amber-pm-convert --base amber-pm-bookworm`,
then reads the result back: package name, architecture, the `amber-pm-bookworm` dependency, the seed
hash and the `apm run` prefix in the desktop entry. A converter that drops any of those fails the
build rather than shipping a package that will not start.

## Installing on the target

```bash
sudo apt install ./7Shell_<version>_<arch>-apm.deb
```

The first install also fetches `amber-pm-bookworm` if it is not present. Starting the application
from the menu runs it inside that base environment; the home directory is shared with the host and
other host paths appear under `/host`.

## Limits

- APM is a compatibility layer, not part of UOS or Kylin. A machine that forbids third-party
  runtimes cannot use this package; there is no native fallback short of a private WebKitGTK 4.1
  build for glibc 2.28.
- In-app updates belong to the AppImage channel. APM packages update through APM or the store.
- The package must be rebuilt when `amber-pm-bookworm` changes (glibc, WebKit, OpenSSL or the
  package index), and the pinned hash must move with it.

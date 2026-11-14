# Install

Two ways to get 7Shell: an installer from GitHub Releases, or a build from source. No distribution
publishes it yet, so a package manager cannot install it for you.

## What a release provides

Builds are published at [github.com/7wdn/7shell/releases](https://github.com/7wdn/7shell/releases).

| Platform | Files | What running it needs |
|---|---|---|
| Windows 10 and later | `.msi` (WiX) and `.exe` (NSIS) | The WebView2 runtime, present on Windows 10 and later |
| macOS | `.dmg` | Nothing beyond macOS itself |
| Linux, glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+) | `.AppImage`, `.deb`, `.rpm` | The executable bit for the AppImage (`chmod +x`); a package manager and WebKitGTK 4.1 for the other two |
| Linux, glibc 2.28 (UOS 20, Kylin V10) | `-apm.deb` | APM and its Debian 12 base environment, see [below](#uos-20-and-kylin-v10) |

The Linux desktop is built against WebKitGTK 4.1, which Debian 10 and the distributions derived
from it do not carry. A native build therefore needs glibc 2.35 or newer; the compatibility package
below is what runs on the older baseline.

The **current release, `0.1.0`, is published as a prerelease**. That is a real download — it is what
the project has built and tested — but it is not what a stable release will be: it says so on its own
page, in its notes, and by not being offered to installed builds yet. A release is promoted to the
stable channel once it has been used, and the
[release process](/release-process.md) describes exactly what that means.

If no build is on the releases page for your platform, the way to run the application is
[to build it from source](#build-from-source).

### The operating system will warn, and that is expected

The installers are **not code-signed**, and the macOS build is **not notarized**. Both are the
current state rather than an instruction to ignore: the project has no signing certificate yet, and
the [release process](/release-process.md) puts signing keys in CI secrets that do not exist.

What each system does about it:

- **Windows.** SmartScreen shows "Windows protected your PC" and names an unknown publisher. The
  `.msi` and `.exe` are the ones the release page links to; check the SHA-256 in the release notes
  against the file you downloaded, then choose to run it anyway.
- **macOS.** Gatekeeper refuses the first launch of an unnotarized application. Right-click the
  application and choose Open, or allow it once under System Settings → Privacy & Security. Nothing
  is wrong with the download; this is what macOS says about any unsigned build.
- **Linux.** No signature check happens on an AppImage. On a `.deb` or `.rpm`, a package manager may
  warn about an untrusted repository or an unsigned package.

Do not turn SmartScreen or Gatekeeper off for this. Check the checksum, or build from source and
compare what you run with what the tag contains.

### UOS 20 and Kylin V10

UOS 20 and Kylin V10 are Debian 10 baselines: glibc 2.28, WebKitGTK 4.0 and OpenSSL 1.1, while the
desktop is built against WebKitGTK 4.1 and glibc 2.35. Installing the ordinary `.deb` there fails
on its WebKit dependency, and the AppImage stops at the loader with `GLIBC_2.34 not found`. The
`-apm.deb` asset is built for exactly these systems: it runs the same application inside
[APM](https://gitee.com/amber-ce/amber-pm)'s Debian 12 environment, so the system's own WebKit,
OpenSSL and glibc are not used by the window.

1. Install APM and its Debian 12 base environment (`amber-pm-bookworm`). On a machine with the
   Spark Store, `sudo aptss install apm`; otherwise install the `apm` package from
   [AmberPM's releases](https://gitee.com/amber-ce/amber-pm/releases/). The APM store installs the
   base together with an application; when installing the downloaded `.deb` by hand, install the
   base first (it is the one dependency the package has, and the APM index provides it — the
   pinned version and its checksum are in
   [packaging/apm/README.md](https://github.com/7wdn/7shell/blob/main/packaging/apm/README.md)).
2. Install the compatibility package:

   ```bash
   sudo apt install ./7Shell_<version>_<arch>-apm.deb
   ```

   If APM reports that `amber-pm-bookworm` is not installable, the base is missing; install it from
   the APM index first, then repeat this command.
3. Start 7Shell from the desktop menu as usual. The entry starts it through `apm run`, inside the
   compatibility environment.

What changes compared with a native install:

- **Files.** Your home directory is shared with the host. Other host paths appear under `/host`
  (for example `/host/etc/ssh`); the container's own root is `/`.
- **Updates.** APM packages update through APM or the store. The in-app update check belongs to the
  AppImage channel and cannot replace an APM package, so it is not the update path here.
- **MCP bridge.** The package carries a bridge built for glibc 2.28 and with OpenSSL linked in, so
  an MCP client running on the host starts it exactly as it does on a native install, and it
  reaches the desktop's loopback endpoint.
- **Old graphics stacks.** If the window stays blank on a machine whose Mesa cannot use the
  DMABUF renderer, start it once with `WEBKIT_DISABLE_DMABUF_RENDERER=1`; if there is no GPU
  acceleration at all, add `LIBGL_ALWAYS_SOFTWARE=1`.

## Build from source

Requirements:

- **Rust**, installed with `rustup`. The exact toolchain is pinned in `rust-toolchain.toml` and
  rustup installs it on the first build.
- **Node 22 or newer**, and **pnpm 11** (`corepack enable` picks up the version the repository
  pins).
- On Windows: the MSVC toolchain (Visual Studio Build Tools with the C++ workload) and WebView2.
- On Linux: the development packages the desktop shell links against —
  `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` and
  `patchelf`.

```bash
pnpm install

pnpm dev:tauri    # the desktop application, rebuilt as files change
pnpm dev          # the interface alone in a browser, with no backend behind it
pnpm build        # the interface bundle
pnpm tauri build  # package the desktop application into an installer

cargo test --workspace   # the Rust tests
pnpm check               # every gate: descriptors, translations, guards, lint, types, tests
```

`pnpm dev` opens the interface in an ordinary browser, where nothing can connect anywhere: every
backend call answers with a "needs the desktop application" error rather than an undefined function.
It is for working on the interface alone.

`pnpm check` runs the gates CI runs. Where `make` is available, the Makefile holds the same commands
as thin wrappers.

## The SSH test lab

Everything interesting about an SSH client only shows up against a real server, so the repository
carries one: a Docker container running `sshd`, with a deliberately trivial account
(**`labuser`** / **`labpassword`**) bound to loopback only.

```bash
make lab-up        # cd deploy/lab/openssh && docker compose up -d
make lab-verify    # the live tests: connect, verify a host key, run a command, read the output
make lab-down      # stop and remove it
```

`make lab-up` is `cd deploy/lab/openssh && docker compose up -d`; `make lab-verify` is

```bash
SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 \
SEVENSHELL_TEST_USER=labuser SEVENSHELL_TEST_PASSWORD=labpassword \
  cargo test -p sevenshell-ssh -p sevenshell-core -- --ignored --test-threads=1
```

The tests that need the lab are `#[ignore]`d and read those variables. Without them they report
that the lab is not configured and pass, so an ordinary `cargo test` stays offline.

The server listens on `127.0.0.1:2222`. On Windows that port is often unavailable: Hyper-V and WSL
reserve ranges of ports, and a `docker compose up` that reports "An attempt was made to access a
socket in a way forbidden by its access permissions" (rather than "already in use") has hit one.
Check what is reserved:

```bash
netsh interface ipv4 show excludedportrange protocol=tcp
```

Then publish the lab on a port outside those ranges with a compose override file, and point the test
variables at it. The recipe in the repository stays on 2222, because CI uses it:

```bash
cd deploy/lab/openssh
docker compose -f compose.yaml -f path/to/override.yaml up -d   # 3222 -> 2222, for example
```

```yaml
services:
  openssh:
    ports:
      - "3222:2222"
```

Recreating the container also recreates its fixtures, among them a directory holding fifty thousand
files; the log line `[ls.io-init] done.` is when it is ready to connect to.

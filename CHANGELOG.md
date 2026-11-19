# Changelog

All notable changes to 7Shell are documented in this file. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

From 0.4.0 each release is written in both languages under one version heading. The GitHub release
body uses the Chinese part; the update manifest keeps both and the application shows the one its
interface is speaking. Older releases stay as they were published.

从 0.4.0 起，每个版本在同一个版本标题下同时写中英文。GitHub Release 正文使用中文章节；更新清单保留两种语言，应用按界面语言显示。更早的版本保持发布时的写法。

## [0.5.0] - 2026-11-20

### English

The compatibility and file-management release. A terminal can speak UTF-8, GBK or GB18030 per
session, an OpenSSH `ProxyJump` chain is followed with every hop's key verified, and the file
manager is rebuilt around the reference's compact listing: type drawings, three columns by default,
a remembered tree and column choice, and a position switch inside each half of the panel.

#### Added

- **Session encoding:** an SSH or Telnet session chooses UTF-8, GBK or GB18030 on its own. Output is
  decoded as a stream, so a character split across two reads is completed instead of drawn as two
  replacement marks; input in another encoding is converted on the host side.
- **Backspace per session:** an SSH session can send Ctrl+H instead of DEL for Backspace.
- **ProxyJump:** a session that names an OpenSSH alias follows the file's `ProxyJump` chain —
  comma-separated hops, `user@host:port`, bracketed IPv6, `none` — recursively, outermost hop
  first. Every hop is probed and compared against the known keys before it authenticates. Saved
  jump hosts still take precedence, and a hop uses its own `IdentityFile` when its Host block names
  one.
- **OpenSSH host filter:** a field above the alias list narrows it by alias, host or account.
- **File manager:** the listing leads with a drawing chosen by extension and, where it matters, by
  the whole name; directories get a solid folder. The header splits into a path row and an action
  row so a narrow panel keeps the whole path, and a parent row above the entries replaces the
  toolbar arrow. The list opens with name, size and modified; the other three columns sit behind
  the column chooser, and the tree, folded columns and sidebar fold are remembered.
- **Positions per half:** files and commands keep separate positions on each terminal tab. The
  position switch lives in the half it moves now, and the terminal strip only toggles the halves.
- **MCP client snippets:** the access tab offers fifteen clients, Qoder first and marked
  Recommended, each with the shape it actually reads — VS Code's `servers`, Codex's TOML,
  DeepSeek Harness's YAML patch, OpenCode's `mcp.local`, Cherry Studio's settings page.

#### Fixed

- **A recorded host key answers beside an older line:** a `known_hosts` file holding several
  records for one host no longer reports a changed key when one record matches the server exactly.
  The vault comparison looks at the algorithm too, so a record of another type reads as unseen
  rather than changed. The mismatch message names the presented fingerprint and the matching
  `ssh-keygen -R` command.
- **macOS update signature:** both macOS builds produced an update archive with the same name, so a
  merged release could pair one architecture's archive with the other's signature — the reason an
  Apple Silicon update from 0.3.0 to 0.4.0 always failed verification. Each macOS archive now
  carries its architecture in its name, matching its manifest entry.
- Trusting a host key from the connection-test notice no longer closes the session editor and
  drops its draft.

#### Notes

- The vault schema is 15. A 0.4.0 build refuses a vault written by 0.5.0; back it up before
  downgrading.
- `ProxyCommand` and `Match` are still refused before a socket opens.
- The browser edition is attached to the same release for amd64 and arm64.
- The installers are not code-signed and the macOS build is not notarized. Nothing in this project
  has had a security review.

### 中文

兼容与文件管理版本。终端可以按会话选择 UTF-8、GBK 或 GB18030，OpenSSH 的 `ProxyJump` 链会被逐跳验键地跟随；文件管理器围绕参考实现的紧凑列表重做：按类型绘制的图标、默认三列、会记住的树与列，以及放在每一半面板里的位置开关。

#### 新增

- **会话编码：** SSH 与 Telnet 会话可以各自选择 UTF-8、GBK 或 GB18030。输出用流式解码，拆在两次读取之间的字符会拼完整，而不是画成两个替换符；其他编码的输入在宿主侧转码。
- **按会话的退格：** SSH 会话可以选择发送 Ctrl+H 而不是 DEL。
- **ProxyJump：** 会话指定 OpenSSH 别名后，会按文件里的 `ProxyJump` 递归跟随——逗号分隔的多跳、`user@host:port`、带方括号的 IPv6、`none`——最外层跳板先连。每一跳都先探测、与已记录的键比对，然后才认证。保存的跳板仍然优先；跳板的 Host 块写了 `IdentityFile` 就用它。
- **OpenSSH 主机筛选：** 别名列表上方的筛选框按别名、主机或账号缩小范围。
- **文件管理器：** 列表按扩展名并视情况按完整文件名取图标，目录使用实心文件夹。头部拆成路径行和动作行，窄面板也能看到完整路径；条目上方的返回上级行取代了工具栏箭头。列表默认显示名称、大小、修改时间，另外三列收在列选择器里；目录树、折叠的列和侧栏折叠都会被记住。
- **两半面板各自定位：** 每个终端标签的文件和命令保持各自的位置。位置开关移入它所移动的那一半，终端条只负责切换两半。
- **MCP 客户端片段：** 接入页提供 15 个客户端，Qoder 排第一并标记推荐；每个客户端用它真正读取的形状——VS Code 的 `servers`、Codex 的 TOML、DeepSeek Harness 的 YAML 补丁、OpenCode 的 `mcp.local`、Cherry Studio 的设置页。

#### 修复

- **已记录的主机键可以与旧记录并存：** `known_hosts` 里同一主机有多条记录时，只要其中一条与服务器完全匹配，就不再报告键已变更。保险库的比对同时看算法，另一种类型的记录按“没见过”处理而不是“已变更”。不一致的提示会给出当前指纹和对应的 `ssh-keygen -R` 命令。
- **macOS 更新签名：** 两条 macOS 构建产出同名的更新归档，合并后的 release 可能把一种架构的归档配上另一种架构的签名——0.3.0 升 0.4.0 在 Apple Silicon 上校验失败就是这个原因。现在每个 macOS 归档在文件名里带上架构，与清单条目一致。
- 从连接测试提示里信任主机键，不再关闭会话编辑器、丢失草稿。

#### 说明

- 保险库 schema 为 15。0.4.0 的版本会拒绝 0.5.0 写出的保险库；降级前请先备份。
- `ProxyCommand` 与 `Match` 仍会在建立套接字前拒绝。
- 浏览器版随同一 release 提供 amd64 与 arm64 归档。
- 安装包未做代码签名，macOS 构建未公证。本项目尚未经过安全审计。

## [0.4.0] - 2026-11-19

### English

The connection and workspace release. A working OpenSSH setup can be reused as it is, the vault can
open with the system keychain, and the workspace gains connection tests, several terminals per host,
a file panel that can move to either side or stand alone, tabs in the title bar, and one chooser for
every new tab.

#### Added

- **OpenSSH configuration:** a session may name an alias from `~/.ssh/config`; its host, account,
  port and identity are resolved on every connection. The system `known_hosts` is read-only: a
  recorded key skips the trust question, and a changed one remains a hard stop.
- **Unlock with the system store:** on Windows and macOS the key derived from the vault password can
  be sealed in the system keychain. A password unlock or change refreshes it, and a failure falls
  back to the password form.
- **Portable self-update:** a portable copy downloads, verifies and replaces itself in place, keeping
  its `data` folder.
- **Connection tests:** SSH sessions can run the complete login without opening a shell, including
  jump hosts and the final host-key decision. Telnet and raw TCP test reachability. Results arrive as
  a transient notice.
- **Several terminals per host:** the tab's duplicate entry and the session menu can open another
  independent terminal for the same host.
- **File panel placement:** each terminal can put its files at the bottom, on either side, or in a
  standalone file-manager tab. The choice is remembered for new terminals; a standalone browser is
  shared by the session and closes only when the last terminal leaves it.
- **Tabs in the title bar:** a fifth tab placement draws the tab strip inside the window's title bar,
  with a shared strip and one scrolling row.
- **New-tab chooser:** the plus opens a searchable session tree for a new tab. Groups fold locally,
  matches always stay visible, and selecting a session that is already open uses the existing
  "open in a new tab / go to it" question.
- **Appearance:** terminal text can carry its own colour per theme, and the wallpaper can show
  through the workspace panels.

#### Fixed

- A vault document written by a newer build now names the newer document instead of reporting a
  wrong password.
- Choosing a background display mode now applies the chosen mode.
- The clear button is available on every list filter.

#### Notes

- The vault schema is 14. A 0.3.0 build refuses a vault written by 0.4.0; back it up before
  downgrading.
- OpenSSH directives this build cannot honour (`ProxyJump`, `ProxyCommand`, `Match`) refuse before a
  socket opens instead of taking a different route.
- A portable copy from 0.3.0 or older must be replaced by hand once; later portable copies update
  themselves.
- The installers are not code-signed and the macOS build is not notarized. Nothing in this project
  has had a security review.

### 中文

连接与工作区版本。已有的 OpenSSH 配置可以直接复用，保险库可以通过系统钥匙串打开；工作区新增连接测试、同一设备的多个终端、可停靠到侧边或独立成页签的文件面板、标题栏标签，以及统一的新建标签页选择器。

#### 新增

- **复用 OpenSSH 配置：** 会话可以直接指定 `~/.ssh/config` 里的别名，主机、账号、端口和身份在每次连接时解析。系统 `known_hosts` 只读生效：已记录的键不再询问，变更的键仍然是硬停。
- **系统钥匙串解锁：** Windows 和 macOS 上，由保险库口令派生的密钥可以封存在系统钥匙串中；口令解锁或修改会刷新它，失败时回退到口令输入。
- **便携版自更新：** 便携副本可以下载、校验并原地替换自己，`data` 目录保持不变。
- **连接测试：** SSH 会话可以在不打开 Shell 的情况下跑完整登录，包含跳板主机和最终主机键确认；Telnet 和原始 TCP 测试可达性。结果用会消失的提示显示。
- **单设备多终端：** 标签页右键的复制入口和会话菜单都可以为同一台主机再开一个独立终端。
- **文件面板位置：** 每个终端可以把文件放在底部、左侧、右侧，或独立的文件管理器页签。位置会记为新终端的默认值；独立浏览器由会话共享，最后一个使用它的终端离开时才收起。
- **标签栏并入标题栏：** 新增第五种标签位置，把标签条画进窗口标题栏，共用同一套标签组件并且只占一行、横向滚动。
- **新建标签页选择器：** 加号打开可搜索的会话树。分组只在本次打开内折叠，搜索结果始终可见；选择已打开的会话时沿用现有的“新标签页打开 / 切到已有标签”询问。
- **外观：** 终端文字可以按主题单独设置颜色，背景壁纸可以透到工作区面板上。

#### 修复

- 更高版本写出的保险库文档现在会明确指出文档版本更新，而不是报告密码错误。
- 选择背景图片的显示方式后会真正应用所选方式。
- 所有列表筛选都提供清除按钮。

#### 说明

- 保险库 schema 为 14。0.3.0 的版本会拒绝 0.4.0 写出的保险库；降级前请先备份。
- 本版本无法支持的 OpenSSH 指令（`ProxyJump`、`ProxyCommand`、`Match`）会在建立套接字前拒绝，而不是悄悄换一条路线。
- 0.3.0 及更早的便携副本需要手动替换一次；之后的便携副本可以自更新。
- 安装包未做代码签名，macOS 构建未公证。本项目尚未经过安全审计。

## [0.3.0] - 2026-11-18

The virtual-machine release. VMware Workstation machines on this computer are part of the library
now: imported with their UUID as identity, shown with their real power state, and driven from the
same workspace and the same agent tools as everything else. The session list also learned to
interleave groups and sessions, and a directory with thousands of entries can be narrowed by name.

### Added

#### Virtual machines

- Scan the Workstation inventory and import machines as sessions, into a chosen group or a new one.
  A machine keeps its UUID when its `.vmx` is renamed or moved.
- Running, suspended and stopped are drawn apart, so a suspended machine resumes instead of
  booting. The workspace starts, resumes, stops and suspends; the power tool adds reset, pause and
  unpause. A lock left by a machine that is not running can be taken over after confirmation.
- SSH, or RDP on Windows, opens from the machine entry. The guest account and password are the
  session's own vault slot.
- Snapshots can be listed, taken, gone back to and deleted; one can be cloned, linked or full, into
  a folder chosen through a host token. `.vmtx` templates are created and deployed the same way.
- Through VMware Tools: list and check paths, copy files both ways, run programs and scripts,
  create, rename and remove, list processes, read the environment, kill a process, and install or
  upgrade Tools.
- Configuration covers CPU, memory and cores, disks to create or extend, network and serial
  adapters, SATA and NVMe controllers, shared folders, VProbes (enable, load, reset), configuration
  entries, device connection and runtime variables.
- On the host: virtual networks and port forwards, console screenshots, key sequences that need no
  guest account, and the Workstation REST service for machine reads, MAC-to-IP maps and network
  creation.
- Host locations are opaque tokens whose paths stay in the vault; imported machines are reached by
  session id. The vault schema is now 13.

#### Agent tools

- 17 built-in VMware tools cover inventory, status and power, snapshots and clones, guest reads and
  writes, console keys, configuration, host networking, templates and images, screenshots and the
  REST reads. Destructive calls stop for the same approval and are written to the same audit log as
  every other tool.

#### Files

- The open directory can be filtered by name: `Ctrl+F` or the search button turns the address bar
  into a field, matching files and folders without case; Enter hands the keyboard to the results,
  Escape puts the whole directory back, and walking into another directory drops the filter.

#### Sessions

- Groups and sessions now share one order. A dragged row lands beside the row it was released on, a
  group can sit between two sessions, pinned groups keep their band, and existing vaults are
  numbered the way they were already drawn.

### Fixed

- Workstation 26H1 installs under 64-bit `Program Files`; that folder is asked before the legacy
  one, a registry entry only counts when the binary is really there, and a missing module command
  names `VMCLI_PATH`.
- Guest operations read the password from the session's vault slot rather than the global one, which
  made an unlocked vault answer `VAULT-LOCKED`.
- Snapshot revert and delete resolve the uid the query prints (`-d` for children), a clone creates
  its destination folder, and reset is hard so it works before Tools answers.
- Console capture uses `vmcli MKS`, which works before a guest has Tools or an account.
- The Workstation REST service no longer returns `.vmx` paths in its answers.

### Notes

- The vault is at schema 13. A 0.2.0 build refuses a vault written by 0.3.0; downgrading means
  restoring a backup.
- The installers are not code-signed and the macOS build is not notarized. Windows SmartScreen and
  macOS Gatekeeper will warn; check the SHA-256 in the release notes.
- The browser edition is attached to the same release for amd64 and arm64 as `*-web.tar.gz`, built
  against the glibc 2.28 baseline. It is not part of the desktop installers.
- Signed update archives and `latest.json` are attached when the signing key is configured. The
  update channel changes only when the release is promoted.

## [0.2.0] - 2026-11-17

The browser-host release. The interface now runs in a normal browser from a loopback host, so
machines without the newer WebKit — older or locked-down Linux, including UOS and Kylin — can use
the application without a desktop shell. Terminals, files and tab handling were reworked around
how people actually use them: one shared command history, tabs that survive a dropped connection,
and a file panel whose header is a single compact strip.

### Added

#### Browser host

- A loopback-only host (`sevenshell-web`) that serves the built interface and answers the same
  command names over HTTP and a WebSocket. One transport layer, `ipc.ts`, selects the backend, so
  the interface is the same code in both hosts.
- The vault, sessions and groups, known hosts, keys, SSH terminals with host-key decisions, SFTP
  browsing and text editing, transfers, settings and application facts all work in the browser.
- The host monitor, batch execution, the command library, container and Kubernetes workspaces,
  sync, key management, SFTP upload and download, drag-and-drop upload, and container and pod file
  browsers are part of the browser surface as well.
- The assistant and external MCP servers are available in the browser: the same tool registry,
  approval prompts and audit log as the desktop, an MCP listener and stdio bridge, OAuth sign-in
  for external servers with token refresh and reconnect, resources, prompts and resource
  templates, per-session agent policy, bounded context, and the same run limits (turns, one tool
  per turn, result truncation).
- Terminal, assistant and MCP events are delivered in the desktop's envelope, so every listener
  behaves identically in both hosts.
- Downloads started in the browser appear in the transfer panel with progress, and Stop cancels
  them. ZMODEM sends and receives through the browser's own file doors.

#### Files

- The address bar is one row: breadcrumbs navigate, and the empty part of the bar or `Ctrl+L`
  turns it into a field for typing a path. Enter or Go travels; Escape or leaving the field returns
  to the breadcrumbs without navigating.
- The panel header is a compact icon strip: refresh the directory, go up, new file, new folder,
  download (only once something is selected), upload, and upload from the clipboard.
- Directory bookmarks: named remote directories kept in the settings file and reachable in one
  click from the path bar's menu.
- The panel follows a `cd` typed at the keyboard, accepted from completion, sent from the command
  bar, or sent from the command library.
- A file browser opens a connection of its own when the terminal's connection has no room for
  another channel, instead of failing when the server reaches its per-connection limit.
- Opening a binary or oversized file as text reports the editor's answer rather than a failed
  transfer.

#### Terminals and tabs

- The tab menu offers duplicating a tab, reconnecting it, reconnecting every closed terminal,
  disconnecting while keeping the tab, closing it, closing the others, closing left, closing right
  and closing all.
- Reconnecting revives the tab in place: the same id, position and title, the same scrollback, and
  the new session writes below what the old one left on screen.
- Command history is shared by every terminal window. Lines run at a prompt are recorded — except
  at password, one-time-code and confirmation prompts — deduplicated, persisted, and offered back
  from the command bar. The history list closes when a click lands anywhere else.
- The host monitor mounts only where a sample can ride an SSH connection, and comes back by itself
  when a monitorable session is on screen again. Switching it off still means switched off.
- An SSH channel failure names the channel rather than a session, which is what made a full
  connection read as an unopenable terminal.

#### Containers and clusters

- A container page keeps only the namespace picker a runtime supports, and a cluster page keeps
  its endpoint and connection state. The session's name and kind live in the tab, not in a second
  heading inside the page.

### Fixed

- Browser-host events reached listeners without the desktop envelope, which left terminals blank
  and assistant deltas undefined.
- An external MCP server reconnects after its OAuth token is refreshed.
- A disconnected terminal tab keeps its last screen and its state; it no longer shows the
  open-failure card underneath, and reconnecting no longer replaces the tab with a new one.

### Notes

- The installers are still not code-signed and the macOS build is not notarized; Windows
  SmartScreen and macOS Gatekeeper will warn. Check the SHA-256 in the release notes.
- The browser host is a separate binary (`sevenshell-web`) and is not part of the desktop
  installers; it is built and packaged on its own. The Windows, macOS and Linux desktop
  installers are unchanged in shape.
- Signed update archives and `latest.json` are attached when the release signing key is
  configured. This is the first release that can seed the update channel once it is promoted.

## [0.1.0] - 2026-11-13

The first public release: a desktop SSH and operations client for Windows, macOS and Linux.
Sessions, terminals, files, containers, clusters and host readings share one application, and
everything the application knows stays on this machine.

### Added

#### Sessions and terminals

- Saved sessions in an encrypted vault, with groups, search, import/export and one passphrase.
  Argon2id derives the key and XChaCha20-Poly1305 encrypts the contents; the passphrase is never
  stored.
- SSH with a password, private key, keyboard-interactive answers or the system key agent. Jump
  hosts, local port forwarding (`-L`) and a SOCKS5 proxy (`-D`) are part of the saved session.
- Telnet, raw TCP, serial, local shells (PowerShell, Command Prompt, Git Bash and the Unix login
  shell) and WSL on Windows.
- Tabs, search, links, keep-alive, reconnect, asciicast v2 recording, 190 colour schemes,
  typography, cursor settings and keyword highlighting seeded with 22 rules.
- Host keys are verified before authentication. An unknown key is a decision; a changed key stops
  the connection.

#### Files

- A two-pane SFTP workbench: create, rename, delete, edit text in place, upload and download.
- A transfer queue with pause, resume and retry from where a transfer stopped. Transfers outlive
  the tab that started them, and a directory can travel as one archive over SSH.
- WebDAV and FTP/FTPS folders in the same workbench, SCP fallback when a server has no SFTP
  subsystem, and ZMODEM inside the terminal with the system file dialogs.

#### Operations

- A host monitor for load, CPU, memory, swap, disks, processes, network and sockets, sampled over
  the connection that is already open. Nothing is installed on the server.
- Batch execution: one command on several hosts, a result per host and a risk verdict before it
  runs.
- A command library with `#1`–`#5` slots, history and completion.
- Sync sessions and commands to a WebDAV server, or to a snippet on GitHub, Gitee or GitLab.
- RDP on Windows through the system Remote Desktop client.

#### Containers and clusters

- Docker, Podman, nerdctl and WSLC: containers, images, logs, stats, an exec terminal, files and
  image-archive import, on this machine or through an SSH session.
- Kubernetes: kubeconfig contexts, resource lists and watches, pod logs, exec, pod files, metrics
  and YAML editing, directly or through an SSH session.

#### Assistant and MCP

- An in-app assistant for Anthropic, OpenAI-compatible endpoints and local models.
- An MCP server for external clients. The assistant and MCP clients use the same tool registry,
  risk rules, approval prompts and audit log; credentials stay in the vault.

#### Packages

- Windows x64 and ARM64: MSI, NSIS, portable ZIP and a WebView2-offline NSIS installer.
- macOS Apple silicon and Intel: DMG, plus an update archive when the release signing key is
  configured.
- Linux x64 and ARM64: AppImage, deb and rpm.
- `latest.json` covers all six platform keys when the release signing key is configured.

### Notes

- The installers are not code-signed and the macOS build is not notarized, so Windows SmartScreen
  and macOS Gatekeeper will warn. Check the SHA-256 in the release notes before running one.
- A portable Windows build keeps its vault, settings and MCP bridge in a `data` folder beside the
  executable.

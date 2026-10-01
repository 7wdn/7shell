# Translation glossary

One concept, one word, in every language. Inconsistent vocabulary is the most common way a
translated interface starts to feel unreliable — "tunnel" in one dialog and "forward" in the
next reads as two different features.

## English

| Term | Meaning | Not this |
|---|---|---|
| **host** | A machine you connect to, identified by name or address. | server, box, machine |
| **session** | One live connection with its terminal or file browser. | connection (a session *uses* a connection) |
| **connection** | The configured way to reach a host: protocol, address, auth, jump chain. | profile, bookmark |
| **vault** | The encrypted file holding credentials and settings. | keychain, store |
| **unlock / lock** | Deriving the vault key from the password; discarding it. | open/close, login |
| **host key** | The server's public key, verified before authenticating. | fingerprint (that is a property of a key) |
| **fingerprint** | The short hash shown for a key. | hash, thumbprint |
| **trust** | Recording a host key so it is accepted next time. | accept, allow |
| **jump host** | An intermediate machine a connection goes through. | bastion, proxy (a proxy is not a jump host) |
| **proxy** | SOCKS5 or HTTP CONNECT used to reach the jump host or the target. | jump host |
| **port forwarding / tunnel** | A local or remote port mapped over a connection. | proxy |
| **capability** | Something the product can do with a protocol. | feature, ability |
| **host monitor** | The panel reporting the connected host's load, memory, network and file systems. | dashboard, metrics |
| **sample** | One reading of a host, taken by running commands over an open connection. | poll, probe |
| **load average** | The 1, 5 and 15 minute run-queue averages. | CPU usage (a different figure) |
| **swap** | Disk space used as memory when there is not enough of the latter. | virtual memory, page file |
| **descriptor** | The YAML declaration a capability or protocol comes from. | config, manifest (the manifest is generated) |
| **policy** | The rules deciding whether an action is allowed. | permission (that is one input to a decision) |
| **approval** | A person saying yes to a specific action. | confirmation, consent |
| **grant** | Access to a local file the user picked, as an opaque token. | permission, path |
| **envelope** | The structured error that crosses a boundary. | error object, payload |

## 简体中文

| 术语 | 译法 | 不要译成 |
|---|---|---|
| host | 主机 | 服务器、机器 |
| session | 会话 | 连接 |
| connection | 连接 | 配置、书签 |
| vault | 保险库 | 密钥库、钥匙串 |
| unlock / lock | 解锁／锁定 | 打开／关闭 |
| host key | 主机键 | 指纹（指纹是主机键的一个属性） |
| fingerprint | 指纹 | 哈希、指纹码 |
| trust | 信任 | 接受、允许 |
| jump host | 跳板机 | 堡垒机、代理 |
| proxy | 代理 | 跳板机 |
| port forwarding / tunnel | 端口转发／隧道 | 代理 |
| capability | 能力 | 功能、特性 |
| host monitor | 主机监控 | 仪表盘、监控台 |
| sample | 采样 | 轮询、探测 |
| load average | 负载 | CPU 占用（那是另一个数） |
| swap | 交换 | 虚拟内存、页面文件 |
| descriptor | 描述符 | 配置、清单 |
| policy | 策略 | 权限 |
| approval | 审批 | 确认、同意 |
| grant | 授权 | 权限 |
| envelope | 错误信封 | 错误对象 |

## Rules

1. **Product nouns stay in the code.** "host", "session", "vault" and "capability" name things
   in the wire format and the file formats; the interface uses the same words so a bug report
   and the code agree.
2. **Translate the idea, not the word.** `errors.vault.kdfFailed` is "The vault key could not be
   derived", not "KDF failed" — a user does not need to know what a KDF is, and an error message
   that names its implementation is an error message nobody can act on.
3. **Commands and file paths are not translated.** `ssh`, `sftp`, `~/.ssh/config` and friends
   stay as they are, in every language.
4. **Keep the parameters.** `{host}`, `{port}` and friends must survive translation; the i18n
   check does not verify placeholders yet, so this is a review item for now.

# 7Shell

**面向 Windows、macOS 与 Linux 的开源 SSH 与运维控制台。**

*[English](README.md)*

> **状态：早期开发中。** 工程基座、错误契约与描述符管线已经就位，但目前还不能连接任何服务器，
> 从未发布过，也没有经过安全审计。**请不要用它保存生产环境凭据。**

## 它要成为什么

- **对安全诚实的会话。** 凭据存放在加密且需要密码解锁的保险库里；服务器的主机键在**发送任何内容之前**
  先校验，主机键发生变化时直接硬停，而不是弹一个可以随手点掉的提示。
- **真正可用的终端。** 多标签、可调整尺寸、可搜索；切换标签不丢回滚内容；会话结束后仍留在屏幕上，
  让你先看清发生了什么再重连。
- **文件工作台，而不只是文件列表。** 浏览、传输、就地编辑，并且告诉你传输为什么失败，
  而不是让进度条一直卡着。
- **成规模的运维能力。** 跳板机、代理、端口转发、多主机批量执行、片段库，以及一个会明确说出
  "这条命令是破坏性的"的命令风险分级器。
- **体积小、无运行时依赖。** Rust + Tauri 单一二进制，不需要 JRE、Python 或内置浏览器运行时。

## 目前已经完成

| 部分 | 状态 |
|---|---|
| 工作区、构建、测试与检查工具链 | 已就位 |
| 错误契约（稳定错误码、文案 key、机密脱敏） | 已就位，带测试 |
| 描述符管线（YAML 单一真源 → Rust 枚举 + TypeScript 类型） | 已就位，CI 检查漂移 |
| 界面外壳（Vue 3、设计令牌、中英双语、后端门面） | 已就位 |
| 保险库、SSH、终端、SFTP、隧道 | 尚未实现 |

运行中的应用也如实说明了这一点：首屏那份能力清单直接读自描述符，未实现的能力会被明确标注。

## 如何构建

依赖：Rust（版本由 `rust-toolchain.toml` 固定）、Node 22 以上、pnpm 11；
Windows 上还需要 MSVC 工具链与 WebView2（Windows 10 及以上自带）。

```bash
pnpm install
pnpm dev:tauri        # 桌面应用
pnpm dev              # 仅界面（浏览器中运行，无后端）
```

```bash
pnpm check              # 全部门禁：描述符、i18n、版本、来源、守卫、lint、类型、测试、Rust
pnpm test               # 界面测试
cargo test --workspace  # Rust 测试
pnpm build              # 界面产物
pnpm icons              # 重新绘制并生成应用图标
```

`make` 目标只是这些命令的薄封装，供习惯 make 的人使用。

## 项目结构

| 目录 | 内容 |
|---|---|
| `apps/desktop/` | 桌面应用使用的 Vue 3 界面 |
| `crates/` | Rust：错误契约、有线类型，以及随后落地的保险库、SSH、终端、SFTP 与策略层 |
| `src-tauri/` | 桌面外壳：窗口、命令面、Tauri 运行时 |
| `descriptors/` | Rust 与 TypeScript 必须一致的所有内容的唯一真源 |
| `packages/` | 跨层测试，以及之后的命令行与 Agent 接口 |
| `scripts/` | 生成、校验与发布工具 |
| `docs/` | 架构、安全模型、约定 |

两条规则解释了大部分结构：

1. **Rust 核心不得依赖 Tauri。** 宿主（今天的桌面外壳，之后的命令行与 Agent 接口）依赖核心，
   而不是反过来。守卫测试会强制这一点，这样这些操作才能被"非窗口"的东西调用。
2. **两种语言必须一致的内容只声明一次。** 协议、认证方式与能力都写在 `descriptors/**/*.yaml`，
   由生成器产出 Rust 枚举与 TypeScript 类型；一旦漂移，`pnpm check:descriptors` 就会失败。

分层说明见 `docs/architecture.md`，代码必须遵守的不变量见 `docs/security-model.md`，
新增协议的方法见 `docs/descriptors.md`。

## 参与贡献

欢迎提交缺陷报告、安全问题与代码贡献——请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)
与 [SECURITY.md](SECURITY.md)。提 PR 前请记住两件事：`docs/security-model.md` 中的不变量
不经讨论不可改动；每个面向用户的文案都需要在两种语言里都有对应的 key。

## 许可

Apache-2.0，详见 [LICENSE](LICENSE)。

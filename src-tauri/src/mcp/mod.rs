//! The desktop side of the agent interface.
//!
//! The desktop is the only process with the vault, the known hosts and the live sessions, so it is
//! the process the MCP bridge talks to. It listens on `127.0.0.1` — the configured port or one the
//! system assigns — exposes three routes on loopback (`POST /tools/call`, `GET /tools` and
//! `POST /tools/cancel`), and writes down the port and the token for this run in
//! `<config_dir>/mcp-endpoint.json`. The bridge reads that file; the interface never does.
//!
//! The pieces, and why they are apart:
//!
//! * [`auth`] — the token and the header checks. Pure functions, so the boundary is testable without
//!   a socket.
//! * [`endpoint`] — the file the bridge reads. Owner-only and replaced atomically.
//! * [`server`] — the HTTP layer and the adapter onto `sevenshell_api`'s registry. The handler is a
//!   trait, so the layer can be exercised with a fake tool and no window.
//! * [`approval`] — the question a destructive call asks, answered by id through the `mcp:approval`
//!   event.
//! * [`audit`] — one JSONL line per call, rotated by size and pruned by age.
//! * [`runtime`] — the listener's lifetime, owned by [`crate::state::ShellState`].
//!
//! Two things belong to this layer rather than to a tool:
//!
//! * **The connection cache.** One [`sevenshell_api::local::ConnectionCache`] per run, shared by
//!   every call, so a conversation is one authenticated connection per session. The runtime owns it
//!   and empties it when the vault locks or the listener stops.
//! * **The approval window.** Destructive calls ask through `mcp:approval` and wait for
//!   `mcp_approval_answer`. Nothing is remembered: "yes, always" is a settings decision, not a side
//!   effect of one click.
//!
//! The token is a secret and never crosses to the interface. [`McpStatus`] names the port, the scope
//! and the audit file's path — the things a person needs to understand what is on — and nothing else.

pub mod approval;
pub mod audit;
pub mod auth;
pub mod endpoint;
mod runtime;
pub mod server;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde::Serialize;
use sevenshell_error::ShellError;
use sevenshell_types::{AppSettings, McpScope};
use tauri::{AppHandle, Emitter as _};

pub use runtime::McpRuntime;

/// The event the interface listens on for the listener's own state.
pub const STATUS_EVENT: &str = "mcp:status";

/// The agent interface, as the settings page sees it.
///
/// `enabled` means *listening*, not "the setting says it should be": a start that failed to bind is
/// a switch that must show off, because an agent cannot reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    /// The bound port, `None` when nothing is listening.
    pub port: Option<u16>,
    pub scope: McpScope,
    /// The snippet to paste into an MCP client, without the token. Empty when the bridge cannot be
    /// located; [`client_config`] is where that becomes a sentence.
    pub client_config: String,
    pub audit_path: String,
    pub pending_approvals: usize,
}

/// The file the installer places beside the application. It is a seed, not the program a client
/// starts: an application update replaces this file while an MCP client may be running the copy in
/// [`bridge_path`], and a running executable cannot be overwritten on Windows.
const BRIDGE_SEED_NAME: &str = if cfg!(windows) {
    "sshell-mcp-seed.exe"
} else {
    "sshell-mcp-seed"
};

/// The name the bridge has in its own directory, which is what a client starts.
const BRIDGE_NAME: &str = if cfg!(windows) { "sshell-mcp.exe" } else { "sshell-mcp" };

/// The client configuration snippet, without the token.
///
/// The command is the bridge's own path, not the seed beside the application. An app update may
/// replace the seed while a client is running the bridge; keeping those two files apart is what
/// makes the update possible. When no bridge can be prepared this fails rather than naming a path
/// that is not a program: a snippet pointing at nothing is a client that starts nothing.
pub fn client_config() -> Result<String, ShellError> {
    let bridge = bridge_path()?;
    let endpoint = endpoint::path()?;
    Ok(serde_json::json!({
        "mcpServers": {
            "7shell": {
                "command": bridge.display().to_string(),
                "env": {
                    "SEVENSHELL_ENDPOINT_FILE": endpoint.display().to_string(),
                },
            },
        },
    })
    .to_string())
}

/// Makes the bridge available in its own directory and answers where it is.
///
/// Release builds copy the seed to the per-user home dot directory. A debug build returns the
/// seed itself unless a test supplied [`sevenshell_platform::BRIDGE_DIR_OVERRIDE`], so running the
/// development application does not replace the bridge an installed release left for a client.
///
/// The prepared path is remembered for this process: a status event asks for it repeatedly, and
/// copying or comparing the same five megabytes on every event is work nobody asked for. A copy
/// that could not replace a running bridge is not an error — the old bridge still works and the
/// next launch tries again; only a missing bridge with nothing to fall back to is one.
pub fn bridge_path() -> Result<PathBuf, ShellError> {
    static PREPARED: OnceLock<PathBuf> = OnceLock::new();
    if let Some(path) = PREPARED.get() {
        return Ok(path.clone());
    }

    let path = prepare_bridge()?;
    let _ = PREPARED.set(path.clone());
    Ok(path)
}

fn prepare_bridge() -> Result<PathBuf, ShellError> {
    let seed = seed_path()?;
    let seed_ready = non_empty_file(&seed);
    let uses_own_directory =
        !cfg!(debug_assertions) || std::env::var_os(sevenshell_platform::BRIDGE_DIR_OVERRIDE).is_some();

    if !uses_own_directory {
        return if seed_ready {
            Ok(seed)
        } else {
            Err(missing_bridge(&seed))
        };
    }

    let directory = sevenshell_platform::bridge_dir()
        .ok_or_else(|| ShellError::app_internal("the bridge directory could not be located on this machine"))?;
    let target = directory.join(BRIDGE_NAME);

    if seed_ready {
        match install_bridge_from(&seed, &target) {
            Ok(_) => return Ok(target),
            Err(error) => {
                // The old bridge is still a working program; an update that cannot replace it is
                // deferred rather than turned into an application that cannot start.
                if non_empty_file(&target) {
                    log::warn!("the MCP bridge could not be replaced; the existing one stays: {error}");
                    return Ok(target);
                }
                return Err(error);
            }
        }
    }

    if non_empty_file(&target) {
        return Ok(target);
    }
    Err(missing_bridge(&seed))
}

/// Where the installer put the seed. It is beside the application executable in every packaged build.
fn seed_path() -> Result<PathBuf, ShellError> {
    let executable = std::env::current_exe()
        .map_err(|error| ShellError::app_internal(format!("this executable's own path could not be read: {error}")))?;
    let directory = executable
        .parent()
        .ok_or_else(|| ShellError::app_internal("this executable has no directory"))?;
    Ok(directory.join(BRIDGE_SEED_NAME))
}

fn missing_bridge(seed: &Path) -> ShellError {
    ShellError::app_internal(format!(
        "the MCP bridge is missing or empty (looked for {}); run `pnpm build:sidecar` or install a packaged 7Shell",
        seed.display()
    ))
}

fn non_empty_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

/// Copies a seed to its final path, answering whether the file changed.
///
/// The target is replaced by renaming the old file aside and the new one into place. Windows permits
/// renaming a running executable even though it cannot be overwritten; if any step fails the old
/// path is put back, so a failed update never leaves the client without a bridge.
fn install_bridge_from(seed: &Path, target: &Path) -> Result<bool, ShellError> {
    if !non_empty_file(seed) {
        return Err(ShellError::app_internal(format!(
            "the MCP bridge seed is missing or empty: {}",
            seed.display()
        )));
    }
    if target.is_file() && std::fs::read(seed).ok() == std::fs::read(target).ok() {
        return Ok(false);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ShellError::app_internal(format!(
                "the bridge directory {} could not be created: {error}",
                parent.display()
            ))
        })?;
    }

    let temporary = target.with_file_name(format!(
        "{}.{}.tmp",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sshell-mcp"),
        std::process::id()
    ));
    std::fs::copy(seed, &temporary).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        ShellError::app_internal(format!(
            "the MCP bridge could not be staged at {}: {error}",
            temporary.display()
        ))
    })?;

    let backup = target.with_file_name(format!(
        "{}.old",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sshell-mcp")
    ));
    let had_target = target.is_file();
    if had_target {
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(target, &backup).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            ShellError::app_internal(format!(
                "the existing MCP bridge {} could not be moved aside: {error}",
                target.display()
            ))
        })?;
    }

    if let Err(error) = std::fs::rename(&temporary, target) {
        if had_target {
            let _ = std::fs::rename(&backup, target);
        }
        let _ = std::fs::remove_file(&temporary);
        return Err(ShellError::app_internal(format!(
            "the MCP bridge could not be moved into place at {}: {error}",
            target.display()
        )));
    }

    // A running old bridge keeps the backup open; leaving it for the next update is harmless and
    // better than turning a successful replacement into a failure.
    let _ = std::fs::remove_file(&backup);
    Ok(true)
}

/// Starts the listener and tells the interface.
pub async fn enable(app: &AppHandle, runtime: &McpRuntime, settings: &AppSettings) -> Result<McpStatus, ShellError> {
    let status = runtime.start(app, settings).await?;
    emit_status(app, &status);
    Ok(status)
}

/// Stops the listener and tells the interface.
pub async fn disable(app: &AppHandle, runtime: &McpRuntime, settings: &AppSettings) -> McpStatus {
    runtime.stop().await;
    let status = runtime.status(settings).await;
    emit_status(app, &status);
    status
}

/// Makes the listener follow a settings change.
///
/// Called from `settings_set`, which is the one place the file is written. Only the five things that
/// describe the listener restart it; a change to the font or the window must not interrupt a call.
pub async fn apply_settings(
    app: &AppHandle,
    runtime: &McpRuntime,
    previous: &AppSettings,
    next: &AppSettings,
) -> Result<McpStatus, ShellError> {
    if !next.mcp_enabled {
        return Ok(disable(app, runtime, next).await);
    }

    // A start that failed leaves the setting on and the listener off; the next settings write is an
    // opportunity to try again rather than a reason to keep pretending.
    if !previous.mcp_enabled || listener_changed(previous, next) || !runtime.is_running().await {
        runtime.stop().await;
        return enable(app, runtime, next).await;
    }

    Ok(runtime.status(next).await)
}

/// Starts the listener when the settings asked for it at launch.
///
/// Spawned rather than awaited: `setup` is synchronous and the window should not wait on a socket.
/// A failure is logged and shown as "off" by the first status the interface asks for.
pub fn start_if_enabled(app: &AppHandle, runtime: Arc<McpRuntime>, settings: &AppSettings) {
    if !settings.mcp_enabled {
        return;
    }

    let app = app.clone();
    let settings = settings.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = enable(&app, &runtime, &settings).await {
            log::warn!("the agent interface could not start: {error}");
        }
    });
}

/// Whether two settings describe a different listener.
fn listener_changed(previous: &AppSettings, next: &AppSettings) -> bool {
    previous.mcp_port != next.mcp_port
        || previous.mcp_scope != next.mcp_scope
        || previous.mcp_approval_timeout_seconds != next.mcp_approval_timeout_seconds
        || previous.mcp_audit_retention_days != next.mcp_audit_retention_days
        || previous.mcp_max_result_bytes != next.mcp_max_result_bytes
}

fn emit_status(app: &AppHandle, status: &McpStatus) {
    if let Err(error) = app.emit(STATUS_EVENT, status) {
        log::warn!("the agent interface's status could not be sent to the window: {error}");
    }
}

#[cfg(test)]
mod bridge_tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("a test file");
    }

    #[test]
    fn a_seed_is_copied_to_the_bridge_directory() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let seed = directory.path().join("seed.exe");
        let target = directory.path().join("bin").join("sshell-mcp.exe");
        write(&seed, b"new bridge");

        assert!(install_bridge_from(&seed, &target).expect("the seed is installed"));
        assert_eq!(fs::read(&target).expect("the target exists"), b"new bridge");
    }

    #[test]
    fn an_unchanged_bridge_is_not_rewritten() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let seed = directory.path().join("seed.exe");
        let target = directory.path().join("bridge.exe");
        write(&seed, b"same bytes");
        write(&target, b"same bytes");

        assert!(!install_bridge_from(&seed, &target).expect("equal files are left alone"));
        assert_eq!(fs::read(&target).expect("the target exists"), b"same bytes");
    }

    #[test]
    fn a_changed_bridge_replaces_the_old_file() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let seed = directory.path().join("seed.exe");
        let target = directory.path().join("bridge.exe");
        write(&seed, b"new");
        write(&target, b"old");

        assert!(install_bridge_from(&seed, &target).expect("the new bridge is installed"));
        assert_eq!(fs::read(&target).expect("the target exists"), b"new");
        assert!(
            !target.with_file_name("bridge.exe.old").exists(),
            "the backup is cleared"
        );
    }

    #[test]
    fn an_empty_seed_is_refused_rather_than_installed() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let seed = directory.path().join("seed.exe");
        let target = directory.path().join("bridge.exe");
        write(&seed, b"");

        assert!(install_bridge_from(&seed, &target).is_err());
        assert!(!target.exists());
    }
}

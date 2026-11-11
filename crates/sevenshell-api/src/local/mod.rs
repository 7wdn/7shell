//! The tools that run against this installation's own core.
//!
//! One module per family, and one place that lists them. The order of `register` is the order a
//! reader sees; the registry keeps them by name, so the order on the wire is alphabetical and does
//! not depend on this function.
//!
//! Every tool here is described by its own module: what it takes, what it answers, and what it asks
//! before it acts. Three rules are shared rather than repeated, and they live in three places:
//!
//! * [`gate`] — how long a call may take, how much of an answer is returned, what an audit line may
//!   say, and the two questions a person is asked (a destructive verdict, an unknown host key).
//! * [`cache`] — the file, container and cluster connections a run of calls reuses, and the one
//!   reopen that turns a stale handle into an answer instead of a failure.
//! * [`session_id`] — the one argument check every tool needs: an empty id is a refusal here rather
//!   than a "no such session" from the vault, which is a true answer to a question nobody asked.

pub mod cache;
mod containers;
mod exec;
mod files;
mod gate;
mod k8s;
mod sessions;

use std::sync::Arc;

use crate::error::ApiError;
use crate::registry::ToolRegistry;
use crate::tool::ToolError;

pub use cache::ConnectionCache;
pub use containers::{ContainerAction, ContainerActionInput, ContainersListInput};
pub use exec::{BatchExecInput, ExecRunInput};
pub use files::{SftpListInput, SftpReadTextInput, SftpWriteTextInput};
pub use k8s::K8sGetInput;
pub use sessions::{SessionOpenInput, SessionsListInput, SessionsListKind};

/// Every tool this build declares.
pub fn register(registry: &mut ToolRegistry) -> Result<(), ApiError> {
    registry.register(Arc::new(sessions::SessionsListTool))?;
    registry.register(Arc::new(sessions::SessionOpenTool))?;
    registry.register(Arc::new(exec::ExecRunTool))?;
    registry.register(Arc::new(exec::BatchExecTool))?;
    registry.register(Arc::new(files::SftpListTool))?;
    registry.register(Arc::new(files::SftpReadTextTool))?;
    registry.register(Arc::new(files::SftpWriteTextTool))?;
    registry.register(Arc::new(containers::ContainersListTool))?;
    registry.register(Arc::new(containers::ContainerActionTool))?;
    registry.register(Arc::new(k8s::K8sGetTool))?;
    Ok(())
}

/// The registry a host starts from.
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register(&mut registry).expect("the built-in tool names are unique and non-empty");
    registry
}

/// A session id, trimmed or refused.
///
/// An empty id would otherwise travel to the vault and come back as "no such session", which is a
/// true answer to a question nobody meant to ask. Refusing it here says what is actually wrong.
pub fn session_id(id: &str) -> Result<&str, ToolError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(ToolError::invalid_arguments("sessionId is empty"));
    }
    Ok(trimmed)
}

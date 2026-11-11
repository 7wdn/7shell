//! The list of tools, and the gate every call goes through.
//!
//! The registry is where three things happen in one order, and the order is the design:
//!
//! 1. **The name is looked up.** A tool this build does not have is refused before anything runs.
//! 2. **The scope is checked.** A read-only scope does not offer a write tool, and the refusal is
//!    an answer rather than a missing tool: an agent can see that the tool exists and that the
//!    person in front of the machine has not allowed it.
//! 3. **The tool decides the rest.** A destructive command still stops inside the tool for an
//!    approval; a changed host key stops it outright.
//!
//! Every outcome — including the two refusals above — becomes one audit line, written here rather
//! than by each tool, so "every call is audited" is a property of the list and not of anybody's
//! memory.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use sevenshell_types::McpScope;

use crate::context::{AuditRecord, AuditStatus, ToolContext};
use crate::error::ApiError;
use crate::tool::{ToolAccess, ToolCall, ToolDescriptor, ToolError, ToolOutcome, ToolResult};

/// One operation an agent may call.
///
/// Boxed behind `Arc<dyn Tool>`: the registry holds a heterogeneous list, and the only two things
/// it needs from an entry are its descriptor and one way to run it.
#[async_trait]
pub trait Tool: Send + Sync {
    fn descriptor(&self) -> ToolDescriptor;

    /// Runs the call. Deserializing the arguments into the tool's own input happens here, so a tool
    /// cannot accidentally accept an argument it did not declare.
    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError>;
}

/// The tools this build offers, by name.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a tool. A second tool with the same name is refused rather than replacing the first:
    /// silently shadowing one would make the list depend on registration order.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ApiError> {
        let name = tool.descriptor().name;
        if name.trim().is_empty() {
            return Err(ApiError::EmptyToolName);
        }
        if self.tools.contains_key(&name) {
            return Err(ApiError::DuplicateTool { name });
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Every descriptor, in name order.
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools.values().map(|tool| tool.descriptor()).collect()
    }

    /// One descriptor, when the registry has it.
    pub fn descriptor(&self, name: &str) -> Option<ToolDescriptor> {
        self.tools.get(name).map(|tool| tool.descriptor())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Runs one call, and records what happened however it happened.
    pub async fn call(&self, context: &ToolContext, call: ToolCall) -> ToolOutcome {
        let started = Instant::now();

        let Some(tool) = self.tools.get(&call.name).cloned() else {
            let error = ToolError::unknown_tool(&call.name);
            context.audit(
                AuditRecord::new(call.name, context.scope(), AuditStatus::Failed)
                    .with_origin(context.origin())
                    .with_duration_ms(millis(started.elapsed()))
                    .with_error_code(error.code().to_owned()),
            );
            return ToolOutcome::failed(error);
        };

        let descriptor = tool.descriptor();
        if !scope_allows(context.scope(), descriptor.access) {
            let error = ToolError::denied(&descriptor.name);
            context.audit(
                AuditRecord::new(descriptor.name, context.scope(), AuditStatus::Denied)
                    .with_origin(context.origin())
                    .with_duration_ms(millis(started.elapsed()))
                    .with_error_code(error.code().to_owned()),
            );
            return ToolOutcome::failed(error);
        }

        match tool.call(context, call.arguments).await {
            Ok(result) => {
                context.audit(
                    AuditRecord::new(descriptor.name, context.scope(), AuditStatus::Succeeded)
                        .with_origin(context.origin())
                        .with_duration_ms(millis(started.elapsed()))
                        .with_audit(result.audit.clone()),
                );
                ToolOutcome::succeeded(result)
            }
            Err(error) => {
                // A policy refusal is a decision, not a malfunction: the audit's own word for it is
                // `denied`, which is also what the settings page shows. Every other failure keeps
                // `failed` so the two are never confused when somebody reads the trail later.
                let status = if error.code() == "POLICY-DENIED" {
                    AuditStatus::Denied
                } else {
                    AuditStatus::Failed
                };
                context.audit(
                    AuditRecord::new(descriptor.name, context.scope(), status)
                        .with_origin(context.origin())
                        .with_duration_ms(millis(started.elapsed()))
                        .with_error_code(error.code().to_owned())
                        .with_audit(error.audit().cloned()),
                );
                ToolOutcome::failed(error)
            }
        }
    }
}

/// Whether a scope offers a tool with this access.
///
/// Read-only means read-only: the narrowest tier does not run a command or write a file, however
/// harmless a particular one would have been. The wider tiers differ in what they offer, not in
/// whether a destructive call still asks — that gate is inside the tool, for every tier.
pub fn scope_allows(scope: McpScope, access: ToolAccess) -> bool {
    match access {
        ToolAccess::Read => true,
        ToolAccess::Write => matches!(scope, McpScope::SafeWrite | McpScope::HighRisk),
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
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

/// A set of tools the registry does not own.
///
/// The built-in tools are compiled in and the registry holds them directly. External MCP servers are
/// different: their tools are learned at runtime, a server may be added or dropped while the
/// application runs, and the process that reaches them is not this crate. A source is the seam
/// where that dynamic half joins the one table an agent, the MCP bridge and a future command line
/// all read.
///
/// `call` answers `None` when the name is not one of this source's tools. That is not a failure: it
/// is how a name is looked for across several sources without any of them having to know about the
/// others.
#[async_trait]
pub trait ToolSource: Send + Sync {
    /// Every tool this source currently offers. Called again on each list, because a server's tools
    /// may have changed since the last handshake.
    fn descriptors(&self) -> Vec<ToolDescriptor>;

    async fn call(&self, name: &str, context: &ToolContext, arguments: Value) -> Option<Result<ToolResult, ToolError>>;
}

/// The tools this build offers, by name.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    /// Sources are added while the application runs and read from several calls at once, which is
    /// why this is a lock rather than a field the registry owns mutably. The lock is only held long
    /// enough to clone the list: a call does not hold it across an await, because a server that
    /// took a minute to answer must not block every other call.
    sources: RwLock<Vec<Arc<dyn ToolSource>>>,
    /// Names the settings page switched off. Kept here rather than in every caller so the one list
    /// an agent, the bridge and the assistant read is also the one list that honours the switch.
    disabled: RwLock<BTreeSet<String>>,
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

    /// Adds a set of runtime tools. Sources join the one table; unlike `register` there is no
    /// duplicate check here, because what a source offers is only known by asking it, and because a
    /// server may be added before it has answered a handshake. The merge below is where a clash is
    /// resolved.
    pub fn add_source(&self, source: Arc<dyn ToolSource>) {
        self.write_sources().push(source);
    }

    /// The sources this registry holds, cloned so a caller does not hold the lock across an await.
    pub fn sources(&self) -> Vec<Arc<dyn ToolSource>> {
        match self.sources.read() {
            Ok(sources) => sources.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn write_sources(&self) -> std::sync::RwLockWriteGuard<'_, Vec<Arc<dyn ToolSource>>> {
        match self.sources.write() {
            Ok(sources) => sources,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Replaces the set of tools the settings page has switched off.
    ///
    /// A live change: the next `descriptors()` and `call()` see it, which is what makes the bridge's
    /// `GET /tools` and the assistant agree with the page without a restart.
    pub fn set_disabled_tools(&self, disabled: BTreeSet<String>) {
        match self.disabled.write() {
            Ok(mut current) => *current = disabled,
            Err(poisoned) => *poisoned.into_inner() = disabled,
        }
    }

    pub fn disabled_tools(&self) -> BTreeSet<String> {
        match self.disabled.read() {
            Ok(disabled) => disabled.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn is_disabled(&self, name: &str) -> bool {
        match self.disabled.read() {
            Ok(disabled) => disabled.contains(name),
            Err(poisoned) => poisoned.into_inner().contains(name),
        }
    }

    /// Every descriptor, in name order — the built-in tools and the ones the sources offer now.
    ///
    /// A name two places claim is served once, by the one that was already there: the built-in
    /// tools first, then the sources in the order they were added. Shadowing is not a feature — it
    /// is a configuration mistake — so the clash is logged rather than silently resolved.
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        let mut merged: BTreeMap<String, ToolDescriptor> = self
            .tools
            .values()
            .map(|tool| {
                let descriptor = tool.descriptor();
                (descriptor.name.clone(), descriptor)
            })
            .collect();

        for source in self.sources() {
            for descriptor in source.descriptors() {
                if merged.contains_key(&descriptor.name) {
                    // A name two places claim is a configuration mistake, not a choice: say so
                    // rather than letting the list depend on which source registered first.
                    log::warn!(
                        "a tool is offered by more than one place; the first one stays: {}",
                        descriptor.name
                    );
                    continue;
                }
                merged.insert(descriptor.name.clone(), descriptor);
            }
        }

        merged.retain(|name, _| !self.is_disabled(name));
        merged.into_values().collect()
    }

    /// One descriptor, when the registry or one of its sources has it and it is switched on.
    pub fn descriptor(&self, name: &str) -> Option<ToolDescriptor> {
        if self.is_disabled(name) {
            return None;
        }
        if let Some(tool) = self.tools.get(name) {
            return Some(tool.descriptor());
        }
        self.sources().into_iter().find_map(|source| {
            source
                .descriptors()
                .into_iter()
                .find(|descriptor| descriptor.name == name)
        })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.descriptor(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.descriptors().len()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors().is_empty()
    }

    /// Runs one call, and records what happened however it happened.
    ///
    /// The built-in tools are looked at first, then each source in turn. A source that does not
    /// know the name answers `None`, which is how the name is looked for across several servers;
    /// a source that does know it runs the call, after the same scope gate the built-in tools go
    /// through.
    pub async fn call(&self, context: &ToolContext, call: ToolCall) -> ToolOutcome {
        let started = Instant::now();

        if self.is_disabled(&call.name) {
            return refuse_disabled(context, &call.name, started);
        }

        if let Some(tool) = self.tools.get(&call.name).cloned() {
            let descriptor = tool.descriptor();
            if !scope_allows(context.scope(), descriptor.access) {
                return refuse_scope(context, &descriptor.name, started);
            }
            return finish(
                tool.call(context, call.arguments).await,
                context,
                descriptor.name,
                started,
            );
        }

        for source in self.sources() {
            let Some(descriptor) = source
                .descriptors()
                .into_iter()
                .find(|descriptor| descriptor.name == call.name)
            else {
                continue;
            };
            if !scope_allows(context.scope(), descriptor.access) {
                return refuse_scope(context, &descriptor.name, started);
            }
            // A source that declared the name may still answer `None` — the server went away
            // between the list and the call — in which case the next source is still worth asking.
            if let Some(result) = source.call(&descriptor.name, context, call.arguments.clone()).await {
                return finish(result, context, descriptor.name, started);
            }
        }

        let error = ToolError::unknown_tool(&call.name);
        context.audit(
            AuditRecord::new(call.name, context.scope(), AuditStatus::Failed)
                .with_origin(context.origin())
                .with_duration_ms(millis(started.elapsed()))
                .with_error_code(error.code().to_owned()),
        );
        ToolOutcome::failed(error)
    }
}

/// Refuses a tool the active scope does not offer. Nothing was sent: the scope table is read before
/// the tool — built-in or external — is entered.
fn refuse_scope(context: &ToolContext, name: &str, started: Instant) -> ToolOutcome {
    let error = ToolError::denied(name);
    context.audit(
        AuditRecord::new(name, context.scope(), AuditStatus::Denied)
            .with_origin(context.origin())
            .with_duration_ms(millis(started.elapsed()))
            .with_error_code(error.code().to_owned()),
    );
    ToolOutcome::failed(error)
}

/// Refuses a tool the settings page switched off. It is a decision, like a scope refusal, and is
/// written down the same way rather than looking like a missing tool.
fn refuse_disabled(context: &ToolContext, name: &str, started: Instant) -> ToolOutcome {
    let error = ToolError::tool_disabled(name);
    context.audit(
        AuditRecord::new(name, context.scope(), AuditStatus::Denied)
            .with_origin(context.origin())
            .with_duration_ms(millis(started.elapsed()))
            .with_error_code(error.code().to_owned()),
    );
    ToolOutcome::failed(error)
}

/// Turns one tool's answer into the outcome the caller gets, and writes the one audit line.
fn finish(result: Result<ToolResult, ToolError>, context: &ToolContext, name: String, started: Instant) -> ToolOutcome {
    match result {
        Ok(result) => {
            context.audit(
                AuditRecord::new(name, context.scope(), AuditStatus::Succeeded)
                    .with_origin(context.origin())
                    .with_duration_ms(millis(started.elapsed()))
                    .with_audit(result.audit.clone()),
            );
            ToolOutcome::succeeded(result)
        }
        Err(error) => {
            // A policy refusal is a decision, not a malfunction: the audit's own word for it is
            // `denied`, which is also what the settings page shows. Every other failure keeps
            // `failed` so the two are never confused when somebody reads the trail later. A hidden
            // session is the same kind of decision as a scope refusal: the rules said no.
            let status = if matches!(
                error.code(),
                "POLICY-DENIED" | "MCP-SESSION-HIDDEN" | "MCP-TOOL-DISABLED"
            ) {
                AuditStatus::Denied
            } else {
                AuditStatus::Failed
            };
            context.audit(
                AuditRecord::new(name, context.scope(), status)
                    .with_origin(context.origin())
                    .with_duration_ms(millis(started.elapsed()))
                    .with_error_code(error.code().to_owned())
                    .with_audit(error.audit().cloned()),
            );
            ToolOutcome::failed(error)
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

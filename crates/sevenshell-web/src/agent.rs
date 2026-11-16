//! The tool table and the one approval channel both agent callers share.
//!
//! The built-in tools, the connection cache, the approval registry and the audit trail live here
//! once; the MCP listener and the assistant each get a context from this runtime instead of
//! building a second copy. The registry comes from `sevenshell-api`, so the tools, their schemas,
//! scope checks and audit lines are the same ones the desktop host uses.

use std::sync::Arc;
use std::time::SystemTime;

use sevenshell_api::{
    ApprovalDecision, Cancel, McpAccess, ToolCall, ToolContext, ToolDescriptor, ToolOutcome, ToolRegistry, ToolSource,
    local::{ConnectionCache, default_registry},
};
use sevenshell_core::Core;
use sevenshell_error::ShellError;
use sevenshell_types::{AgentOrigin, McpScope};
use sevenshell_vault::Vault;
use tokio::sync::broadcast;

use crate::approvals::{ApprovalRegistry, EventPresenter, WindowBroker};
use crate::mcp::audit::{self, AuditLog};

pub struct ToolRuntime {
    pub registry: Arc<ToolRegistry>,
    pub cache: Arc<ConnectionCache>,
    approvals: Arc<ApprovalRegistry>,
    presenter: Arc<EventPresenter>,
    audit: Arc<AuditLog>,
    core: Arc<Core>,
}

impl ToolRuntime {
    pub fn new(vault: Arc<Vault>, core: Arc<Core>, events: broadcast::Sender<String>) -> Result<Self, ShellError> {
        let _ = vault;
        let registry = Arc::new(default_registry());
        let cache = Arc::new(ConnectionCache::new(core.clone()));
        let settings = sevenshell_platform::load().ok();
        if let Some(settings) = settings.as_ref() {
            registry.set_disabled_tools(settings.mcp_tool_policy.disabled_tools.clone());
        }
        let path = audit::default_path()?;
        let retention = settings
            .as_ref()
            .map(|settings| settings.mcp_audit_retention_days)
            .unwrap_or_default();
        let audit = Arc::new(AuditLog::new(path, retention));
        audit.prune(SystemTime::now());

        Ok(Self {
            registry,
            cache,
            approvals: Arc::new(ApprovalRegistry::default()),
            presenter: Arc::new(EventPresenter::new(events)),
            audit,
            core,
        })
    }

    /// The settings that shape a context are read here, so a page that changed them takes effect on
    /// the next call without a restart.
    fn settings_scope() -> McpScope {
        sevenshell_platform::load()
            .map(|settings| settings.mcp_scope)
            .unwrap_or_default()
    }

    fn timeout_seconds() -> u64 {
        sevenshell_platform::load()
            .ok()
            .map(|settings| settings.mcp_approval_timeout_seconds)
            .filter(|seconds| *seconds > 0)
            .unwrap_or(sevenshell_types::MCP_APPROVAL_TIMEOUT_SECONDS_DEFAULT)
    }

    fn context_with(&self, origin: AgentOrigin, broker: WindowBroker) -> ToolContext {
        let scope = Self::settings_scope();
        let access = Arc::new(McpAccess::open(scope));
        ToolContext::new(self.core.clone(), scope)
            .with_approvals(Arc::new(broker))
            .with_audit(self.audit.clone())
            .with_cache(self.cache.clone())
            .with_approval_timeout_seconds(Self::timeout_seconds())
            .with_origin(origin)
            .with_access(access)
    }

    /// A context for one tool call, answered by a fixed call id — what the MCP listener uses.
    #[allow(dead_code)]
    pub fn context_for_call(&self, origin: AgentOrigin, call_id: String) -> ToolContext {
        let broker = WindowBroker::new(self.approvals.clone(), self.presenter.clone(), call_id);
        self.context_with(origin, broker)
    }

    /// A listener call: fixed call id, and the handle `POST /tools/cancel` reaches.
    pub fn context_for_listener(&self, origin: AgentOrigin, call_id: String, cancel: Cancel) -> ToolContext {
        let broker = WindowBroker::new(self.approvals.clone(), self.presenter.clone(), call_id);
        self.context_with(origin, broker).with_cancel(cancel)
    }

    /// A context for a run that may ask several questions — what the assistant uses.
    pub fn context_for_run(&self, origin: AgentOrigin, prefix: &str) -> ToolContext {
        let broker = WindowBroker::sequenced(
            self.approvals.clone(),
            self.presenter.clone(),
            origin,
            prefix.to_owned(),
        );
        self.context_with(origin, broker)
    }

    pub fn answer_approval(&self, call_id: &str, decision: ApprovalDecision) -> bool {
        self.approvals.answer(call_id, decision)
    }

    pub fn pending_approvals(&self) -> usize {
        self.approvals.len()
    }

    #[allow(dead_code)]
    pub fn clear_origin(&self, origin: AgentOrigin) -> usize {
        self.approvals.clear_origin(origin)
    }

    pub fn audit_tail(&self, limit: usize) -> Vec<sevenshell_api::AuditRecord> {
        audit::read_tail(self.audit.path(), limit)
    }

    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.registry.descriptors()
    }

    pub async fn call(&self, context: &ToolContext, call: ToolCall) -> ToolOutcome {
        self.registry.call(context, call).await
    }

    pub fn add_source(&self, source: Arc<dyn ToolSource>) {
        self.registry.add_source(source);
    }
}

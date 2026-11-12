//! What a tool is given when it is called: the core, a way to ask a person, and the trail.
//!
//! A tool has no ambient state of its own. Everything it may reach arrives in one context, which is
//! what makes a tool callable from a test without a window, and what makes "which calls were audited"
//! a property of the registry rather than of every tool remembering to.
//!
//! Approval and audit are trait objects on purpose. The desktop supplies a broker that draws a
//! question and a sink that appends JSONL; a test supplies a script and a vector. A tool cannot tell
//! the difference, which is the point.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sevenshell_core::Core;
use sevenshell_types::{AgentOrigin, McpScope, PresentedHostKey, RiskAssessment};
use tokio::sync::Notify;

use crate::local::ConnectionCache;
use crate::policy::McpAccess;
use crate::registry::scope_allows;
use crate::tool::{ToolAccess, ToolAudit, ToolError};

/// Something a tool can be called off through.
///
/// Per call rather than global: two calls may be running at once, and stopping one is not a request
/// to stop the other. Shared by cloning, because a caller that answers a question and the step
/// waiting for it have to be looking at the same flag.
#[derive(Debug, Clone, Default)]
pub struct Cancel {
    inner: Arc<CancelInner>,
}

#[derive(Debug, Default)]
struct CancelInner {
    cancelled: AtomicBool,
    /// One permit rather than a broadcast: a call has one cancellation, and `notify_one` keeps it
    /// until somebody listens — which is what makes cancelling before the step has registered a
    /// waiter mean something.
    signal: Notify,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
        self.inner.signal.notify_one();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Refuses if the call was already called off.
    pub fn check(&self) -> Result<(), ToolError> {
        if self.is_cancelled() {
            return Err(ToolError::cancelled());
        }
        Ok(())
    }

    /// Runs one step, unless the call is cancelled first.
    ///
    /// Dropping the future at the await is the cancellation: the socket being dialled is closed and
    /// whatever the step had already built goes with it.
    pub async fn guard<T>(
        &self,
        step: impl std::future::Future<Output = Result<T, ToolError>>,
    ) -> Result<T, ToolError> {
        self.check()?;
        tokio::select! {
            result = step => result,
            () = self.inner.signal.notified() => Err(ToolError::cancelled()),
        }
    }
}

/// What a person is being asked about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ApprovalReason {
    /// A command or an action the policy graded destructive.
    Risk { assessment: RiskAssessment },
    /// A command the matrix raised to a question at a particular layer.
    ///
    /// The verdict may be safe or caution: the classifier did not ask, a host, group or directory
    /// rule did. The source is the layer that produced the strictest action, so the person sees
    /// which rule is waiting rather than a verdict that looks contradictory.
    Policy { assessment: RiskAssessment, source: String },
    /// A host whose key has never been seen. The fingerprint is what the person compares against
    /// what the server's administrator gave them; the key itself is what gets trusted.
    HostKeyUnknown { key: PresentedHostKey },
    /// A tool from an external MCP server.
    ///
    /// Nothing about that tool can be judged here: this build did not write it, cannot see what it
    /// will do with the arguments, and has only the server's own word that it is read-only. An
    /// external call therefore stops for an answer every time, which is why the question names the
    /// server and the tool rather than a risk verdict it would have had to invent.
    External { server: String, tool: String },
}

/// One question about one call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    /// The tool that is asking, so the window can say what is waiting.
    pub tool: String,
    /// Who is asking: an MCP client or the in-application assistant.
    ///
    /// Absent reads as `mcp`, which is what every request written before the two callers existed
    /// meant. A tool does not normally set this — [`ToolContext::request_approval`] fills it from the
    /// context — but one that knows better may.
    #[serde(default)]
    pub origin: AgentOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    /// The command line or the remote path, when it helps a person decide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub reason: ApprovalReason,
    /// Zero means "the configured timeout", which the context fills in when it asks.
    #[serde(default)]
    pub timeout_seconds: u64,
}

impl ApprovalRequest {
    pub fn risk(tool: impl Into<String>, assessment: RiskAssessment) -> Self {
        Self {
            tool: tool.into(),
            origin: AgentOrigin::default(),
            session_id: None,
            session_name: None,
            subject: None,
            reason: ApprovalReason::Risk { assessment },
            timeout_seconds: 0,
        }
    }

    /// A question a matrix layer raised, with the layer named.
    pub fn policy(tool: impl Into<String>, assessment: RiskAssessment, source: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            origin: AgentOrigin::default(),
            session_id: None,
            session_name: None,
            subject: None,
            reason: ApprovalReason::Policy {
                assessment,
                source: source.into(),
            },
            timeout_seconds: 0,
        }
    }

    pub fn host_key(tool: impl Into<String>, key: PresentedHostKey) -> Self {
        Self {
            tool: tool.into(),
            origin: AgentOrigin::default(),
            session_id: None,
            session_name: None,
            subject: None,
            reason: ApprovalReason::HostKeyUnknown { key },
            timeout_seconds: 0,
        }
    }

    /// A question about a tool an external server provides.
    ///
    /// No session and no risk verdict: the call is not necessarily about a host this window has
    /// open, and this build has no rules for a tool it did not write. The server and the tool are
    /// the whole of what a person can decide on, so they are the whole of the reason.
    pub fn external(server: impl Into<String>, tool: impl Into<String>) -> Self {
        let tool = tool.into();
        Self {
            tool: tool.clone(),
            origin: AgentOrigin::default(),
            session_id: None,
            session_name: None,
            subject: None,
            reason: ApprovalReason::External {
                server: server.into(),
                tool,
            },
            timeout_seconds: 0,
        }
    }

    /// Names the caller the question belongs to, when the context is not the one that will answer.
    pub fn with_origin(mut self, origin: AgentOrigin) -> Self {
        self.origin = origin;
        self
    }

    pub fn for_session(mut self, session_id: impl Into<String>, session_name: Option<String>) -> Self {
        self.session_id = Some(session_id.into());
        self.session_name = session_name;
        self
    }

    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }
}

/// What came back from the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecision {
    Approved,
    Denied,
    /// Nobody answered inside the timeout, which is a refusal rather than a yes.
    TimedOut,
}

impl ApprovalDecision {
    pub const fn is_approved(self) -> bool {
        matches!(self, Self::Approved)
    }
}

/// One approval, as the audit trail keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRecord {
    pub reason: ApprovalReason,
    pub decision: ApprovalDecision,
    pub waited_ms: u64,
}

/// The window that asks, behind a trait so a tool never knows whether one is on screen.
#[async_trait]
pub trait ApprovalBroker: Send + Sync {
    async fn request(&self, request: ApprovalRequest) -> Result<ApprovalDecision, ToolError>;
}

/// The broker a process with no window has.
///
/// It refuses without asking. Failing closed is the only safe default: an agent that could run a
/// destructive command because nobody had wired the question up would be worse than an agent that
/// cannot run one at all.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyingBroker;

#[async_trait]
impl ApprovalBroker for DenyingBroker {
    async fn request(&self, _request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        Ok(ApprovalDecision::Denied)
    }
}

/// How a call ended, as the audit trail records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditStatus {
    Succeeded,
    /// The tool ran or was reached, and answered with a failure.
    Failed,
    /// Nothing ran: the scope does not offer this tool.
    Denied,
}

/// One line of the audit trail.
///
/// A record is written by the registry for every call, including the calls that were refused, so
/// the file answers "what was asked" and not only "what ran". No field carries a credential: the
/// subject is a command line or a path, and the tool inputs hold no secrets by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    /// Seconds since the Unix epoch, the same clock the rest of the application records.
    pub at: u64,
    pub tool: String,
    /// Who asked. Absent in a line written before the assistant existed, which was an MCP call.
    #[serde(default)]
    pub origin: AgentOrigin,
    /// The scope in force when the call was made, so a widening or a narrowing is visible in the
    /// trail rather than inferred from the calls that did or did not happen.
    pub scope: McpScope,
    pub status: AuditStatus,
    /// The failure's code, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<RiskAssessment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalRecord>,
}

impl AuditRecord {
    pub fn new(tool: impl Into<String>, scope: McpScope, status: AuditStatus) -> Self {
        Self {
            at: now_seconds(),
            tool: tool.into(),
            origin: AgentOrigin::default(),
            scope,
            status,
            code: None,
            duration_ms: 0,
            session_id: None,
            subject: None,
            risk: None,
            approval: None,
        }
    }

    pub fn with_origin(mut self, origin: AgentOrigin) -> Self {
        self.origin = origin;
        self
    }

    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    pub fn with_error_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Merges in what the tool reported. Nothing the registry already knows is replaced.
    pub fn with_audit(mut self, audit: Option<ToolAudit>) -> Self {
        if let Some(audit) = audit {
            self.session_id = audit.session_id;
            self.subject = audit.subject;
            self.risk = audit.risk;
            self.approval = audit.approval;
        }
        self
    }
}

/// Where audit lines go.
///
/// Infallible on purpose. By the time a line is written the action has already happened, and an
/// audit that could fail the call would be a failure that cannot undo anything — the sink logs its
/// own trouble to stderr instead.
pub trait AuditSink: Send + Sync {
    fn record(&self, record: AuditRecord);
}

/// The sink a process without a trail has, which is also what tests override.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoAuditSink;

impl AuditSink for NoAuditSink {
    fn record(&self, _record: AuditRecord) {}
}

/// Everything one call is given.
pub struct ToolContext {
    core: Arc<Core>,
    approvals: Arc<dyn ApprovalBroker>,
    audit: Arc<dyn AuditSink>,
    scope: McpScope,
    access: Arc<McpAccess>,
    /// Who the calls made through this context belong to. Written into the audit line and into any
    /// approval question a tool asks.
    origin: AgentOrigin,
    approval_timeout_seconds: u64,
    cancel: Cancel,
    /// The handles this installation reuses between calls. Shared rather than created per call: a
    /// file channel opened by one call is what the next call reads through, and a context that
    /// owned its own would authenticate once per question an agent asks.
    cache: Arc<ConnectionCache>,
}

impl ToolContext {
    /// A context that refuses every approval and records nothing. A host replaces both.
    pub fn new(core: Arc<Core>, scope: McpScope) -> Self {
        // A cache of its own, so a context built without a host still works — a test, or a caller
        // that makes one call. The host replaces it with the shared one below.
        let cache = Arc::new(ConnectionCache::new(Arc::clone(&core)));
        Self {
            core,
            approvals: Arc::new(DenyingBroker),
            audit: Arc::new(NoAuditSink),
            scope,
            access: Arc::new(McpAccess::open(scope)),
            origin: AgentOrigin::default(),
            approval_timeout_seconds: sevenshell_types::MCP_APPROVAL_TIMEOUT_SECONDS_DEFAULT,
            cancel: Cancel::new(),
            cache,
        }
    }

    pub fn with_approvals(mut self, approvals: Arc<dyn ApprovalBroker>) -> Self {
        self.approvals = approvals;
        self
    }

    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = audit;
        self
    }

    /// Gives every call the same set of open handles.
    ///
    /// The cache is what makes a conversation one authenticated connection rather than one per
    /// question. It is handed in rather than built here because the host is the only side that
    /// knows when the vault locks — which is when [`ConnectionCache::close_all`] has to run.
    pub fn with_cache(mut self, cache: Arc<ConnectionCache>) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_approval_timeout_seconds(mut self, seconds: u64) -> Self {
        self.approval_timeout_seconds = seconds;
        self
    }

    /// Says who the calls through this context belong to.
    ///
    /// Set by whoever builds the context — the MCP server or the assistant — rather than by a tool:
    /// a tool answers the same question for either caller, and one that named a caller itself would
    /// be one that could get it wrong.
    pub fn with_origin(mut self, origin: AgentOrigin) -> Self {
        self.origin = origin;
        self
    }

    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    /// The handles this installation is holding open, shared with every call that follows.
    pub fn cache(&self) -> &Arc<ConnectionCache> {
        &self.cache
    }

    pub fn scope(&self) -> McpScope {
        self.scope
    }

    /// Replaces the global scope with a resolved session policy.
    ///
    /// `scope` becomes the widest the policy allows, so the registry's early refusal cannot reject a
    /// call a session override would have permitted; the tool's guard is what narrows it again.
    pub fn with_access(mut self, access: Arc<McpAccess>) -> Self {
        self.scope = access.max_scope();
        self.access = access;
        self
    }

    pub fn access(&self) -> &Arc<McpAccess> {
        &self.access
    }

    /// The scope for one session, refusing a session the policy hides.
    pub fn session_scope(&self, session_id: &str) -> Result<McpScope, ToolError> {
        if !self.access.visible(session_id) {
            return Err(ToolError::session_hidden(session_id));
        }
        Ok(self.access.scope_for(session_id))
    }

    /// Refuses a hidden session or a tool the session's own scope does not offer.
    pub fn guard_session(&self, session_id: &str, access: ToolAccess) -> Result<McpScope, ToolError> {
        let scope = self.session_scope(session_id)?;
        if !scope_allows(scope, access) {
            return Err(ToolError::denied("this tool under the session's agent access scope"));
        }
        Ok(scope)
    }

    /// Who the calls through this context belong to.
    pub fn origin(&self) -> AgentOrigin {
        self.origin
    }

    pub fn cancel(&self) -> &Cancel {
        &self.cancel
    }

    pub fn audit(&self, record: AuditRecord) {
        self.audit.record(record);
    }

    /// Asks a person, and answers with what the window said and how long it took to say it.
    ///
    /// The timeout is the context's when the request leaves it at zero, so a tool does not have to
    /// know the configured number and cannot quietly choose its own.
    pub async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalRecord, ToolError> {
        let mut request = request;
        // A request that did not name a caller inherits the context's: the tool that asks does not
        // have to know whether it was reached over MCP or from the assistant panel.
        if request.origin == AgentOrigin::default() {
            request.origin = self.origin;
        }
        if request.timeout_seconds == 0 {
            request.timeout_seconds = self.approval_timeout_seconds;
        }

        let started = Instant::now();
        let decision = self.approvals.request(request.clone()).await?;
        Ok(ApprovalRecord {
            reason: request.reason,
            decision,
            waited_ms: millis(started.elapsed()),
        })
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

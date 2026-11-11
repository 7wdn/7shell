//! The question a destructive call asks a person.
//!
//! The bridge has no window, and the rules say an agent never decides on its own that a destructive
//! command was agreed to. So a call that needs approval parks here: one sender per call id, one event
//! to the interface, and a deadline. Three ways out, and all of them are refusals except the one a
//! person clicks:
//!
//! * the window answers `approved` or `denied`;
//! * nobody answers before the configured timeout;
//! * there is no window to ask (or the event could not be delivered), which is a refusal because a
//!   question nobody saw cannot have been agreed to.
//!
//! Nothing is remembered. The first wave deliberately asks again for the next destructive call: a
//! "yes, always" is a policy decision the settings page should make, not a side effect of one click.
//!
//! The presenter is a trait so the request path can be tested without a webview. The real one emits
//! `mcp:approval`; a test records it and answers by hand.
//!
//! One registry serves both callers — an MCP client and the assistant panel — because the window
//! draws one dialog and answers by call id. A question remembers who asked it, so one caller
//! stopping does not take the other's question with it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sevenshell_api::{ApprovalBroker, ApprovalDecision, ApprovalReason, ApprovalRequest, ToolError};
use sevenshell_types::{AgentOrigin, MCP_APPROVAL_TIMEOUT_SECONDS_DEFAULT};
use tauri::{AppHandle, Emitter as _, Manager as _};
use tokio::sync::oneshot;

/// The event the interface listens on.
pub const APPROVAL_EVENT: &str = "mcp:approval";

/// One question, as the window receives it.
///
/// `expiresAt` is epoch milliseconds rather than a duration: the window may be drawn a second after
/// the request was made, and a countdown has to start at the moment the deadline was set, not at the
/// moment a button appeared. It is the same number the server enforces.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApprovalRequest {
    pub call_id: String,
    pub tool: String,
    /// Who is asking. The MCP broker always says `mcp`; the assistant's own broker will say `ai`, and
    /// the window shows which one is waiting. Absent reads as `mcp` for a listener written before the
    /// field existed.
    #[serde(default)]
    pub origin: AgentOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub reason: ApprovalReason,
    pub expires_at: u64,
}

/// Where a pending question can be answered from.
///
/// Shared by every call of a run, and — since the assistant asks the same window through the same
/// channel — shared between the MCP listener and the assistant panel. One registry rather than two
/// is what keeps a single dialog on screen: the interface answers by call id through
/// `mcp_approval_answer`, and it does not need to know which caller asked.
#[derive(Debug, Default)]
pub struct ApprovalRegistry {
    pending: Mutex<HashMap<String, Pending>>,
    /// One question at a time: the window draws one dialog, and a second event while the first is
    /// open would replace it and leave the first waiting for an answer nobody can give.
    turn: tokio::sync::Mutex<()>,
}

/// One waiting question: who asked it, and where its answer goes.
#[derive(Debug)]
struct Pending {
    /// Remembered so one caller can be turned off without dropping the other's question. Stopping
    /// the MCP listener is not a reason for the assistant panel to lose a question it is waiting on.
    origin: AgentOrigin,
    sender: oneshot::Sender<ApprovalDecision>,
}

impl ApprovalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn register(&self, call_id: &str, origin: AgentOrigin, sender: oneshot::Sender<ApprovalDecision>) {
        // A repeated id would mean two requests shared one id; replacing the old sender drops it,
        // which resolves that request as a refusal rather than leaving it to time out in the dark.
        self.lock().insert(call_id.to_owned(), Pending { origin, sender });
    }

    fn withdraw(&self, call_id: &str) {
        self.lock().remove(call_id);
    }

    /// Answers one question. False when no question with that id is waiting — an id from a call that
    /// already timed out, or one a stale window sent twice.
    pub fn answer(&self, call_id: &str, decision: ApprovalDecision) -> bool {
        let sender = self.lock().remove(call_id).map(|pending| pending.sender);
        match sender {
            Some(sender) => sender.send(decision).is_ok(),
            None => false,
        }
    }

    /// Drops the questions one caller is waiting on, and leaves the other's alone.
    ///
    /// Each dropped sender resolves its request as a refusal, which is the same answer a window
    /// nobody can reach gives. Answers with how many were dropped, so the two callers can say so.
    pub fn clear_origin(&self, origin: AgentOrigin) -> usize {
        let mut guard = self.lock();
        let before = guard.len();
        guard.retain(|_, pending| pending.origin != origin);
        before - guard.len()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Waits for the previous question to leave the screen.
    async fn turn(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.turn.lock().await
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        match self.pending.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// What draws the question. `false` means nobody could be asked.
pub trait ApprovalPresenter: Send + Sync {
    fn present(&self, request: &McpApprovalRequest) -> bool;
}

/// The real presenter: one event to the window.
pub struct TauriPresenter {
    app: AppHandle,
}

impl TauriPresenter {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ApprovalPresenter for TauriPresenter {
    fn present(&self, request: &McpApprovalRequest) -> bool {
        // A window that does not exist cannot be answered, and emitting into nothing would wait out
        // the whole timeout before saying so. Hiding the window in the tray is not the same thing:
        // it still exists, the event still arrives, and the person can bring it back — the deadline
        // is what covers one they never do.
        if self.app.get_webview_window("main").is_none() {
            return false;
        }
        self.app.emit(APPROVAL_EVENT, request).is_ok()
    }
}

/// The broker one HTTP request — or one assistant run — is given.
pub struct WindowBroker {
    registry: Arc<ApprovalRegistry>,
    presenter: Arc<dyn ApprovalPresenter>,
    /// Who this broker asks for. Written into every question it presents, because the window shows
    /// which of the two callers is waiting: an MCP client or the assistant panel in this window.
    origin: AgentOrigin,
    ids: CallIds,
}

/// Where the id a question is answered by comes from.
enum CallIds {
    /// One call, one id: what the MCP listener does, because one HTTP request is one tool call.
    Fixed(String),
    /// A run of several questions, each of which needs an id of its own: an assistant run may call
    /// more than one tool, and two questions sharing an id would replace each other's sender.
    Sequence { prefix: String, next: AtomicU64 },
}

impl WindowBroker {
    pub fn new(registry: Arc<ApprovalRegistry>, presenter: Arc<dyn ApprovalPresenter>, call_id: String) -> Self {
        Self {
            registry,
            presenter,
            origin: AgentOrigin::Mcp,
            ids: CallIds::Fixed(call_id),
        }
    }

    /// A broker for a run that may ask more than once: every question gets `{prefix}-{n}`.
    ///
    /// The prefix is the run id, which makes the ids in one window's answer command unique per run
    /// without the window having to know what a run is.
    pub fn sequenced(
        registry: Arc<ApprovalRegistry>,
        presenter: Arc<dyn ApprovalPresenter>,
        origin: AgentOrigin,
        prefix: impl Into<String>,
    ) -> Self {
        Self {
            registry,
            presenter,
            origin,
            ids: CallIds::Sequence {
                prefix: prefix.into(),
                next: AtomicU64::new(1),
            },
        }
    }

    fn next_call_id(&self) -> String {
        match &self.ids {
            CallIds::Fixed(id) => id.clone(),
            CallIds::Sequence { prefix, next } => {
                format!("{prefix}-{}", next.fetch_add(1, Ordering::Relaxed))
            }
        }
    }
}

#[async_trait]
impl ApprovalBroker for WindowBroker {
    async fn request(&self, request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        // Held from the moment this question is about to be shown until it is answered, refused or
        // timed out. The deadline therefore starts when the dialog can actually be drawn, not while
        // this call is still queued behind someone else's question.
        let _turn = self.registry.turn().await;

        let call_id = self.next_call_id();
        let timeout_seconds = if request.timeout_seconds == 0 {
            MCP_APPROVAL_TIMEOUT_SECONDS_DEFAULT
        } else {
            request.timeout_seconds
        };
        let expires_at = now_millis().saturating_add(timeout_seconds.saturating_mul(1_000));

        let question = McpApprovalRequest {
            call_id: call_id.clone(),
            tool: request.tool.clone(),
            origin: self.origin,
            session_id: request.session_id.clone(),
            session_name: request.session_name.clone(),
            subject: request.subject.clone(),
            reason: request.reason.clone(),
            expires_at,
        };

        let (sender, receiver) = oneshot::channel();
        self.registry.register(&call_id, self.origin, sender);

        // Registered before it is presented, withdrawn if presentation fails: an answer racing the
        // event is answered, and an event nobody received leaves nothing behind.
        if !self.presenter.present(&question) {
            self.registry.withdraw(&call_id);
            return Ok(ApprovalDecision::Denied);
        }

        match tokio::time::timeout(Duration::from_secs(timeout_seconds), receiver).await {
            Ok(Ok(decision)) => Ok(decision),
            // The sender was dropped: the endpoint stopped underneath the call, which is a refusal
            // for the same reason no window is.
            Ok(Err(_)) => Ok(ApprovalDecision::Denied),
            Err(_) => {
                self.registry.withdraw(&call_id);
                Ok(ApprovalDecision::TimedOut)
            }
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_types::RiskAssessment;

    #[derive(Default)]
    struct RecordingPresenter {
        answers: Mutex<Vec<McpApprovalRequest>>,
        present: bool,
    }

    impl RecordingPresenter {
        fn answering() -> Self {
            Self {
                answers: Mutex::new(Vec::new()),
                present: true,
            }
        }

        fn absent() -> Self {
            Self {
                answers: Mutex::new(Vec::new()),
                present: false,
            }
        }

        fn seen(&self) -> Vec<McpApprovalRequest> {
            self.answers.lock().expect("not poisoned").clone()
        }
    }

    impl ApprovalPresenter for RecordingPresenter {
        fn present(&self, request: &McpApprovalRequest) -> bool {
            self.answers.lock().expect("not poisoned").push(request.clone());
            self.present
        }
    }

    fn a_request(timeout_seconds: u64) -> ApprovalRequest {
        let mut request = ApprovalRequest::risk("sshell_exec_run", RiskAssessment::safe())
            .for_session("session-1", Some("the database".to_owned()))
            .with_subject("rm -rf /var/lib/example");
        request.timeout_seconds = timeout_seconds;
        request
    }

    async fn wait_until_pending(registry: &ApprovalRegistry) {
        for _ in 0..100 {
            if !registry.is_empty() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the question was never registered");
    }

    #[tokio::test]
    async fn an_approved_question_comes_back_as_approved() {
        let registry = Arc::new(ApprovalRegistry::default());
        let presenter = Arc::new(RecordingPresenter::answering());
        let broker = WindowBroker::new(registry.clone(), presenter.clone(), "call-1".to_owned());

        let task = tokio::spawn(async move { broker.request(a_request(120)).await });
        wait_until_pending(&registry).await;

        let seen = presenter.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].call_id, "call-1");
        assert_eq!(seen[0].tool, "sshell_exec_run");
        assert_eq!(
            seen[0].origin,
            AgentOrigin::Mcp,
            "this broker answers for an MCP client"
        );
        assert_eq!(seen[0].subject.as_deref(), Some("rm -rf /var/lib/example"));
        assert!(seen[0].expires_at > 0);

        assert!(registry.answer("call-1", ApprovalDecision::Approved));
        assert_eq!(
            task.await.expect("joined").expect("answered"),
            ApprovalDecision::Approved
        );
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn a_denied_question_comes_back_as_denied() {
        let registry = Arc::new(ApprovalRegistry::default());
        let presenter = Arc::new(RecordingPresenter::answering());
        let broker = WindowBroker::new(registry.clone(), presenter, "call-2".to_owned());

        let task = tokio::spawn(async move { broker.request(a_request(120)).await });
        wait_until_pending(&registry).await;

        assert!(registry.answer("call-2", ApprovalDecision::Denied));
        assert_eq!(task.await.expect("joined").expect("answered"), ApprovalDecision::Denied);
    }

    #[tokio::test(start_paused = true)]
    async fn a_question_nobody_answers_times_out_and_does_not_stay_registered() {
        let registry = Arc::new(ApprovalRegistry::default());
        let presenter = Arc::new(RecordingPresenter::answering());
        let broker = WindowBroker::new(registry.clone(), presenter, "call-3".to_owned());

        let task = tokio::spawn(async move { broker.request(a_request(1)).await });
        wait_until_pending(&registry).await;
        // Let the task reach its timer: the question is registered just before the deadline starts.
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(2)).await;

        assert_eq!(
            task.await.expect("joined").expect("answered"),
            ApprovalDecision::TimedOut
        );
        assert!(registry.is_empty(), "a timed-out question must not linger");
    }

    #[tokio::test]
    async fn a_question_nobody_could_be_asked_is_denied_at_once() {
        let registry = Arc::new(ApprovalRegistry::default());
        let presenter = Arc::new(RecordingPresenter::absent());
        let broker = WindowBroker::new(registry.clone(), presenter, "call-4".to_owned());

        let decision = broker.request(a_request(120)).await.expect("an answer");

        assert_eq!(decision, ApprovalDecision::Denied);
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn stopping_the_endpoint_resolves_a_waiting_question_as_denied() {
        let registry = Arc::new(ApprovalRegistry::default());
        let presenter = Arc::new(RecordingPresenter::answering());
        let broker = WindowBroker::new(registry.clone(), presenter, "call-5".to_owned());

        let task = tokio::spawn(async move { broker.request(a_request(120)).await });
        wait_until_pending(&registry).await;

        registry.clear_origin(AgentOrigin::Mcp);

        assert_eq!(task.await.expect("joined").expect("answered"), ApprovalDecision::Denied);
    }

    #[tokio::test]
    async fn only_one_question_is_on_screen_at_a_time() {
        let registry = Arc::new(ApprovalRegistry::default());
        let presenter = Arc::new(RecordingPresenter::answering());
        let first = WindowBroker::new(registry.clone(), presenter.clone(), "call-a".to_owned());
        let second = WindowBroker::new(registry.clone(), presenter.clone(), "call-b".to_owned());

        let first_task = tokio::spawn(async move { first.request(a_request(120)).await });
        wait_until_pending(&registry).await;

        let second_task = tokio::spawn(async move { second.request(a_request(120)).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            presenter.seen().len(),
            1,
            "the second question must wait for the first instead of replacing it"
        );

        assert!(registry.answer("call-a", ApprovalDecision::Approved));
        for _ in 0..100 {
            if presenter.seen().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(presenter.seen().len(), 2, "the second question follows the first");
        assert!(registry.answer("call-b", ApprovalDecision::Denied));

        assert_eq!(
            first_task.await.expect("joined").expect("answered"),
            ApprovalDecision::Approved
        );
        assert_eq!(
            second_task.await.expect("joined").expect("answered"),
            ApprovalDecision::Denied
        );
    }

    #[test]
    fn answering_an_id_that_is_not_waiting_is_a_no_op() {
        let registry = ApprovalRegistry::default();

        assert!(!registry.answer("never-asked", ApprovalDecision::Approved));
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn one_run_asking_twice_gets_an_id_for_each_question() {
        // An assistant run may call more than one tool. Two questions under one id would replace
        // each other's sender in the registry, and the first would be answered as a refusal nobody
        // clicked.
        let registry = Arc::new(ApprovalRegistry::default());
        let presenter = Arc::new(RecordingPresenter::answering());
        let broker = Arc::new(WindowBroker::sequenced(
            registry.clone(),
            presenter.clone(),
            AgentOrigin::Ai,
            "run-7",
        ));

        let first = tokio::spawn({
            let broker = broker.clone();
            async move { broker.request(a_request(120)).await }
        });
        wait_until_pending(&registry).await;

        let second = tokio::spawn({
            let broker = broker.clone();
            async move { broker.request(a_request(120)).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(presenter.seen().len(), 1, "the second question waits for the first");

        assert!(registry.answer("run-7-1", ApprovalDecision::Approved));
        for _ in 0..100 {
            if presenter.seen().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let seen = presenter.seen();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].call_id, "run-7-2", "the second question has an id of its own");
        assert_eq!(seen[1].origin, AgentOrigin::Ai);

        assert!(registry.answer("run-7-2", ApprovalDecision::Denied));
        assert_eq!(
            first.await.expect("joined").expect("answered"),
            ApprovalDecision::Approved
        );
        assert_eq!(
            second.await.expect("joined").expect("answered"),
            ApprovalDecision::Denied
        );
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn turning_the_agent_interface_off_does_not_drop_the_assistant_question() {
        // The two callers share one registry, so stopping the MCP listener must be an answer about
        // MCP questions only. The assistant panel is still on screen, and its dialog would vanish
        // under a person who was reading it.
        let registry = Arc::new(ApprovalRegistry::default());
        let presenter = Arc::new(RecordingPresenter::answering());

        let interface = WindowBroker::new(registry.clone(), presenter.clone(), "call-mcp".to_owned());
        let task = tokio::spawn(async move { interface.request(a_request(120)).await });
        wait_until_pending(&registry).await;
        assert_eq!(registry.clear_origin(AgentOrigin::Mcp), 1);
        assert_eq!(
            task.await.expect("joined").expect("answered"),
            ApprovalDecision::Denied,
            "a question whose asker is gone is a refusal"
        );

        let assistant = WindowBroker::sequenced(registry.clone(), presenter.clone(), AgentOrigin::Ai, "run-1");
        let task = tokio::spawn(async move { assistant.request(a_request(120)).await });
        wait_until_pending(&registry).await;

        assert_eq!(
            registry.clear_origin(AgentOrigin::Mcp),
            0,
            "the interface stopping has no opinion about the assistant's question"
        );
        assert!(registry.answer("run-1-1", ApprovalDecision::Approved));
        assert_eq!(
            task.await.expect("joined").expect("answered"),
            ApprovalDecision::Approved
        );
        assert_eq!(
            presenter.seen().last().map(|question| question.origin),
            Some(AgentOrigin::Ai),
            "the window shows which caller is asking"
        );
    }

    #[test]
    fn the_question_names_the_caller_on_the_wire() {
        let question = McpApprovalRequest {
            call_id: "call-1".to_owned(),
            tool: "sshell_exec_run".to_owned(),
            origin: AgentOrigin::Mcp,
            session_id: None,
            session_name: None,
            subject: None,
            reason: ApprovalReason::Risk {
                assessment: RiskAssessment::safe(),
            },
            expires_at: 1,
        };

        let json = serde_json::to_value(&question).expect("a question is JSON");

        assert_eq!(
            json["origin"],
            serde_json::json!("mcp"),
            "the window reads this to say who is asking"
        );
    }
}

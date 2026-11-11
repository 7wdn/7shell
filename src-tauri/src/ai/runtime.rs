//! The assistant the shell holds: the conversation on screen, one answer at a time, and the
//! provider that answers it.
//!
//! Four things are decided here that nothing below this layer can decide:
//!
//! * **Which provider answers**, from the settings file, and **with which key**, from the vault. A
//!   provider the person pinned wins; otherwise the active one; otherwise the only one configured.
//!   A key is read, handed to the adapter and dropped — no message, event, log line or error carries
//!   it.
//! * **Which conversation a question joins.** The conversation is a [`StoredSession`] held in memory
//!   and written to the vault at the end of each turn: a named id loads it, no id continues the one
//!   already open, and a first question opens a new one. The vault does not parse it and this layer
//!   does not care where it is stored — only that a failed write must never cost a message.
//! * **What that conversation is, in this process.** The messages the window mirrors, plus the
//!   assistant's own half reconstructed from the events it streamed. A question is appended before
//!   the run starts, so a second question asked while the first is answering sees it — and cancels
//!   the first, because one conversation has one answer.
//! * **What a run may reach.** The [`ToolContext`] is built with the shared registry, cache,
//!   approval registry and audit sink, and with [`AgentOrigin::Ai`], so a tool that asks a question
//!   or writes a line says who asked.
//!
//! The loop itself is [`sevenshell_agent`]'s. It is behind [`TurnRunner`] rather than called
//! directly: the host owes tests a way to drive the run table, the event bridge and the conversation
//! without a real provider. The provider is built through [`ProviderFactory`] for the same reason —
//! compaction sends a request of its own, and a test must be able to answer it without a socket.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt as _;
use sevenshell_agent::{Agent, AgentError, AgentEvent, EventSink, RunOutcome, RunStopReason};
use sevenshell_ai::{AiProviderConfig, AiProviderView, ChatMessage, Provider, ProviderError, build_provider};
use sevenshell_api::{AuditSink, Cancel, ToolContext, ToolRegistry};
use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_types::{AgentOrigin, AiContextSummary, AiSessionDetail, AiSessionSummary, AiToolOutcome, AppSettings};
use sevenshell_vault::{ServicePassword, Vault};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::ai::context::{self, AiChatContext};
use crate::ai::events::{AiChatEvent, AiEventPresenter};
use crate::ai::sessions::{self, StoredSession};
use crate::ai::{catalog_error, compact, message_key_for};
use crate::mcp::approval::{ApprovalPresenter, WindowBroker};
use crate::state::SharedAgent;

/// How long a provider gets to start answering a connection test.
///
/// A test is a person clicking a button and watching it; a provider that has not begun a reply in
/// half a minute is the same answer as one that is not reachable at all.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// What a send was asked for, as the command received it.
#[derive(Debug, Clone)]
pub struct AiChatSend {
    pub text: String,
    /// The provider the question pinned, or nothing for the active one.
    pub provider_id: Option<String>,
    pub context: Option<AiChatContext>,
    /// The conversation the question continues, or nothing for the one already open.
    pub session_id: Option<String>,
}

/// What a send answers once the run exists.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatStarted {
    pub run_id: String,
    /// The conversation the run belongs to. Always present: a send always joins or opens one.
    pub session_id: String,
}

/// What a connection test answered.
///
/// Not a rejection when the provider refused: "that key is wrong" is the *answer* to the question
/// that was asked, and drawing it as a failure of the test would make the page report the wrong
/// thing. The envelope's two halves travel with it so the page says the sentence every other failure
/// says.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderTestResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_key: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub message_params: BTreeMap<String, String>,
}

impl AiProviderTestResult {
    fn ok() -> Self {
        Self {
            ok: true,
            code: None,
            message_key: None,
            message_params: BTreeMap::new(),
        }
    }

    fn from_code(code: ErrorCode) -> Self {
        Self {
            ok: false,
            code: Some(code.as_str().to_owned()),
            message_key: Some(code.message_key().to_owned()),
            message_params: BTreeMap::new(),
        }
    }

    fn from_error(error: &ShellError) -> Self {
        Self {
            ok: false,
            code: Some(error.code().as_str().to_owned()),
            message_key: Some(error.message_key().to_owned()),
            message_params: error.message_params().clone(),
        }
    }

    fn from_provider_error(error: &ProviderError) -> Self {
        Self {
            ok: false,
            code: Some(error.code().to_owned()),
            message_key: Some(message_key_for(error.code()).to_owned()),
            message_params: BTreeMap::new(),
        }
    }
}

/// One run, as the table keeps it.
struct Run {
    cancel: Cancel,
    /// The conversation the run is answering. A run whose conversation has been left, deleted or
    /// superseded is dropped rather than written back into whichever one is open now.
    session_id: String,
}

/// The assistant as the shell keeps it.
pub struct AiRuntime {
    vault: Arc<Vault>,
    shared: Arc<SharedAgent>,
    /// The conversation the window mirrors. Guarded rather than locked for the life of a run: a run
    /// reads a snapshot, and writes its own half back when it ends.
    session: Mutex<Option<StoredSession>>,
    /// The run answering right now, by the id its events carry. At most one is producing; a second
    /// may be finishing after a cancel, which is why the entry outlives the cancellation.
    runs: Mutex<BTreeMap<String, Run>>,
    runner: Arc<dyn TurnRunner>,
    /// How a provider is built, so a test can answer the compaction request without a socket.
    providers: Arc<dyn ProviderFactory>,
}

impl AiRuntime {
    pub fn new(vault: Arc<Vault>, shared: Arc<SharedAgent>) -> Self {
        Self::with_parts(vault, shared, Arc::new(AgentRunner), Arc::new(BuiltProvider))
    }

    /// The runtime with both seams replaced: the loop and the provider factory.
    ///
    /// Tests drive the run table and the event bridge with a runner that answers in microseconds,
    /// and compaction with a provider that never opens a socket; a shipped build always uses
    /// [`AgentRunner`] and [`BuiltProvider`].
    pub(crate) fn with_parts(
        vault: Arc<Vault>,
        shared: Arc<SharedAgent>,
        runner: Arc<dyn TurnRunner>,
        providers: Arc<dyn ProviderFactory>,
    ) -> Self {
        Self {
            vault,
            shared,
            session: Mutex::new(None),
            runs: Mutex::new(BTreeMap::new()),
            runner,
            providers,
        }
    }

    /// Every configured provider, with whether a key is held.
    ///
    /// `hasKey` is `None` while the vault is locked: "could not tell right now" is not "there is
    /// none", and a page that drew the second for the first would be telling somebody their key had
    /// been lost.
    pub fn providers(&self, settings: &AppSettings) -> Vec<AiProviderView> {
        settings
            .ai_providers
            .iter()
            .map(|config| AiProviderView::new(config.clone(), self.key_state(&config.id)))
            .collect()
    }

    /// Whether a key is held for one provider, or `None` when the vault cannot say.
    pub fn key_state(&self, provider_id: &str) -> Option<bool> {
        match self.vault.ai_token(provider_id) {
            Ok(token) => Some(token.is_some_and(|token| !token.is_empty())),
            // Locked, missing or unreadable: the answer is the same one — nobody can say right now.
            Err(_) => None,
        }
    }

    /// Keeps a key for one provider, or forgets the one that was kept.
    ///
    /// An empty or blank value is a deletion, which is the vault's own rule; trimming is this
    /// layer's, because a key pasted from a password manager arrives with the space around it and a
    /// key with a trailing newline fails authentication for no reason a person can see.
    pub fn set_key(&self, provider_id: &str, key: Option<String>) -> Result<(), ShellError> {
        let trimmed = key
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self.vault.set_ai_token(provider_id, trimmed)
    }

    /// Removes a provider: its key first, then its configuration.
    ///
    /// The order is what makes a half-done removal recoverable. A locked vault refuses the key
    /// deletion before anything else has changed, so the list cannot lose an entry whose key nobody
    /// could remove. The settings write that follows a successful deletion is the caller's; if it
    /// fails, what is left is a provider with no key — a state the page draws and the person can
    /// finish removing by clicking again.
    pub fn remove_provider(&self, settings: &mut AppSettings, provider_id: &str) -> Result<(), ShellError> {
        if !settings.ai_providers.iter().any(|config| config.id == provider_id) {
            return Err(catalog_error(ErrorCode::AiNoProvider));
        }

        self.vault.set_ai_token(provider_id, None)?;
        settings.ai_providers.retain(|config| config.id != provider_id);
        if settings.ai_default_provider_id.as_deref() == Some(provider_id) {
            settings.ai_default_provider_id = None;
        }
        Ok(())
    }

    /// Sends the smallest possible request to one provider and says whether it answered.
    ///
    /// Nothing is written to the conversation: a test is a question about the configuration, not a
    /// turn of somebody's conversation.
    pub async fn test_provider(&self, settings: &AppSettings, provider_id: &str) -> AiProviderTestResult {
        let Some(config) = settings.ai_providers.iter().find(|config| config.id == provider_id) else {
            return AiProviderTestResult::from_code(ErrorCode::AiNoProvider);
        };

        let key = match self.key_for(config) {
            Ok(key) => key,
            Err(error) => return AiProviderTestResult::from_error(&error),
        };
        let provider = match provider_for(config, key.as_ref()) {
            Ok(provider) => provider,
            Err(error) => return AiProviderTestResult::from_error(&error),
        };

        probe(provider, &config.model).await
    }

    /// Starts one run for one question.
    ///
    /// Answers as soon as the run exists, with the id every event of it carries and the conversation
    /// it joins. A new question supersedes the run that is answering: one conversation has one
    /// answer, and a person typing a second question has moved on from the first.
    pub fn start(
        self: &Arc<Self>,
        settings: &AppSettings,
        events: Arc<dyn AiEventPresenter>,
        approvals: Arc<dyn ApprovalPresenter>,
        request: AiChatSend,
    ) -> Result<AiChatStarted, ShellError> {
        if request.text.trim().is_empty() {
            return Err(ShellError::app_internal(
                "the assistant was asked to send an empty message",
            ));
        }

        let config = resolve_provider(settings, request.provider_id.as_deref())?;
        let key = self.key_for(&config)?;
        let provider = self.providers.build(&config, key.as_ref())?;
        let prompt = context::system_prompt(&config, request.context.as_ref());

        // The retention this installation chose, applied before anything can be recorded. The MCP
        // listener does the same when it starts; whichever of the two runs first is the one that
        // configures the shared trail.
        self.shared.audit().configure(settings.mcp_audit_retention_days);

        // The question is in the conversation before the run exists, so the history the run is
        // handed ends with it — and so a superseded run's question is not lost with its answer.
        let session = self.attach_question(
            request.session_id.as_deref(),
            &request.text,
            sessions::now_seconds(),
            events.as_ref(),
        )?;

        let run_id = Agent::new_run_id();
        let cancel = Cancel::new();
        self.supersede();

        let broker = WindowBroker::sequenced(
            Arc::clone(self.shared.approvals()),
            approvals,
            AgentOrigin::Ai,
            run_id.clone(),
        );
        let tool_context = ToolContext::new(Arc::clone(self.shared.connections().core()), settings.mcp_scope)
            .with_cache(Arc::clone(self.shared.connections()))
            .with_approvals(Arc::new(broker))
            .with_audit(Arc::clone(self.shared.audit()) as Arc<dyn AuditSink>)
            .with_origin(AgentOrigin::Ai)
            .with_approval_timeout_seconds(settings.mcp_approval_timeout_seconds)
            .with_cancel(cancel.clone());

        self.lock_runs().insert(
            run_id.clone(),
            Run {
                cancel: cancel.clone(),
                session_id: session.id.clone(),
            },
        );

        let planned = PlannedRun {
            run_id: run_id.clone(),
            session_id: session.id.clone(),
            provider,
            model: config.model.clone(),
            system: prompt,
            max_tokens: config.max_output_tokens,
            context: tool_context,
            messages: session.messages.clone(),
            summary: session.context_summary.clone(),
            cancel,
        };
        let runtime = Arc::clone(self);
        // Spawned rather than awaited: a run may take minutes, and the command's answer is the id
        // the window listens for. Compaction is part of this task for the same reason — it may cost
        // one provider round trip, and a send that waited on it would leave the window looking
        // stalled before it could even draw the question. `tauri::async_runtime` rather than
        // `tokio::spawn` because this is also called from tests, which have no Tauri runtime — only
        // a tokio one.
        tauri::async_runtime::spawn(async move { runtime.drive(planned, events).await });

        Ok(AiChatStarted {
            run_id,
            session_id: session.id,
        })
    }

    /// Calls off the named run, or whichever one is producing when none is named.
    ///
    /// The entry stays until the run reports back, so the part of the answer that was already
    /// streamed is still added to the conversation: "stop" is not "forget what you saw".
    pub fn cancel(&self, run_id: Option<&str>) -> bool {
        let cancel = {
            let runs = self.lock_runs();
            match run_id {
                Some(id) => runs.get(id).map(|run| run.cancel.clone()),
                None => runs
                    .values()
                    .find(|run| !run.cancel.is_cancelled())
                    .map(|run| run.cancel.clone()),
            }
        };

        let Some(cancel) = cancel else {
            return false;
        };
        cancel.cancel();
        // A question this run is waiting on has nobody left to answer it for. The MCP client's
        // questions are the listener's, and they stay.
        self.shared.approvals().clear_origin(AgentOrigin::Ai);
        true
    }

    /// The vault is locking: no run may keep using a key that is about to stop existing.
    ///
    /// The conversation stays — it is the person's own typing — but the answer that was in flight
    /// does not, and neither does a question waiting on a tool call that can no longer succeed.
    /// Nothing is written here: the vault is closing, so a save could only fail, and the conversation
    /// in memory is what the next successful save writes.
    pub fn on_vault_locked(&self) {
        self.stop_everything(false);
    }

    /// The window is going away: nothing here has a reader or an answer left.
    ///
    /// The conversation is written down on the way out rather than dropped: closing a window is not
    /// the same act as deleting a chat, and the row in the list is what makes the next window able to
    /// show it again.
    pub fn shutdown(&self) {
        self.stop_everything(true);
    }

    /// The id of the run that is producing, when one is.
    ///
    /// One answer at a time is the invariant, so this is the run a `ai_chat_cancel` with no id names.
    /// Nothing else asks, which is why the only callers are the cancellation below and the tests.
    #[cfg(test)]
    fn active_run(&self) -> Option<String> {
        self.lock_runs()
            .iter()
            .find(|(_, run)| !run.cancel.is_cancelled())
            .map(|(id, _)| id.clone())
    }

    /// Restarts a conversation.
    ///
    /// The one on screen is written down first, so "clear" is never "lose": without an id a fresh
    /// conversation opens and the old row stays in the list under its own name, and with an id that
    /// conversation is emptied and stays where it is — the row a person knows, starting over.
    ///
    /// A locked vault refuses the emptying with its own error (there is nowhere to write), which is
    /// the answer the page draws; the conversation in memory is left as it was.
    pub fn reset(&self, session_id: Option<&str>, events: &dyn AiEventPresenter) -> Result<(), ShellError> {
        // Whatever was answering belonged to the conversation being left.
        self.supersede();
        let now = sessions::now_seconds();

        let mut current = self.lock_session();
        if let Some(session) = current.as_ref() {
            self.persist(session, Some(events));
        }

        match session_id {
            Some(id) => {
                let Some(mut session) = sessions::load(&self.vault, id)? else {
                    // The row is gone (deleted from another window, or aged out by retention). The
                    // window asked for an empty chat, so that is what it gets: keeping the previous
                    // one open would put the next question into a conversation they just cleared.
                    *current = Some(StoredSession::new(now));
                    return Ok(());
                };
                sessions::clear(&self.vault, &mut session, now)?;
                events.emit(&AiChatEvent::session_saved(&session.id, session.updated_at));
                *current = Some(session);
            }
            None => {
                // Not written yet: a row with nothing in it is one nobody can open to anything. The
                // first question names it and the end of that turn writes it.
                *current = Some(StoredSession::new(now));
            }
        }
        Ok(())
    }

    /// The stored conversations, newest first, with retention applied.
    pub fn list_sessions(&self, settings: &AppSettings) -> Result<Vec<AiSessionSummary>, ShellError> {
        sessions::list(&self.vault, settings, sessions::now_seconds())
    }

    /// One conversation in full, as the transcript draws it.
    ///
    /// The conversation in memory is the fresher copy when it is the one asked for: it may hold a
    /// turn the vault has not been told about yet. Everything else is read from the vault, which is
    /// also where a locked vault says so.
    pub fn session(&self, id: &str) -> Result<AiSessionDetail, ShellError> {
        {
            let current = self.lock_session();
            if let Some(session) = current.as_ref().filter(|session| session.id == id) {
                return Ok(session.detail());
            }
        }

        match sessions::load(&self.vault, id)? {
            Some(session) => Ok(session.detail()),
            None => Err(ShellError::session_not_found(id)),
        }
    }

    /// Renames a stored conversation.
    ///
    /// The row's `updatedAt` does not move: a rename changes a label, and a list that reordered
    /// itself because of one would be sorting by the wrong thing.
    pub fn rename_session(
        &self,
        id: &str,
        name: &str,
        events: &dyn AiEventPresenter,
    ) -> Result<AiSessionSummary, ShellError> {
        let renamed = sessions::rename(&self.vault, id, name)?.ok_or_else(|| ShellError::session_not_found(id))?;

        {
            let mut current = self.lock_session();
            if let Some(session) = current.as_mut().filter(|session| session.id == id) {
                session.name = renamed.name.clone();
            }
        }

        events.emit(&AiChatEvent::session_saved(&renamed.id, renamed.updated_at));
        Ok(renamed.summary())
    }

    /// Forgets a conversation, and anything still answering it.
    pub fn delete_session(&self, id: &str) -> Result<(), ShellError> {
        sessions::delete(&self.vault, id)?;

        // A deleted conversation has nothing left to answer into: the run is called off and its
        // entry taken, so its half-written answer cannot be written back under an id that is gone.
        let removed = {
            let mut runs = self.lock_runs();
            let doomed = runs
                .iter()
                .filter(|(_, run)| run.session_id == id)
                .map(|(run_id, _)| run_id.clone())
                .collect::<Vec<_>>();
            doomed
                .into_iter()
                .filter_map(|run_id| runs.remove(&run_id))
                .collect::<Vec<_>>()
        };
        if !removed.is_empty() {
            for run in removed {
                run.cancel.cancel();
            }
            self.shared.approvals().clear_origin(AgentOrigin::Ai);
        }

        let mut current = self.lock_session();
        if current.as_ref().is_some_and(|session| session.id == id) {
            *current = None;
        }
        Ok(())
    }

    /// The key the vault holds for this provider, if one is kept.
    ///
    /// A locked vault is an error rather than "no key": the page says "cannot tell" and a send says
    /// the vault is locked. Whether a missing key is fatal is the provider's own rule — a local
    /// server accepts none — and Lane A's factory is where that rule lives.
    fn key_for(&self, config: &AiProviderConfig) -> Result<Option<ServicePassword>, ShellError> {
        self.vault.ai_token(&config.id)
    }

    /// Ends the run that is producing, because a new question arrived.
    ///
    /// Its entry goes too: the answer it was streaming belongs to a conversation the person has
    /// already moved past, and appending half of it under the new question would misrepresent both.
    fn supersede(&self) {
        let previous = std::mem::take(&mut *self.lock_runs());
        if previous.is_empty() {
            return;
        }
        for (_, run) in previous {
            run.cancel.cancel();
        }
        self.shared.approvals().clear_origin(AgentOrigin::Ai);
    }

    fn stop_everything(&self, write_down: bool) {
        let runs = std::mem::take(&mut *self.lock_runs());
        for (_, run) in runs {
            run.cancel.cancel();
        }
        self.shared.approvals().clear_origin(AgentOrigin::Ai);
        if write_down {
            // The window is going away, so nothing here has a reader left — but the conversation is
            // this installation's, and the vault (while it is open) is where it belongs.
            let mut current = self.lock_session();
            if let Some(session) = current.as_ref() {
                self.persist(session, None);
            }
            *current = None;
        }
    }

    /// The conversation a question joins, with the question in it.
    ///
    /// A named conversation that is not the one open is loaded first; a name the vault no longer
    /// holds opens a conversation under that id rather than refusing the question, because the person
    /// asked something and the row they were looking at is the least interesting part of that. With
    /// no name the conversation already open continues — a second turn of the same chat — and only a
    /// chat with nothing open starts a new one.
    fn attach_question(
        &self,
        requested: Option<&str>,
        question: &str,
        now: u64,
        events: &dyn AiEventPresenter,
    ) -> Result<StoredSession, ShellError> {
        let mut current = self.lock_session();

        let switching = match (current.as_ref(), requested) {
            (Some(session), Some(id)) => session.id != id,
            (None, Some(_)) => true,
            _ => false,
        };

        if switching {
            // The conversation being left is written down before another is opened: switching away
            // from a chat is not deleting it, and a failed write costs nothing the next save cannot
            // repair.
            if let Some(session) = current.as_ref() {
                self.persist(session, Some(events));
            }
            let next = match requested {
                Some(id) => sessions::load(&self.vault, id)?.unwrap_or_else(|| StoredSession::with_id(id, now)),
                None => StoredSession::new(now),
            };
            *current = Some(next);
        } else if current.is_none() {
            *current = Some(StoredSession::new(now));
        }

        let session = current.as_mut().expect("a conversation was just put in memory");
        session.messages.push(ChatMessage::user(question));
        if session.name.trim().is_empty() {
            session.name = sessions::name_from_question(question);
        }
        session.updated_at = now;
        Ok(session.clone())
    }

    /// Writes the conversation down, saying so on the window's channel when it worked.
    ///
    /// A failure is logged and nothing else: a locked vault or a full disk must not end a turn,
    /// and the document in memory is still whole. Every save writes the entire conversation, so the
    /// next successful one repairs what this one could not.
    fn persist(&self, session: &StoredSession, events: Option<&dyn AiEventPresenter>) -> bool {
        match sessions::save(&self.vault, session) {
            Ok(()) => {
                if let Some(events) = events {
                    events.emit(&AiChatEvent::session_saved(&session.id, session.updated_at));
                }
                true
            }
            Err(error) => {
                log::warn!("an assistant conversation could not be written down: {error}");
                false
            }
        }
    }

    /// Replaces the conversation in memory with what compaction produced.
    ///
    /// Guarded by the run still being in the table: a superseded run's compaction describes a
    /// conversation the person has already moved past, and applying it would truncate whichever one
    /// is open now under the wrong name. Both guards are taken together — the run table first, as
    /// [`Self::finish`] does — because a question arriving between the check and the write would
    /// otherwise be dropped by the trim it raced.
    fn apply_compaction(&self, run_id: &str, session_id: &str, compacted: &compact::Compacted) -> bool {
        let runs = self.lock_runs();
        if runs.get(run_id).is_none() {
            return false;
        }
        let mut current = self.lock_session();
        let Some(session) = current.as_mut() else {
            return false;
        };
        if session.id != session_id {
            return false;
        }
        session.messages = compacted.messages.clone();
        session.context_summary = compacted.summary.clone();
        session.updated_at = sessions::now_seconds();
        true
    }

    /// Runs the loop, forwards what it says, and files what it answered.
    async fn drive(self: Arc<Self>, planned: PlannedRun, events: Arc<dyn AiEventPresenter>) {
        // Compaction is the first thing the run does, and it happens here rather than in `start` so
        // that the send already answered: a summary may cost one provider round trip of up to a
        // minute, and a window waiting on the send would have nothing to draw it against.
        let compacted = compact::compact(
            &planned.provider,
            &planned.model,
            planned.summary.as_ref(),
            &planned.messages,
            &planned.cancel,
            sessions::now_seconds(),
        )
        .await;

        if compacted.compacted {
            let applied = self.apply_compaction(&planned.run_id, &planned.session_id, &compacted);
            if applied {
                events.emit(&AiChatEvent::compacted(
                    &planned.session_id,
                    compacted.summarized_messages,
                    compacted.kept_messages,
                    compacted.degraded,
                ));
            } else {
                log::debug!(
                    "a compaction for run {} was dropped with the run it belonged to",
                    planned.run_id
                );
            }
        }

        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let pump = tokio::spawn(pump_events(receiver, Arc::clone(&events), planned.run_id.clone()));

        let outcome = self
            .runner
            .run(RunRequest {
                provider: planned.provider,
                registry: Arc::clone(self.shared.registry()),
                model: planned.model,
                system: planned.system,
                max_tokens: planned.max_tokens,
                context: planned.context,
                history: compacted.history,
                events: sender.clone(),
                cancel: planned.cancel,
            })
            .await;
        // The run's own sender is gone with the future; dropping the last one is what tells the pump
        // that no further event is coming.
        drop(sender);

        let pumped = pump.await.unwrap_or_default();

        let (expected_end, history) = match outcome {
            Ok(outcome) => (
                Some((outcome.stop_reason, outcome.turns, outcome.usage)),
                // The loop hands the conversation back, oldest first, including the question this
                // run was asked. That is the whole reason to keep a conversation here.
                Some(outcome.history),
            ),
            Err(error) => {
                // The only loop-level failure is a closed event sink: the window this answer was
                // for is gone. There is nobody to tell and nothing worth filing.
                log::warn!("the assistant run {} stopped before answering: {error}", planned.run_id);
                (Some((RunStopReason::Error, 0, None)), None)
            }
        };

        // The loop announces its own ending; a run that ended without one would leave the window
        // drawing an answer forever.
        if !pumped.terminated
            && let Some((stop_reason, turns, usage)) = expected_end
        {
            events.emit(&AiChatEvent::done(&planned.run_id, stop_reason, turns, usage));
        }

        match history {
            Some(history) => self.finish(&planned.run_id, history, pumped.outcomes, events.as_ref()),
            None => self.discard(&planned.run_id),
        }
    }

    /// Files the conversation a finished run came back with, unless it belongs to nobody any more.
    ///
    /// A cancelled run arrives here too, and that is deliberate: the loop hands back the conversation
    /// *without* the turn it was interrupted in (Lang B's rule), so what is written is everything that
    /// had already been said — the question, and every exchange that finished before it.
    fn finish(
        &self,
        run_id: &str,
        history: Vec<ChatMessage>,
        outcomes: BTreeMap<String, AiToolOutcome>,
        events: &dyn AiEventPresenter,
    ) {
        // The run entry outlives the write. That is what makes `active_run` a truthful "the turn is
        // over" signal: a caller that sees it gone (the window, a test) is also looking at a
        // conversation whose save has been attempted. Removing it first left a window in which the
        // run was gone and the file still old.
        let mut runs = self.lock_runs();
        let Some(session_id) = runs.get(run_id).map(|run| run.session_id.clone()) else {
            // Superseded, reset or deleted: the entry was taken while the run was still going, and
            // its answer belongs to a conversation the person has moved past.
            return;
        };

        let mut current = self.lock_session();
        let Some(session) = current.as_mut() else {
            runs.remove(run_id);
            return;
        };
        if session.id != session_id {
            runs.remove(run_id);
            return;
        }

        // Replaced rather than appended: the loop returns the whole conversation, so appending it to
        // what the host already had would double every message. The summary turn the request carried
        // is not part of the transcript — it is the compact form of messages that are gone.
        session.messages = strip_summary_head(history, session.context_summary.as_ref());
        session.outcomes.extend(outcomes);
        session.updated_at = sessions::now_seconds();
        if let Err(error) = session.trim_to_limits() {
            log::warn!("an assistant conversation outgrew what may be stored: {error}");
        }

        self.persist(session, Some(events));
        runs.remove(run_id);
    }

    /// Forgets a run that ended without a conversation.
    fn discard(&self, run_id: &str) {
        self.lock_runs().remove(run_id);
    }

    fn lock_session(&self) -> std::sync::MutexGuard<'_, Option<StoredSession>> {
        match self.session.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn lock_runs(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Run>> {
        match self.runs.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// What the conversation holds, for the tests that assert a run was filed or dropped.
    #[cfg(test)]
    fn conversation(&self) -> Vec<ChatMessage> {
        self.lock_session()
            .as_ref()
            .map(|session| session.messages.clone())
            .unwrap_or_default()
    }
}

/// The messages a finished run gave back, without the summary turn the request was prefixed with.
///
/// The summary is compared rather than assumed to be first, because "the first message is synthetic"
/// is only true when a summary exists *and* the loop returned the request history it was given. A
/// run that answered without echoing it must not lose its first real message to a guess.
fn strip_summary_head(history: Vec<ChatMessage>, summary: Option<&AiContextSummary>) -> Vec<ChatMessage> {
    let Some(summary) = summary else {
        return history;
    };
    if history.first() == Some(&compact::summary_message(summary)) {
        history.into_iter().skip(1).collect()
    } else {
        history
    }
}

/// One run, as the loop is handed it.
pub struct RunRequest {
    pub provider: Arc<dyn Provider>,
    pub registry: Arc<ToolRegistry>,
    /// The model the run's provider serves, from its configuration.
    pub model: String,
    /// The prompt this host built for the run.
    pub system: String,
    pub max_tokens: u32,
    pub context: ToolContext,
    pub history: Vec<ChatMessage>,
    pub events: EventSink,
    pub cancel: Cancel,
}

/// What a loop does with one run.
///
/// A trait rather than a direct call so the host's half of a run — the table, the events, the
/// conversation — can be tested without a provider and without a network round trip.
pub trait TurnRunner: Send + Sync {
    fn run(&self, request: RunRequest) -> RunFuture;
}

pub type RunFuture = Pin<Box<dyn Future<Output = Result<RunOutcome, AgentError>> + Send>>;

/// The real runner: Lane B's loop, one run at a time.
///
/// The four things the loop is not asked for — which model, which prompt, how much output, and
/// which tool list — are the host's, and they are set here because they come from the settings the
/// host read.
pub struct AgentRunner;

impl TurnRunner for AgentRunner {
    fn run(&self, request: RunRequest) -> RunFuture {
        Box::pin(async move {
            let agent = Agent::new(request.provider, request.registry)
                .with_model(request.model)
                .with_system(request.system)
                .with_max_tokens(request.max_tokens);
            agent
                .run(&request.context, request.history, request.events, request.cancel)
                .await
        })
    }
}

/// One run's inputs, gathered so the task has one thing to move.
struct PlannedRun {
    run_id: String,
    session_id: String,
    provider: Arc<dyn Provider>,
    model: String,
    system: String,
    max_tokens: u32,
    context: ToolContext,
    /// The conversation as the session held it when the question arrived, before compaction.
    messages: Vec<ChatMessage>,
    /// What replaced the older half last time, if anything did.
    summary: Option<AiContextSummary>,
    cancel: Cancel,
}

/// What the event pump came to.
#[derive(Default)]
struct Pumped {
    /// Whether the loop told the window the run ended. A run that did not is ended by the host.
    terminated: bool,
    /// One row per tool call the run made, which is what a restored card is drawn from.
    outcomes: BTreeMap<String, AiToolOutcome>,
}

/// Forwards a run's events to the window, under the run id the window was told.
///
/// The loop mints its own run id — it is the one inside its `messageId`s — and the window was given
/// the host's id before the first event arrived. Every event is therefore re-filed under the host's
/// id on the way out; nothing else about it changes. A `Agent::with_run_id` on the loop would remove
/// this step, and is the one interface request this lane makes of Lane B.
///
/// Tool outcomes are collected here because this is the one place that sees every event of the run:
/// the alternative is for each caller of the loop to remember to collect them.
async fn pump_events(
    mut receiver: UnboundedReceiver<AgentEvent>,
    events: Arc<dyn AiEventPresenter>,
    run_id: String,
) -> Pumped {
    let mut pumped = Pumped::default();

    while let Some(event) = receiver.recv().await {
        if let AgentEvent::ToolResult {
            call_id,
            ok,
            code,
            truncated,
            duration_ms,
            ..
        } = &event
        {
            pumped.outcomes.insert(
                call_id.clone(),
                AiToolOutcome {
                    call_id: call_id.clone(),
                    ok: *ok,
                    code: code.clone(),
                    truncated: *truncated,
                    duration_ms: *duration_ms,
                },
            );
        }

        let out = AiChatEvent::from_agent(&event).under(&run_id);
        if matches!(out, AiChatEvent::Done { .. }) {
            pumped.terminated = true;
        }
        // A window that is gone is why the run is being cancelled; the event has nowhere to go and
        // saying so on every delta would be noise.
        events.emit(&out);
    }

    pumped
}

/// Which provider a question goes to.
///
/// Pinned, then active, then the only one configured. That last step is what makes a fresh install
/// one provider away from working: a person who added a single provider has already said which one
/// they mean.
fn resolve_provider(settings: &AppSettings, requested: Option<&str>) -> Result<AiProviderConfig, ShellError> {
    if let Some(id) = requested {
        return settings
            .ai_providers
            .iter()
            .find(|config| config.id == id)
            .cloned()
            .ok_or_else(|| catalog_error(ErrorCode::AiNoProvider));
    }

    if let Some(active) = settings.ai_default_provider_id.as_deref()
        && let Some(config) = settings.ai_providers.iter().find(|config| config.id == active)
    {
        return Ok(config.clone());
    }

    match settings.ai_providers.as_slice() {
        [only] => Ok(only.clone()),
        _ => Err(catalog_error(ErrorCode::AiNoProvider)),
    }
}

/// The adapter for one provider, with the key it was given.
///
/// Lane A's factory is the only place that dispatches on the kind, and it is also where the address
/// rule lives: a plain `http` address anywhere but this machine is refused before a key is attached.
/// Its two refusals are turned into the catalog's own codes, because they are the two things a
/// person can act on — "no key stored" and "this provider cannot be reached".
fn provider_for(config: &AiProviderConfig, key: Option<&ServicePassword>) -> Result<Arc<dyn Provider>, ShellError> {
    build_provider(config, key.map(ServicePassword::expose)).map_err(|error| match error {
        ProviderError::NoKey => catalog_error(ErrorCode::AiNoKey),
        other => {
            log::warn!("the assistant could not build the provider `{}`: {other}", config.id);
            catalog_error(ErrorCode::AiProviderUnavailable)
        }
    })
}

/// How the runtime gets a provider.
///
/// A seam rather than a direct call for one reason: compaction sends a summary request of its own
/// through the same provider, and a test that ran against a real adapter would need a socket. A
/// shipped build always uses [`BuiltProvider`].
pub trait ProviderFactory: Send + Sync {
    fn build(&self, config: &AiProviderConfig, key: Option<&ServicePassword>) -> Result<Arc<dyn Provider>, ShellError>;
}

/// The real factory: Lane A's adapter for the configured kind.
pub struct BuiltProvider;

impl ProviderFactory for BuiltProvider {
    fn build(&self, config: &AiProviderConfig, key: Option<&ServicePassword>) -> Result<Arc<dyn Provider>, ShellError> {
        provider_for(config, key)
    }
}

/// One minimal request, and whatever the provider answered to it.
async fn probe(provider: Arc<dyn Provider>, model: &str) -> AiProviderTestResult {
    let request = context::provider_request(model, vec![ChatMessage::user("ping")], 1);

    match provider.stream(request).await {
        Ok(mut stream) => match tokio::time::timeout(PROBE_TIMEOUT, stream.next()).await {
            // The first event — or the end of an empty stream — is an answer: the key was accepted
            // and the model was asked. Reading further would cost tokens for a question nobody asked.
            Ok(Some(Ok(_))) | Ok(None) => AiProviderTestResult::ok(),
            Ok(Some(Err(error))) => AiProviderTestResult::from_provider_error(&error),
            Err(_) => AiProviderTestResult::from_provider_error(&ProviderError::Timeout),
        },
        Err(error) => AiProviderTestResult::from_provider_error(&error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use serde_json::Value;
    use sevenshell_ai::{AiProviderKind, ProviderRequest, ProviderStream, StopReason, StreamEvent};
    use sevenshell_api::{ToolCall, ToolOutcome};
    use sevenshell_core::Core;
    use sevenshell_types::McpScope;

    use crate::state::SharedAudit;

    #[derive(Default)]
    struct RecordingEvents {
        seen: StdMutex<Vec<(String, Value)>>,
    }

    impl RecordingEvents {
        fn seen(&self) -> Vec<(String, Value)> {
            self.seen.lock().expect("not poisoned").clone()
        }
    }

    impl AiEventPresenter for RecordingEvents {
        fn emit(&self, event: &AiChatEvent) -> bool {
            self.seen
                .lock()
                .expect("not poisoned")
                .push((event.name().to_owned(), event.payload()));
            true
        }
    }

    /// A window that is always there. Nothing in these tests asks a question, so it never draws one.
    struct NoQuestions;

    impl ApprovalPresenter for NoQuestions {
        fn present(&self, _request: &crate::mcp::approval::McpApprovalRequest) -> bool {
            true
        }
    }

    /// Answers one turn and reports what happened, without a provider.
    ///
    /// Its events carry a run id of its own, like the loop's: the host is the side that told the
    /// window which id to expect, so the pump has to re-file them.
    struct EchoRunner;

    impl TurnRunner for EchoRunner {
        fn run(&self, request: RunRequest) -> RunFuture {
            Box::pin(async move {
                // Unbounded: `send` is immediate, and a dropped receiver means the window is gone.
                let _ = request.events.send(AgentEvent::TextDelta {
                    run_id: "the-loop's-own-id".to_owned(),
                    message_id: "message-1".to_owned(),
                    block_index: 0,
                    text: "hello".to_owned(),
                });
                let _ = request.events.send(AgentEvent::Done {
                    run_id: "the-loop's-own-id".to_owned(),
                    stop_reason: RunStopReason::EndTurn,
                    turns: 1,
                    usage: None,
                });
                let mut history = request.history;
                history.push(ChatMessage::assistant(vec![sevenshell_ai::ContentBlock::Text {
                    text: "hello".to_owned(),
                }]));
                Ok(RunOutcome {
                    run_id: "the-loop's-own-id".to_owned(),
                    stop_reason: RunStopReason::EndTurn,
                    turns: 1,
                    usage: None,
                    history,
                })
            })
        }
    }

    /// Produces nothing until it is called off, then reports that it was.
    struct BlockingRunner;

    impl TurnRunner for BlockingRunner {
        fn run(&self, request: RunRequest) -> RunFuture {
            Box::pin(async move {
                while !request.cancel.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Ok(RunOutcome {
                    run_id: "the-blocking-run".to_owned(),
                    stop_reason: RunStopReason::Cancelled,
                    turns: 1,
                    usage: None,
                    history: request.history,
                })
            })
        }
    }

    /// Answers with one tool call, its result, and a closing sentence.
    struct ToolRunner;

    impl TurnRunner for ToolRunner {
        fn run(&self, request: RunRequest) -> RunFuture {
            Box::pin(async move {
                let run_id = "the-loop's-own-id".to_owned();
                let _ = request.events.send(AgentEvent::ToolCall {
                    run_id: run_id.clone(),
                    call_id: "call-1".to_owned(),
                    tool: "sshell_sessions_list".to_owned(),
                    input: serde_json::json!({}),
                    session_id: None,
                });
                let _ = request.events.send(AgentEvent::ToolResult {
                    run_id: run_id.clone(),
                    call_id: "call-1".to_owned(),
                    tool: "sshell_sessions_list".to_owned(),
                    ok: true,
                    code: None,
                    preview: "one session".to_owned(),
                    truncated: false,
                    duration_ms: 42,
                });
                let _ = request.events.send(AgentEvent::Done {
                    run_id: run_id.clone(),
                    stop_reason: RunStopReason::EndTurn,
                    turns: 1,
                    usage: None,
                });

                let mut history = request.history;
                history.push(ChatMessage::assistant(vec![sevenshell_ai::ContentBlock::ToolUse {
                    id: "call-1".to_owned(),
                    name: "sshell_sessions_list".to_owned(),
                    input: serde_json::json!({}),
                }]));
                history.push(ChatMessage::tool_result("call-1", "one session", false));
                history.push(ChatMessage::assistant(vec![sevenshell_ai::ContentBlock::Text {
                    text: "one session is open".to_owned(),
                }]));
                Ok(RunOutcome {
                    run_id,
                    stop_reason: RunStopReason::EndTurn,
                    turns: 1,
                    usage: None,
                    history,
                })
            })
        }
    }

    /// Answers one turn after a pause, so a test can change something underneath it.
    struct SlowRunner;

    impl TurnRunner for SlowRunner {
        fn run(&self, request: RunRequest) -> RunFuture {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(40)).await;
                let mut history = request.history;
                history.push(ChatMessage::assistant(vec![sevenshell_ai::ContentBlock::Text {
                    text: "answered".to_owned(),
                }]));
                Ok(RunOutcome {
                    run_id: "the-slow-run".to_owned(),
                    stop_reason: RunStopReason::EndTurn,
                    turns: 1,
                    usage: None,
                    history,
                })
            })
        }
    }

    /// Remembers the history it was asked with, then answers.
    struct RecordingRunner {
        seen: StdMutex<Vec<Vec<ChatMessage>>>,
    }

    impl RecordingRunner {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: StdMutex::new(Vec::new()),
            })
        }

        fn histories(&self) -> Vec<Vec<ChatMessage>> {
            self.seen.lock().expect("not poisoned").clone()
        }
    }

    impl TurnRunner for RecordingRunner {
        fn run(&self, request: RunRequest) -> RunFuture {
            self.seen.lock().expect("not poisoned").push(request.history.clone());
            Box::pin(async move {
                let _ = request.events.send(AgentEvent::Done {
                    run_id: "the-loop's-own-id".to_owned(),
                    stop_reason: RunStopReason::EndTurn,
                    turns: 1,
                    usage: None,
                });
                let mut history = request.history;
                history.push(ChatMessage::assistant(vec![sevenshell_ai::ContentBlock::Text {
                    text: "answered".to_owned(),
                }]));
                Ok(RunOutcome {
                    run_id: "the-loop's-own-id".to_owned(),
                    stop_reason: RunStopReason::EndTurn,
                    turns: 1,
                    usage: None,
                    history,
                })
            })
        }
    }

    /// Answers the summarising request with one paragraph, or refuses.
    struct SummaryProvider {
        answer: Result<String, ProviderError>,
    }

    #[async_trait]
    impl Provider for SummaryProvider {
        fn kind(&self) -> AiProviderKind {
            AiProviderKind::Local
        }

        async fn stream(&self, _request: ProviderRequest) -> Result<ProviderStream, ProviderError> {
            let answer = self.answer.clone()?;
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(StreamEvent::TextDelta { index: 0, text: answer }),
                Ok(StreamEvent::Stop {
                    reason: StopReason::EndTurn,
                }),
            ])))
        }
    }

    /// Hands the runtime a provider whatever the configuration says.
    struct FixedProvider(Arc<dyn Provider>);

    impl ProviderFactory for FixedProvider {
        fn build(
            &self,
            _config: &AiProviderConfig,
            _key: Option<&ServicePassword>,
        ) -> Result<Arc<dyn Provider>, ShellError> {
            Ok(Arc::clone(&self.0))
        }
    }

    struct Harness {
        _directory: tempfile::TempDir,
        vault: Arc<Vault>,
        shared: Arc<SharedAgent>,
        audit_path: std::path::PathBuf,
        runtime: Arc<AiRuntime>,
    }

    fn harness(runner: Arc<dyn TurnRunner>) -> Harness {
        harness_with(runner, Arc::new(BuiltProvider))
    }

    fn harness_with(runner: Arc<dyn TurnRunner>, providers: Arc<dyn ProviderFactory>) -> Harness {
        let directory = tempfile::tempdir().expect("a temp directory");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("master").expect("a vault");
        let (core, _events) = Core::new(Arc::clone(&vault));
        let audit_path = crate::mcp::audit::path_in(directory.path());
        let shared = Arc::new(SharedAgent::with_audit(
            core,
            Arc::new(SharedAudit::at(Some(audit_path.clone()))),
        ));
        let runtime = Arc::new(AiRuntime::with_parts(
            Arc::clone(&vault),
            Arc::clone(&shared),
            runner,
            providers,
        ));
        Harness {
            _directory: directory,
            vault,
            shared,
            audit_path,
            runtime,
        }
    }

    fn local_config() -> AiProviderConfig {
        AiProviderConfig::new(
            "local",
            "Local model",
            AiProviderKind::Local,
            "http://127.0.0.1:11434/v1",
            "llama3.1",
        )
    }

    fn hosted_config() -> AiProviderConfig {
        AiProviderConfig::new(
            "claude",
            "Claude",
            AiProviderKind::Anthropic,
            "https://api.anthropic.com",
            "claude-sonnet-4-5",
        )
    }

    fn settings_with(config: AiProviderConfig) -> AppSettings {
        AppSettings {
            ai_default_provider_id: Some(config.id.clone()),
            ai_providers: vec![config],
            ..AppSettings::default()
        }
    }

    fn send(text: &str) -> AiChatSend {
        AiChatSend {
            text: text.to_owned(),
            provider_id: None,
            context: None,
            session_id: None,
        }
    }

    fn send_in(text: &str, session_id: &str) -> AiChatSend {
        AiChatSend {
            session_id: Some(session_id.to_owned()),
            ..send(text)
        }
    }

    async fn wait_for(runtime: &Arc<AiRuntime>, done: impl Fn(&AiRuntime) -> bool, what: &str) {
        for _ in 0..400 {
            if done(runtime) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("{what} never happened");
    }

    #[test]
    fn a_locked_vault_cannot_say_whether_a_key_is_held() {
        let harness = harness(Arc::new(BlockingRunner));
        let settings = settings_with(hosted_config());

        harness
            .vault
            .set_ai_token("claude", Some("sk-claude".to_owned()))
            .expect("keep");
        harness.vault.lock();

        let views = harness.runtime.providers(&settings);
        assert_eq!(views.len(), 1);
        assert_eq!(
            views[0].has_key, None,
            "a locked vault must not draw as `no key`: somebody would think their key was lost"
        );
    }

    #[test]
    fn storing_and_forgetting_a_key_is_what_the_view_reports() {
        let harness = harness(Arc::new(BlockingRunner));
        let settings = settings_with(hosted_config());

        assert_eq!(harness.runtime.providers(&settings)[0].has_key, Some(false));

        harness
            .runtime
            .set_key("claude", Some("  sk-claude\n".to_owned()))
            .expect("keep");
        assert_eq!(
            harness.runtime.providers(&settings)[0].has_key,
            Some(true),
            "a pasted key arrives with whitespace and still counts as kept"
        );
        assert_eq!(
            harness.vault.ai_token("claude").expect("read").expect("kept").expose(),
            "sk-claude",
            "the key is stored trimmed"
        );

        harness
            .runtime
            .set_key("claude", Some("   ".to_owned()))
            .expect("forget");
        assert_eq!(harness.runtime.providers(&settings)[0].has_key, Some(false));
    }

    #[test]
    fn the_settings_file_has_no_field_a_key_could_travel_in() {
        let harness = harness(Arc::new(BlockingRunner));
        let mut settings = settings_with(hosted_config());
        harness
            .runtime
            .set_key("claude", Some("sk-claude".to_owned()))
            .expect("keep");

        let rendered = serde_json::to_string(&settings).expect("settings are JSON");

        assert!(
            !rendered.contains("sk-claude"),
            "an assistant key reached the settings file: {rendered}"
        );
        assert!(!rendered.contains("hasKey"), "whether a key is held is not a setting");

        // Removing the provider is what writes the list back, and the key is gone from the vault by
        // then rather than left behind under an id nothing names.
        harness
            .runtime
            .remove_provider(&mut settings, "claude")
            .expect("remove");
        assert!(settings.ai_providers.is_empty());
        assert_eq!(settings.ai_default_provider_id, None);
        assert!(harness.vault.ai_token("claude").expect("read").is_none());
    }

    #[test]
    fn removing_a_provider_with_a_locked_vault_changes_nothing() {
        let harness = harness(Arc::new(BlockingRunner));
        let mut settings = settings_with(hosted_config());
        harness
            .runtime
            .set_key("claude", Some("sk-claude".to_owned()))
            .expect("keep");
        harness.vault.lock();

        let error = harness
            .runtime
            .remove_provider(&mut settings, "claude")
            .expect_err("a locked vault cannot delete a key");

        assert_eq!(error.code(), ErrorCode::VaultLocked);
        assert_eq!(
            settings.ai_providers.len(),
            1,
            "the list did not lose an entry whose key nobody could remove"
        );
        harness.vault.unlock("master").expect("unlock");
        assert_eq!(
            harness.vault.ai_token("claude").expect("read").expect("kept").expose(),
            "sk-claude"
        );
    }

    #[test]
    fn a_question_with_no_provider_is_refused_before_a_run_exists() {
        let harness = harness(Arc::new(BlockingRunner));
        let settings = AppSettings::default();

        let error = harness
            .runtime
            .start(
                &settings,
                Arc::new(RecordingEvents::default()),
                Arc::new(NoQuestions),
                send("hello"),
            )
            .expect_err("no provider");

        assert_eq!(error.code(), ErrorCode::AiNoProvider);
        assert_eq!(error.message_key(), "errors.ai.noProvider");
        assert_eq!(harness.runtime.active_run(), None);
    }

    #[test]
    fn a_hosted_provider_with_no_key_is_refused_with_the_catalog_code() {
        let harness = harness(Arc::new(BlockingRunner));
        let settings = settings_with(hosted_config());

        let error = harness
            .runtime
            .start(
                &settings,
                Arc::new(RecordingEvents::default()),
                Arc::new(NoQuestions),
                send("hello"),
            )
            .expect_err("no key");

        assert_eq!(error.code(), ErrorCode::AiNoKey);
        assert_eq!(error.message_key(), "errors.ai.noKey");
    }

    #[tokio::test]
    async fn a_run_delivers_its_events_and_files_the_answer() {
        let harness = harness(Arc::new(EchoRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let started = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("hello"))
            .expect("a run");
        assert!(!started.run_id.is_empty());

        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        let seen = events.seen();
        assert_eq!(seen.first().map(|(name, _)| name.as_str()), Some("ai:delta"));
        assert_eq!(
            seen.first().map(|(_, payload)| &payload["runId"]),
            Some(&Value::String(started.run_id.clone())),
            "every event is filed under the run the send answered with"
        );
        assert!(seen.iter().any(|(name, _)| name == "ai:done"));

        let conversation = harness.runtime.conversation();
        assert_eq!(conversation.len(), 2, "the question and the answer");
        assert_eq!(conversation[0], ChatMessage::user("hello"));
        assert_eq!(
            conversation[1],
            ChatMessage::assistant(vec![sevenshell_ai::ContentBlock::Text {
                text: "hello".to_owned()
            }])
        );
    }

    #[tokio::test]
    async fn a_cancel_stops_the_run_and_keeps_what_was_already_said() {
        let harness = harness(Arc::new(BlockingRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let started = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("hello"))
            .expect("a run");

        assert!(harness.runtime.cancel(Some(&started.run_id)));
        assert_eq!(harness.runtime.active_run(), None, "a cancelled run is not producing");

        wait_for(
            &harness.runtime,
            |runtime| runtime.lock_runs().is_empty(),
            "the cancelled run to report back",
        )
        .await;
        assert!(
            events
                .seen()
                .iter()
                .any(|(name, payload)| name == "ai:done"
                    && payload["stopReason"] == Value::String("cancelled".to_owned())),
            "the window is told the run was called off: {:?}",
            events.seen()
        );
        assert_eq!(
            harness.runtime.conversation(),
            vec![ChatMessage::user("hello")],
            "what was already said is kept; nothing invented"
        );
    }

    #[tokio::test]
    async fn a_reset_writes_the_conversation_down_and_opens_an_empty_one() {
        let harness = harness(Arc::new(BlockingRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let started = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("hello"))
            .expect("a run");
        assert!(harness.runtime.active_run().is_some());

        harness
            .runtime
            .reset(None, events.as_ref())
            .expect("a new conversation");

        assert_eq!(harness.runtime.active_run(), None);
        assert!(
            harness.runtime.conversation().is_empty(),
            "the new conversation starts with nothing in it"
        );
        let stored = harness.vault.ai_sessions().expect("read");
        assert_eq!(
            stored.len(),
            1,
            "the conversation being left is in the vault rather than gone"
        );
        let stored = sessions::load(&harness.vault, &started.session_id)
            .expect("read")
            .expect("kept");
        assert_eq!(stored.messages, vec![ChatMessage::user("hello")]);

        // The runner is still winding down; what it would have filed belongs to the conversation
        // that was left and must not appear in the new one.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(harness.runtime.conversation().is_empty());
        assert_eq!(
            sessions::load(&harness.vault, &started.session_id)
                .expect("read")
                .expect("kept")
                .messages,
            vec![ChatMessage::user("hello")],
            "the left conversation is not written into the new one"
        );
    }

    #[tokio::test]
    async fn a_reset_of_a_named_conversation_empties_it_where_it_stands() {
        let harness = harness(Arc::new(EchoRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let started = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("hello"))
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;
        let before = sessions::load(&harness.vault, &started.session_id)
            .expect("read")
            .expect("kept");
        assert_eq!(before.messages.len(), 2);

        harness
            .runtime
            .reset(Some(&started.session_id), events.as_ref())
            .expect("cleared");

        let after = sessions::load(&harness.vault, &started.session_id)
            .expect("read")
            .expect("still there");
        assert!(after.messages.is_empty(), "the row stays and its messages do not");
        assert_eq!(after.name, before.name, "the name is what a person knows the row by");
        assert_eq!(after.created_at, before.created_at);
        assert!(
            events.seen().iter().any(|(name, payload)| name == "ai:session_saved"
                && payload["sessionId"] == Value::String(started.session_id.clone())),
            "the window is told the row is empty rather than guessing: {:?}",
            events.seen()
        );
    }

    #[tokio::test]
    async fn a_finished_run_saves_the_conversation_and_names_it_after_the_first_question() {
        let harness = harness(Arc::new(EchoRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        harness
            .runtime
            .start(
                &settings,
                events.clone(),
                Arc::new(NoQuestions),
                send("  why is nginx returning 502?  "),
            )
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        let rows = harness.runtime.list_sessions(&settings).expect("listed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "why is nginx returning 502?");
        assert_eq!(rows[0].message_count, 2);
        assert!(!rows[0].has_summary);
        assert!(
            events
                .seen()
                .iter()
                .any(|(name, payload)| name == "ai:session_saved" && payload["updatedAt"] == rows[0].updated_at),
            "the save is announced with the timestamp the row now carries: {:?}",
            events.seen()
        );

        let detail = harness.runtime.session(&rows[0].id).expect("the transcript");
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[0]["role"], Value::String("user".to_owned()));
        assert_eq!(
            detail.messages[0]["blocks"][0]["text"],
            Value::String("  why is nginx returning 502?  ".to_owned()),
            "the message is stored as it was sent; only the name is trimmed"
        );
    }

    #[tokio::test]
    async fn a_second_question_continues_the_open_conversation() {
        let harness = harness(Arc::new(EchoRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("first"))
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the first run to end",
        )
        .await;

        let started = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("second"))
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the second run to end",
        )
        .await;

        let rows = harness.runtime.list_sessions(&settings).expect("listed");
        assert_eq!(rows.len(), 1, "one chat, two exchanges");
        assert_eq!(rows[0].id, started.session_id);
        let detail = harness.runtime.session(&rows[0].id).expect("the transcript");
        assert_eq!(detail.messages.len(), 4);
        assert_eq!(
            detail.messages[2]["blocks"][0]["text"],
            Value::String("second".to_owned())
        );
        assert_eq!(
            detail.summary.name, "first",
            "the name comes from the first question and a later one does not rename the row"
        );
    }

    #[tokio::test]
    async fn a_named_conversation_is_loaded_before_the_question_is_appended() {
        let harness = harness(Arc::new(EchoRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let first = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("first"))
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        // Somebody clicked a different chat in the list and asked into it.
        harness
            .runtime
            .reset(None, events.as_ref())
            .expect("a new conversation");
        let second = harness
            .runtime
            .start(
                &settings,
                events.clone(),
                Arc::new(NoQuestions),
                send_in("second chat", &first.session_id),
            )
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        assert_eq!(second.session_id, first.session_id);
        let detail = harness.runtime.session(&first.session_id).expect("the transcript");
        assert_eq!(detail.messages.len(), 4, "the stored exchange came first");
        assert_eq!(
            detail.messages[0]["blocks"][0]["text"],
            Value::String("first".to_owned())
        );
    }

    #[test]
    fn a_send_naming_a_conversation_the_vault_does_not_hold_opens_one_under_that_id() {
        let harness = harness(Arc::new(BlockingRunner));
        let settings = settings_with(local_config());

        let started = harness
            .runtime
            .start(
                &settings,
                Arc::new(RecordingEvents::default()),
                Arc::new(NoQuestions),
                send_in("hello", "a-row-that-was-deleted"),
            )
            .expect("a run");

        assert_eq!(started.session_id, "a-row-that-was-deleted");
        assert_eq!(harness.runtime.conversation(), vec![ChatMessage::user("hello")]);
        assert!(harness.runtime.cancel(Some(&started.run_id)));
    }

    #[tokio::test]
    async fn a_locked_vault_keeps_the_conversation_and_the_next_save_writes_it() {
        let harness = harness(Arc::new(SlowRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let started = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("first"))
            .expect("a run");

        // The vault closes while the answer is being written. The turn ends normally; only the save
        // fails, and a failed save must cost nothing.
        harness.vault.lock();
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        assert!(
            harness.vault.ai_sessions().is_err(),
            "the vault is locked, so nothing could have been written"
        );
        assert_eq!(
            harness.runtime.conversation().len(),
            2,
            "the answer stays in memory when there is nowhere to write it"
        );
        assert!(
            !events.seen().iter().any(|(name, _)| name == "ai:session_saved"),
            "a save that did not happen is not announced"
        );

        harness.vault.unlock("master").expect("unlock");
        harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("second"))
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the second run to end",
        )
        .await;

        let stored = sessions::load(&harness.vault, &started.session_id)
            .expect("read")
            .expect("the whole conversation, including the turn that could not be written");
        assert_eq!(stored.messages.len(), 4);
    }

    #[tokio::test]
    async fn renaming_a_conversation_answers_the_row_the_list_now_draws() {
        let harness = harness(Arc::new(EchoRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let started = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("hello"))
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;
        let saved = harness.runtime.session(&started.session_id).expect("kept").summary;

        let renamed = harness
            .runtime
            .rename_session(&started.session_id, "  nginx 502  ", events.as_ref())
            .expect("renamed");

        assert_eq!(renamed.id, started.session_id);
        assert_eq!(renamed.name, "nginx 502");
        assert_eq!(
            renamed.updated_at, saved.updated_at,
            "a rename is a label, not a message: the list must not reorder for it"
        );
        assert_eq!(
            harness.runtime.session(&started.session_id).expect("kept").summary.name,
            "nginx 502",
            "the conversation in memory carries the new name too"
        );
    }

    #[test]
    fn renaming_a_conversation_that_is_gone_says_so_with_the_catalog() {
        let harness = harness(Arc::new(BlockingRunner));

        let error = harness
            .runtime
            .rename_session("nothing-here", "a name", &RecordingEvents::default())
            .expect_err("nothing to rename");

        assert_eq!(error.code(), ErrorCode::SessionNotFound);
    }

    #[tokio::test]
    async fn deleting_a_conversation_stops_whatever_is_answering_it() {
        let harness = harness(Arc::new(BlockingRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let started = harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("hello"))
            .expect("a run");

        harness.runtime.delete_session(&started.session_id).expect("deleted");

        assert_eq!(harness.runtime.active_run(), None, "a deleted chat has no reader");
        assert!(harness.vault.ai_session(&started.session_id).expect("read").is_none());
        assert!(
            harness.runtime.conversation().is_empty(),
            "and nothing is open any more"
        );

        // The run is still winding down; what it would have filed must not come back.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(harness.vault.ai_session(&started.session_id).expect("read").is_none());
    }

    #[tokio::test]
    async fn a_tool_call_the_run_made_is_restored_as_an_outcome() {
        let harness = harness(Arc::new(ToolRunner));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let started = harness
            .runtime
            .start(
                &settings,
                events.clone(),
                Arc::new(NoQuestions),
                send("list my sessions"),
            )
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        let detail = harness.runtime.session(&started.session_id).expect("the transcript");

        let outcome = detail.outcomes.get("call-1").expect("the card's facts were kept");
        assert_eq!(outcome.call_id, "call-1");
        assert!(outcome.ok);
        assert_eq!(outcome.duration_ms, 42);
        assert_eq!(
            detail.messages.len(),
            4,
            "the question, the call, its result and the answer: a restored card has its block to answer"
        );
        assert_eq!(
            detail.messages[1]["blocks"][0]["type"],
            Value::String("toolUse".to_owned())
        );
    }

    /// A stored conversation long enough that the next question has to compact it.
    fn long_session(id: &str, exchanges: usize) -> StoredSession {
        let mut session = StoredSession::with_id(id, 1_000);
        session.name = "a long conversation".to_owned();
        for index in 0..exchanges {
            session
                .messages
                .push(ChatMessage::user(format!("question {index} {}", "q".repeat(2_000))));
            session
                .messages
                .push(ChatMessage::assistant(vec![sevenshell_ai::ContentBlock::Text {
                    text: format!("answer {index} {}", "a".repeat(2_000)),
                }]));
        }
        session
    }

    #[tokio::test]
    async fn an_over_long_conversation_is_summarised_before_the_run_sees_it() {
        let provider = Arc::new(SummaryProvider {
            answer: Ok("the person is chasing a 502 on a mail host".to_owned()),
        });
        let runner = RecordingRunner::new();
        let harness = harness_with(runner.clone(), Arc::new(FixedProvider(provider as Arc<dyn Provider>)));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        let stored = long_session("session-1", 20);
        sessions::save(&harness.vault, &stored).expect("stored");
        let before = stored.messages.len();

        harness
            .runtime
            .start(
                &settings,
                events.clone(),
                Arc::new(NoQuestions),
                send_in("and now?", "session-1"),
            )
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        let seen = events.seen();
        let compacted = seen
            .iter()
            .find(|(name, _)| name == "ai:compacted")
            .unwrap_or_else(|| panic!("the window is told the older half was folded: {seen:?}"));
        assert_eq!(compacted.1["sessionId"], Value::String("session-1".to_owned()));
        assert_eq!(compacted.1["degraded"], Value::Bool(false));
        assert!(compacted.1["summarizedMessages"].as_u64().unwrap_or(0) > 0);

        let histories = runner.histories();
        assert_eq!(histories.len(), 1);
        let request = &histories[0];
        let first = match &request[0].blocks[0] {
            sevenshell_ai::ContentBlock::Text { text } => text.clone(),
            other => panic!("the request begins with a summary turn: {other:?}"),
        };
        assert!(first.starts_with("<conversation_summary>"));
        assert!(first.contains("the person is chasing a 502 on a mail host"));
        assert!(
            request.iter().flat_map(|message| message.blocks.iter()).all(
                |block| !matches!(block, sevenshell_ai::ContentBlock::Text { text } if text.contains("question 0 "))
            ),
            "the messages the summary replaced are not in the request"
        );
        assert_eq!(
            request.last().map(|message| message.blocks[0].clone()),
            Some(sevenshell_ai::ContentBlock::Text {
                text: "and now?".to_owned()
            }),
            "the newest question is the last thing a provider is asked"
        );

        let after = sessions::load(&harness.vault, "session-1")
            .expect("read")
            .expect("kept");
        assert!(
            after.messages.len() < before + 2,
            "the document keeps the tail rather than the whole conversation: {} messages",
            after.messages.len()
        );
        assert!(
            after.context_summary.is_some(),
            "the summary is what the next run starts from"
        );
        assert_eq!(
            after.messages.last().cloned(),
            Some(ChatMessage::assistant(vec![sevenshell_ai::ContentBlock::Text {
                text: "answered".to_owned(),
            }]))
        );
    }

    #[tokio::test]
    async fn a_failed_summary_still_answers_and_says_it_degraded() {
        let provider = Arc::new(SummaryProvider {
            answer: Err(ProviderError::Server {
                status: 500,
                detail: "boom".to_owned(),
            }),
        });
        let runner = RecordingRunner::new();
        let harness = harness_with(runner.clone(), Arc::new(FixedProvider(provider as Arc<dyn Provider>)));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        sessions::save(&harness.vault, &long_session("session-1", 20)).expect("stored");

        harness
            .runtime
            .start(
                &settings,
                events.clone(),
                Arc::new(NoQuestions),
                send_in("and now?", "session-1"),
            )
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        let seen = events.seen();
        let compacted = seen
            .iter()
            .find(|(name, _)| name == "ai:compacted")
            .unwrap_or_else(|| panic!("a degraded compaction is still announced: {seen:?}"));
        assert_eq!(compacted.1["degraded"], Value::Bool(true));
        assert!(
            seen.iter().any(
                |(name, payload)| name == "ai:done" && payload["stopReason"] == Value::String("endTurn".to_owned())
            ),
            "the run answered rather than failing with the summary: {seen:?}"
        );

        let stored = sessions::load(&harness.vault, "session-1")
            .expect("read")
            .expect("kept");
        assert!(stored.context_summary.is_none(), "nothing was summarised");
        assert_eq!(
            stored.messages.last().map(|message| &message.blocks[0]),
            Some(&sevenshell_ai::ContentBlock::Text {
                text: "answered".to_owned()
            })
        );
    }

    #[tokio::test]
    async fn a_conversation_under_the_trigger_is_not_summarised() {
        let provider = Arc::new(SummaryProvider {
            answer: Ok("never asked".to_owned()),
        });
        let runner = RecordingRunner::new();
        let harness = harness_with(runner.clone(), Arc::new(FixedProvider(provider as Arc<dyn Provider>)));
        let settings = settings_with(local_config());
        let events = Arc::new(RecordingEvents::default());

        harness
            .runtime
            .start(&settings, events.clone(), Arc::new(NoQuestions), send("hello"))
            .expect("a run");
        wait_for(
            &harness.runtime,
            |runtime| runtime.active_run().is_none(),
            "the run to end",
        )
        .await;

        assert!(
            !events.seen().iter().any(|(name, _)| name == "ai:compacted"),
            "a conversation that fits costs no summary request"
        );
        assert_eq!(runner.histories()[0], vec![ChatMessage::user("hello")]);
    }

    #[tokio::test]
    async fn locking_the_vault_stops_the_run_and_drops_its_question() {
        use sevenshell_api::{ApprovalBroker as _, ApprovalDecision, ApprovalRequest};
        use sevenshell_types::RiskAssessment;

        let harness = harness(Arc::new(BlockingRunner));
        let settings = settings_with(local_config());

        harness
            .runtime
            .start(
                &settings,
                Arc::new(RecordingEvents::default()),
                Arc::new(NoQuestions),
                send("hello"),
            )
            .expect("a run");

        // A question of this run is on screen, waiting for a person. It is the assistant's, so the
        // lock is what ends it — as a refusal rather than a dialog nobody will ever answer.
        let broker = WindowBroker::sequenced(
            Arc::clone(harness.shared.approvals()),
            Arc::new(NoQuestions),
            AgentOrigin::Ai,
            "another-run",
        );
        let question = tokio::spawn(async move {
            broker
                .request(ApprovalRequest::risk("sshell_exec_run", RiskAssessment::safe()))
                .await
        });
        for _ in 0..200 {
            if !harness.shared.approvals().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!harness.shared.approvals().is_empty(), "the question was never asked");

        harness.runtime.on_vault_locked();

        assert_eq!(harness.runtime.active_run(), None);
        assert!(
            harness.shared.approvals().is_empty(),
            "no assistant question outlives the key it was asked under"
        );
        assert_eq!(
            question.await.expect("joined").expect("answered"),
            ApprovalDecision::Denied
        );
    }

    #[tokio::test]
    async fn an_assistant_tool_call_is_audited_with_its_own_origin() {
        let harness = harness(Arc::new(BlockingRunner));

        let assistant = ToolContext::new(Arc::clone(harness.shared.connections().core()), McpScope::ReadOnly)
            .with_cache(Arc::clone(harness.shared.connections()))
            .with_audit(Arc::clone(harness.shared.audit()) as Arc<dyn AuditSink>)
            .with_origin(AgentOrigin::Ai);
        let interface = ToolContext::new(Arc::clone(harness.shared.connections().core()), McpScope::ReadOnly)
            .with_cache(Arc::clone(harness.shared.connections()))
            .with_audit(Arc::clone(harness.shared.audit()) as Arc<dyn AuditSink>);

        for context in [&assistant, &interface] {
            let outcome = harness
                .shared
                .registry()
                .call(
                    context,
                    ToolCall {
                        name: "sshell_sessions_list".to_owned(),
                        arguments: serde_json::json!({}),
                    },
                )
                .await;
            assert!(
                matches!(outcome, ToolOutcome::Succeeded { .. } | ToolOutcome::Failed { .. }),
                "the call is an answer either way"
            );
        }

        let records = crate::mcp::audit::read_tail(&harness.audit_path, 10);
        assert_eq!(records.len(), 2, "both callers wrote to the one trail");
        assert_eq!(records[0].origin, AgentOrigin::Ai);
        assert_eq!(records[1].origin, AgentOrigin::Mcp);
    }

    #[tokio::test]
    async fn a_provider_that_answers_begins_a_successful_test() {
        let result = probe(Arc::new(QuietProvider), "any-model").await;

        assert!(result.ok);
        assert_eq!(result.code, None);
    }

    #[tokio::test]
    async fn a_rejected_key_is_the_answer_to_a_test_rather_than_its_failure() {
        let result = probe(Arc::new(RejectingProvider), "any-model").await;

        assert!(!result.ok);
        assert_eq!(result.code.as_deref(), Some("AI-PROVIDER-UNAVAILABLE"));
        assert_eq!(result.message_key.as_deref(), Some("errors.ai.providerUnavailable"));
        assert!(result.message_params.is_empty());
    }

    /// Answers with one event and no more, like a provider that accepted the key.
    struct QuietProvider;

    #[async_trait]
    impl Provider for QuietProvider {
        fn kind(&self) -> AiProviderKind {
            AiProviderKind::Local
        }

        async fn stream(&self, _request: ProviderRequest) -> Result<ProviderStream, ProviderError> {
            Ok(Box::pin(futures_util::stream::iter(vec![Ok(StreamEvent::Stop {
                reason: StopReason::EndTurn,
            })])))
        }
    }

    /// Refuses the credentials, like a provider given the wrong key.
    struct RejectingProvider;

    #[async_trait]
    impl Provider for RejectingProvider {
        fn kind(&self) -> AiProviderKind {
            AiProviderKind::OpenAi
        }

        async fn stream(&self, _request: ProviderRequest) -> Result<ProviderStream, ProviderError> {
            Err(ProviderError::Auth)
        }
    }
}

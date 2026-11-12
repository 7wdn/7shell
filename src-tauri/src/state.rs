//! What the shell keeps between commands.
//!
//! Four long-lived things: the vault (one file, one lock state), the core (the registry of open
//! terminals, plus the receiver their output arrives on), the pieces the tool layer's two callers
//! share — the MCP listener and the assistant ([`SharedAgent`]) — and the external servers this
//! desktop calls. All are created once, in `setup`, and handed to every command through managed
//! state.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use sevenshell_api::local::ConnectionCache;
use sevenshell_api::{AuditRecord, AuditSink, McpAccess, ToolRegistry};
use sevenshell_core::Core;
use sevenshell_mcp_client::McpClientManager;
use sevenshell_types::{AppSettings, McpScope, Session, SessionGroup};
use sevenshell_vault::Vault;

use crate::ai::AiRuntime;
use crate::dropped::PendingDrops;
use crate::mcp::McpRuntime;
use crate::mcp::approval::ApprovalRegistry;
use crate::mcp::audit::AuditLog;
use crate::mcp_client::{McpClientSupervisor, TauriServerStatusEvents};
use crate::uploads::PendingUploads;

/// A file an operation is in the middle of, waiting for an answer about it.
///
/// A sealed file has to be asked about — the passphrase is not in it — and the file has already been
/// chosen by the time that is known. So the choice waits here between the two calls instead of
/// travelling out to the interface and back, for the reason every other path stays in this
/// process: what the interface is told is a name, never somewhere on this disk. One slot per
/// operation, because only one of each can be waiting for an answer at a time.
#[derive(Default)]
pub struct PendingFile {
    path: Mutex<Option<PathBuf>>,
}

impl PendingFile {
    /// Remembers the file an operation is waiting on, replacing whatever was waiting before. A new
    /// operation starting is the end of the old one, whoever asked and however it ended.
    pub fn remember(&self, path: PathBuf) {
        *self.lock() = Some(path);
    }

    /// The file waiting for an answer, or nothing when no question is outstanding.
    pub fn path(&self) -> Option<PathBuf> {
        self.lock().clone()
    }

    /// Drops what was waiting. Called when the operation finishes and when the vault locks: a path is
    /// not a secret, but a question nobody can answer any more should not be left on the table.
    ///
    /// A question that was answered with nothing leaves this standing until the next operation, and
    /// that is deliberate rather than an oversight: it can only be reached by a call carrying an
    /// answer, and there is no question open for one to be given to.
    pub fn clear(&self) {
        *self.lock() = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<PathBuf>> {
        match self.path.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// The pieces both callers of the tool layer use, created once for the shell.
///
/// An MCP client and the assistant panel are two doors onto the same ten tools, and three things
/// must not exist twice:
///
/// * **The tool list.** It is this build's own and does not depend on settings; two lists would let
///   the two callers disagree about what a name means.
/// * **The approval questions.** The window draws one dialog and answers by call id, so both
///   callers' questions park in one registry and one command answers them.
/// * **The connection cache.** A session the assistant opened is the session an MCP call reads
///   through, and the vault lock empties one cache rather than two.
///
/// The audit trail is shared for the same reason in the other direction: two writers appending to
/// one JSONL file — through one sink — is what keeps rotation and retention a rule about the file
/// instead of a rule each caller remembers.
pub struct SharedAgent {
    registry: Arc<ToolRegistry>,
    approvals: Arc<ApprovalRegistry>,
    connections: Arc<ConnectionCache>,
    audit: Arc<SharedAudit>,
    policy: AgentPolicy,
}

/// The resolved session policy both callers read, replaceable while the window runs.
///
/// A handle rather than a field read once: the settings page can hide a session or narrow one
/// session's scope, and the next bridge call must see it without restarting the listener or the
/// assistant. The value itself is cheap to clone and never contains a secret.
#[derive(Clone, Default)]
pub struct AgentPolicy {
    inner: Arc<RwLock<McpAccess>>,
}

impl AgentPolicy {
    pub fn new(access: McpAccess) -> Self {
        Self {
            inner: Arc::new(RwLock::new(access)),
        }
    }

    pub fn get(&self) -> Arc<McpAccess> {
        match self.inner.read() {
            Ok(access) => Arc::new(access.clone()),
            Err(poisoned) => Arc::new(poisoned.into_inner().clone()),
        }
    }

    pub fn set(&self, access: McpAccess) {
        match self.inner.write() {
            Ok(mut current) => *current = access,
            Err(poisoned) => *poisoned.into_inner() = access,
        }
    }
}

/// Folds the settings' group and session overrides onto the saved sessions.
///
/// The API's snapshot is keyed by session id, and the settings are keyed by group and by session —
/// only the host knows which saved session lives in which group, which is why this fold happens
/// here rather than in the tool layer.
///
/// A group's scope is inherited down the tree: the nearest group with a scope decides, so a folder
/// somebody narrowed narrows everything inside it without every child repeating the setting. A
/// session's own entry beats any group. `sessions` is `None` while the vault is locked — group
/// scopes cannot be resolved then, so only the session-level overrides and the visibility allowlist
/// are applied, and the whole snapshot is rebuilt when the vault opens again.
fn resolve_agent_access(
    settings: &AppSettings,
    sessions: Option<&[Session]>,
    groups: Option<&[SessionGroup]>,
) -> McpAccess {
    let policy = &settings.mcp_access_policy;
    let mut session_scopes = BTreeMap::new();

    if let Some(sessions) = sessions {
        let parents: BTreeMap<&str, Option<&str>> = groups
            .unwrap_or_default()
            .iter()
            .map(|group| (group.id.as_str(), group.parent_id.as_deref()))
            .collect();
        for session in sessions {
            let Some(group_id) = session.group_id.as_deref() else {
                continue;
            };
            if let Some(scope) = inherited_group_scope(group_id, &policy.group_scopes, &parents) {
                session_scopes.insert(session.id.clone(), scope);
            }
        }
    }

    // A session's own entry is the highest priority, whatever group it sits in.
    for (id, scope) in &policy.session_scopes {
        session_scopes.insert(id.clone(), *scope);
    }

    McpAccess {
        default_scope: settings.mcp_scope,
        visible_sessions: policy.visible_session_ids.clone(),
        session_scopes,
    }
}

/// The scope the nearest group in this group's ancestry gives, if any.
fn inherited_group_scope(
    group_id: &str,
    scopes: &BTreeMap<String, McpScope>,
    parents: &BTreeMap<&str, Option<&str>>,
) -> Option<McpScope> {
    let mut current = Some(group_id);
    // A tree is not supposed to have a cycle; a guard here keeps a hand-edited file from becoming
    // a hang rather than an answer.
    let mut seen = BTreeSet::new();
    while let Some(id) = current {
        if !seen.insert(id) {
            log::warn!("the session group tree contains a cycle at `{id}`; the walk stops there");
            return None;
        }
        if let Some(scope) = scopes.get(id) {
            return Some(*scope);
        }
        current = parents.get(id).copied().flatten();
    }
    None
}

/// Applies one settings snapshot to the two callers: the resolved access, and the switched-off tools.
fn apply_access_policy(shared: &SharedAgent, vault: &Vault, settings: &AppSettings) {
    let sessions = vault.sessions();
    let groups = vault.groups();
    if sessions.is_err() {
        log::debug!("the agent access policy is applied without the saved sessions: the vault is not open");
    }
    let access = resolve_agent_access(settings, sessions.as_deref().ok(), groups.as_deref().ok());
    shared.set_access(access);
    shared
        .registry()
        .set_disabled_tools(settings.mcp_tool_policy.disabled_tools.clone());
}

impl SharedAgent {
    pub fn new(core: Arc<Core>) -> Self {
        let audit = match crate::mcp::audit::default_path() {
            Ok(path) => SharedAudit::at(Some(path)),
            Err(error) => {
                log::warn!("the agent audit trail has nowhere to live: {error}");
                SharedAudit::at(None)
            }
        };
        Self::with_audit(core, Arc::new(audit))
    }

    /// The same pieces, with the trail somewhere the caller chose. Tests use this so an assertion
    /// about an audit line never writes into the machine's real configuration directory.
    pub(crate) fn with_audit(core: Arc<Core>, audit: Arc<SharedAudit>) -> Self {
        Self {
            registry: Arc::new(sevenshell_api::local::default_registry()),
            approvals: Arc::new(ApprovalRegistry::new()),
            connections: Arc::new(ConnectionCache::new(core)),
            audit,
            policy: AgentPolicy::new(McpAccess::open(sevenshell_types::McpScope::ReadOnly)),
        }
    }

    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }

    pub fn approvals(&self) -> &Arc<ApprovalRegistry> {
        &self.approvals
    }

    pub fn connections(&self) -> &Arc<ConnectionCache> {
        &self.connections
    }

    pub fn audit(&self) -> &Arc<SharedAudit> {
        &self.audit
    }

    pub fn policy_handle(&self) -> AgentPolicy {
        self.policy.clone()
    }

    pub fn access(&self) -> Arc<McpAccess> {
        self.policy.get()
    }

    /// Replaces the resolved policy. The next call from either caller sees it.
    pub fn set_access(&self, access: McpAccess) {
        self.policy.set(access);
    }
}

/// One JSONL trail, written to by both callers.
///
/// [`AuditLog`] is the file — append, rotate, prune — and this is the sharing: the sink is handed to
/// the MCP handler and to every assistant run, and the retention setting arrives with whichever of
/// them is the first to run. A record that arrives before anybody configured the file is written
/// under the default retention rather than dropped: an audit line is evidence, and evidence that
/// depended on which caller happened to speak first would be worse than none.
pub struct SharedAudit {
    path: Option<PathBuf>,
    log: Mutex<Option<AuditLog>>,
}

impl SharedAudit {
    pub fn at(path: Option<PathBuf>) -> Self {
        Self {
            path,
            log: Mutex::new(None),
        }
    }

    /// Points the trail at this installation's retention window, and applies it once.
    ///
    /// Called when the MCP listener starts and before an assistant run: both are the moment a
    /// person's setting is known to matter, and neither is frequent enough for the rebuild to cost
    /// anything.
    pub fn configure(&self, retention_days: u32) {
        let Some(path) = self.path.clone() else {
            log::warn!("the agent audit trail has nowhere to live, so nothing is recorded");
            return;
        };
        let log = AuditLog::new(path, retention_days);
        log.prune(SystemTime::now());
        log::debug!("the agent audit trail is {}", log.path().display());
        *self.lock() = Some(log);
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<AuditLog>> {
        match self.log.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl AuditSink for SharedAudit {
    fn record(&self, record: AuditRecord) {
        let mut guard = self.lock();
        if guard.is_none() {
            let Some(path) = self.path.clone() else {
                log::warn!("an agent call could not be audited: this machine has no configuration directory");
                return;
            };
            let log = AuditLog::new(path, sevenshell_types::MCP_AUDIT_RETENTION_DAYS_DEFAULT);
            log.prune(SystemTime::now());
            *guard = Some(log);
        }
        if let Some(log) = guard.as_ref() {
            // The sink's contract is that a failed write is said out loud and the call goes on.
            AuditSink::record(log, record);
        }
    }
}

/// The application state commands borrow.
pub struct ShellState {
    vault: Arc<Vault>,
    core: Arc<Core>,
    /// The pieces both agent callers share: the one tool list, the one approval registry, the one
    /// connection cache and the one audit trail. Held here as well because the settings page applies
    /// a new access policy through it without either caller being restarted.
    shared: Arc<SharedAgent>,
    /// The loopback listener an MCP bridge calls, and the token this run rotates for it.
    ///
    /// Always present, listening or not: a command that turns it on needs the same object a command
    /// that reads its status does, and an absent runtime would make "off" and "not built yet" look
    /// alike.
    mcp: Arc<McpRuntime>,
    /// The external MCP servers this installation calls, and when they are opened.
    ///
    /// Separate from the listener above because it is the other direction: the listener is somebody
    /// else calling this desktop's tools, this is this desktop calling somebody else's. They share
    /// only the tool table, which is why both hold the same registry.
    mcp_client: Arc<McpClientSupervisor>,
    /// The assistant: the conversation on screen and whichever run is answering.
    ///
    /// The conversation is a document in the vault (one opaque string per chat), written at the end
    /// of each turn; what lives here is the copy the window is looking at, which may be a turn ahead
    /// of the vault while a write is failing.
    ai: Arc<AiRuntime>,
    /// Files the operating system dropped or the clipboard held, waiting to be uploaded.
    pending_drops: PendingDrops,
    /// Uploads that stopped to ask about a name already on the far side, waiting for the answer.
    pending_uploads: PendingUploads,
    /// A session export that stopped to ask for its passphrase, waiting for the answer.
    pending_import: PendingFile,
    /// A backup that has been read and is waiting to be applied, or to be asked about.
    pending_restore: PendingFile,
    /// The release a look at the update channel found, waiting for somebody to install it.
    ///
    /// Not a secret and not read by anything but its own command, but it is a *decision* waiting to
    /// happen: what is installed has to be the release the person was shown.
    pending_update: crate::commands::update::PendingUpdate,
    /// Whether the interface has said that closing the window would throw something away.
    quit_guard: AtomicBool,
    /// Whether closing the window ends the application or puts it away.
    ///
    /// The shell's copy of one setting rather than a read of the settings file: the file is the
    /// interface's to write and this decision is made on the way out, where a failed read would be a
    /// window that would not close.
    close_to_tray: AtomicBool,
    /// Whether the tray icon is on screen, which is what a window can be put away *to*.
    tray_shown: AtomicBool,
    /// Whether the person put the window away and has not asked for it back.
    ///
    /// Two things inside the shell put the window on screen by themselves — the interface saying it
    /// has a frame, and the deadline that covers an interface which never says so — and neither of
    /// them can tell "nobody has shown this yet" from "somebody has just closed it into the tray":
    /// both read as a window that is not visible. So the answer is remembered here, at the one place
    /// the window is hidden on purpose, and the two of them ask before they show anything.
    window_put_away: AtomicBool,
    /// Whether the interface has answered the close question and meant "quit".
    ///
    /// The same shape as the guard above and for the same reason: only the interface can see what is
    /// unsaved, so only the interface can say that it has been dealt with. Without it, a window set to
    /// hide would hide again on the way out of its own quit question.
    quit_approved: AtomicBool,
}

impl ShellState {
    /// Opens the vault at its usual place and builds the core around it.
    ///
    /// The vault is *not* unlocked here: it starts locked, and the interface asks for the password.
    /// The window handle is taken because one of the pieces below answers with an event — an
    /// external server's state — and there is no other moment at which it can be handed over.
    pub fn new(app: &tauri::AppHandle) -> (Self, tokio::sync::mpsc::Receiver<sevenshell_core::CoreEvent>) {
        let path = sevenshell_vault::default_path().unwrap_or_else(|| {
            log::warn!("the configuration directory could not be located; the vault will not persist");
            std::path::PathBuf::from("vault.json")
        });

        let vault = Arc::new(Vault::open_at(path));
        let (core, events) = Core::new(Arc::clone(&vault));
        let shared = Arc::new(SharedAgent::new(Arc::clone(&core)));
        let mcp = Arc::new(McpRuntime::new(Arc::clone(&shared)));
        // The settings are applied a moment later by `apply_startup_shell_settings`, which is the
        // one path that hands the manager both its list, its secrets and the result bound. What
        // starts here is the health loop: it watches the manager for as long as the window lives,
        // retrying a failed server and pinging a ready one, even when no settings page is open.
        let manager = Arc::new(McpClientManager::new());
        let mcp_client = McpClientSupervisor::new(
            manager,
            Arc::clone(shared.registry()),
            Arc::clone(&vault),
            Arc::new(TauriServerStatusEvents::new(app.clone())),
        );
        mcp_client.start();
        let ai = Arc::new(AiRuntime::new(Arc::clone(&vault), Arc::clone(&shared)));
        (
            Self {
                vault,
                core,
                shared,
                mcp,
                mcp_client,
                ai,
                pending_drops: PendingDrops::default(),
                pending_uploads: PendingUploads::default(),
                pending_import: PendingFile::default(),
                pending_restore: PendingFile::default(),
                pending_update: crate::commands::update::PendingUpdate::default(),
                quit_guard: AtomicBool::new(false),
                close_to_tray: AtomicBool::new(true),
                tray_shown: AtomicBool::new(false),
                window_put_away: AtomicBool::new(false),
                quit_approved: AtomicBool::new(false),
            },
            events,
        )
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    /// A handle to the core that can be moved onto another task, for work that outlives a command.
    pub fn core_arc(&self) -> Arc<Core> {
        Arc::clone(&self.core)
    }

    /// The pieces both agent callers share, for a command that has to read the one tool list.
    pub fn shared(&self) -> &Arc<SharedAgent> {
        &self.shared
    }

    /// Recomputes what an agent may see and do, from the settings and the saved sessions.
    ///
    /// Called at startup, after every settings write and after the vault opens — the last one
    /// because a locked vault cannot say which saved session is in which group, so group overrides
    /// are only folded onto sessions once it can. The registry's disabled set is replaced in the
    /// same pass, so the page, the bridge and the assistant agree without a listener restart.
    pub fn apply_agent_policy(&self, settings: &AppSettings) {
        apply_access_policy(&self.shared, &self.vault, settings);
    }

    /// The agent interface's listener and the questions it is waiting on.
    pub fn mcp(&self) -> &McpRuntime {
        &self.mcp
    }

    /// A handle to the agent interface for the one path that outlives a command: the window closing.
    pub fn mcp_arc(&self) -> Arc<McpRuntime> {
        Arc::clone(&self.mcp)
    }

    /// The external servers this installation calls.
    pub fn mcp_client(&self) -> &Arc<McpClientSupervisor> {
        &self.mcp_client
    }

    /// A handle to the external-server supervisor for the paths that outlive a command: the vault
    /// locking and the window closing.
    pub fn mcp_client_arc(&self) -> Arc<McpClientSupervisor> {
        Arc::clone(&self.mcp_client)
    }

    /// The assistant: its one conversation, and whichever run is answering.
    pub fn ai(&self) -> &AiRuntime {
        &self.ai
    }

    /// A handle to the assistant for the same path as [`Self::mcp_arc`]: the window closing.
    pub fn ai_arc(&self) -> Arc<AiRuntime> {
        Arc::clone(&self.ai)
    }

    /// What a drop or a paste left behind, keyed by the token the interface was given.
    pub fn pending_drops(&self) -> &PendingDrops {
        &self.pending_drops
    }

    /// Uploads held while the interface answers the question about names already there.
    pub fn pending_uploads(&self) -> &PendingUploads {
        &self.pending_uploads
    }

    /// The export an import is waiting on a passphrase for.
    pub fn pending_import(&self) -> &PendingFile {
        &self.pending_import
    }

    /// The backup that was read, waiting to be applied.
    pub fn pending_restore(&self) -> &PendingFile {
        &self.pending_restore
    }

    /// The release the update channel offered, waiting to be installed or dropped.
    pub fn pending_update(&self) -> &crate::commands::update::PendingUpdate {
        &self.pending_update
    }

    /// Whether the interface has something a close would throw away.
    ///
    /// A flag the *interface* sets rather than a question the shell answers itself: the shell cannot
    /// see an editor holding unsaved text or a transfer in flight, and a window that refused to
    /// close because Rust guessed wrong would be worse than one that closed too eagerly. Unset is the
    /// default, and it is also what a frontend that has died leaves behind.
    pub fn set_quit_guard(&self, armed: bool) {
        self.quit_guard.store(armed, Ordering::Relaxed);
    }

    pub fn quit_guard(&self) -> bool {
        self.quit_guard.load(Ordering::Relaxed)
    }

    /// Records what the settings say about closing: whether the window should go away rather than
    /// the application ending.
    pub fn set_close_to_tray(&self, put_away: bool) {
        self.close_to_tray.store(put_away, Ordering::Relaxed);
    }

    pub fn close_to_tray(&self) -> bool {
        self.close_to_tray.load(Ordering::Relaxed)
    }

    /// Records whether there is an icon to bring a hidden window back from.
    pub fn set_tray_shown(&self, shown: bool) {
        self.tray_shown.store(shown, Ordering::Relaxed);
    }

    pub fn tray_shown(&self) -> bool {
        self.tray_shown.load(Ordering::Relaxed)
    }

    /// Records that the window was put away on purpose, or that somebody has asked for it back.
    pub fn set_window_put_away(&self, put_away: bool) {
        self.window_put_away.store(put_away, Ordering::Relaxed);
    }

    /// Whether the window is in the tray rather than merely not shown yet.
    pub fn window_put_away(&self) -> bool {
        self.window_put_away.load(Ordering::Relaxed)
    }

    /// Says that the close question has been asked and answered, so the next request is obeyed.
    pub fn approve_quit(&self) {
        self.quit_approved.store(true, Ordering::Relaxed);
        self.quit_guard.store(false, Ordering::Relaxed);
    }

    pub fn quit_approved(&self) -> bool {
        self.quit_approved.load(Ordering::Relaxed)
    }

    /// Locks the vault, and drops every question that was waiting on it.
    ///
    /// One place rather than two, because there are two ways in: the command the interface calls and
    /// the tray icon, which has no interface at all. An import or a restore waiting for a passphrase
    /// goes with it — the next call would write into a locked vault — and "what locking means" must
    /// not be a thing the two entrances can come to disagree about.
    pub fn lock_vault(&self) {
        self.pending_import.clear();
        self.pending_restore.clear();
        // The assistant's run was started with a key that is about to stop existing, and any
        // question it is waiting on belongs to a tool call that can no longer be answered with
        // anything useful. Both go before the vault does.
        self.ai.on_vault_locked();
        // The agent interface's file, container and cluster handles were opened with credentials
        // that are about to stop existing. They go now; the listener itself stays up so a call
        // answers `VAULT-LOCKED` rather than looking like a desktop that is not running. The cache
        // is the one the assistant used too.
        self.mcp.close_connections();
        // An external MCP server's environment and headers came from the same vault. Their sessions
        // are ended and the manager forgets the secrets *before* the vault closes, so a refresh
        // racing the lock cannot reopen one with a credential the person just put away.
        self.mcp_client.on_vault_locked();
        self.vault.lock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_was_cleared_is_no_longer_waiting_for_an_answer() {
        // The second half of an import acts on what was remembered, so forgetting it has to be
        // enough to make that call refuse — a locked vault or a dismissed question must not leave a
        // file a later call would quietly import.
        let pending = PendingFile::default();
        pending.remember(PathBuf::from("library.json"));

        pending.clear();

        assert_eq!(pending.path(), None);
    }

    #[test]
    fn a_new_import_replaces_the_one_that_was_waiting() {
        let pending = PendingFile::default();
        pending.remember(PathBuf::from("first.json"));
        pending.remember(PathBuf::from("second.json"));

        assert_eq!(pending.path(), Some(PathBuf::from("second.json")));
    }

    #[test]
    fn one_trail_records_what_both_callers_did() {
        // Two writers, one file: the assistant writes while no MCP listener is up, and the MCP
        // handler writes through the same sink. A second sink would be a second file to rotate, and
        // a rotation racing another writer is a line that ends up in neither.
        use sevenshell_api::AuditStatus;
        use sevenshell_types::{AgentOrigin, McpScope};

        let directory = tempfile::tempdir().expect("a temp directory");
        let path = crate::mcp::audit::path_in(directory.path());
        let audit = SharedAudit::at(Some(path.clone()));
        audit.configure(30);

        audit.record(AuditRecord::new(
            "sshell_sessions_list",
            McpScope::ReadOnly,
            AuditStatus::Succeeded,
        ));
        audit.record(
            AuditRecord::new("sshell_exec_run", McpScope::ReadOnly, AuditStatus::Failed).with_origin(AgentOrigin::Ai),
        );

        let records = crate::mcp::audit::read_tail(&path, 10);
        assert_eq!(records.len(), 2, "both callers' lines are in the one file");
        assert_eq!(
            records[0].origin,
            AgentOrigin::Mcp,
            "an unnamed caller is the MCP client"
        );
        assert_eq!(records[1].origin, AgentOrigin::Ai);
    }

    #[test]
    fn a_record_before_anybody_configured_the_trail_is_still_written() {
        // The assistant can ask a tool before the MCP listener was ever started, so the first line
        // may arrive with no retention configured. Dropping it would make the trail depend on which
        // caller spoke first.
        use sevenshell_api::AuditStatus;
        use sevenshell_types::{AgentOrigin, McpScope};

        let directory = tempfile::tempdir().expect("a temp directory");
        let path = crate::mcp::audit::path_in(directory.path());
        let audit = SharedAudit::at(Some(path.clone()));

        audit.record(
            AuditRecord::new("sshell_sessions_list", McpScope::ReadOnly, AuditStatus::Succeeded)
                .with_origin(AgentOrigin::Ai),
        );

        assert_eq!(crate::mcp::audit::read_tail(&path, 10).len(), 1);
    }

    // -----------------------------------------------------------------------------------------
    // The agent access snapshot
    // -----------------------------------------------------------------------------------------

    fn group(id: &str, parent: Option<&str>) -> SessionGroup {
        SessionGroup {
            id: id.to_owned(),
            name: id.to_owned(),
            parent_id: parent.map(str::to_owned),
            order: 0,
            pinned: false,
        }
    }

    fn saved_session(id: &str, group_id: Option<&str>) -> Session {
        Session {
            id: id.to_owned(),
            name: id.to_owned(),
            group_id: group_id.map(str::to_owned),
            protocol: sevenshell_types::Protocol::Ssh,
            host: "host.example".to_owned(),
            port: 22,
            user: "root".to_owned(),
            auth: sevenshell_types::SessionAuth::with_password(sevenshell_types::AuthMethod::Password),
            terminal: sevenshell_types::TerminalSettings::default(),
            desktop: sevenshell_types::DesktopSettings::default(),
            telnet: sevenshell_types::TelnetSettings::default(),
            serial: sevenshell_types::SerialSettings::default(),
            local: sevenshell_types::LocalSettings::default(),
            wsl: sevenshell_types::WslSettings::default(),
            container: sevenshell_types::ContainerSettings::default(),
            kubernetes: sevenshell_types::KubernetesSettings::default(),
            webdav: sevenshell_types::WebDavSettings::default(),
            ftp: sevenshell_types::FtpSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 1,
        }
    }

    fn policy_with(
        mcp_scope: McpScope,
        group_scopes: &[(&str, McpScope)],
        session_scopes: &[(&str, McpScope)],
        visible: Option<&[&str]>,
    ) -> AppSettings {
        let mut settings = AppSettings {
            mcp_scope,
            ..AppSettings::default()
        };
        settings.mcp_access_policy.group_scopes = group_scopes
            .iter()
            .map(|(id, scope)| ((*id).to_owned(), *scope))
            .collect();
        settings.mcp_access_policy.session_scopes = session_scopes
            .iter()
            .map(|(id, scope)| ((*id).to_owned(), *scope))
            .collect();
        settings.mcp_access_policy.visible_session_ids =
            visible.map(|ids| ids.iter().map(|id| (*id).to_owned()).collect());
        settings
    }

    #[test]
    fn a_group_scope_reaches_the_sessions_saved_in_it() {
        let groups = vec![group("work", None), group("home", None)];
        let sessions = vec![saved_session("a", Some("work")), saved_session("b", None)];
        let settings = policy_with(McpScope::ReadOnly, &[("work", McpScope::SafeWrite)], &[], None);

        let access = resolve_agent_access(&settings, Some(&sessions), Some(&groups));

        assert_eq!(
            access.default_scope,
            McpScope::ReadOnly,
            "the global scope is still the fallback"
        );
        assert_eq!(
            access.scope_for("a"),
            McpScope::SafeWrite,
            "the group's scope is the session's"
        );
        assert_eq!(
            access.scope_for("b"),
            McpScope::ReadOnly,
            "a session outside every scoped group keeps the default"
        );
        assert_eq!(
            access.max_scope(),
            McpScope::SafeWrite,
            "the registry's early refusal must not be narrower than a session override"
        );
    }

    #[test]
    fn a_session_scope_beats_the_group_it_sits_in() {
        let groups = vec![group("work", None)];
        let sessions = vec![saved_session("a", Some("work")), saved_session("b", Some("work"))];
        let settings = policy_with(
            McpScope::HighRisk,
            &[("work", McpScope::SafeWrite)],
            &[("a", McpScope::ReadOnly)],
            None,
        );

        let access = resolve_agent_access(&settings, Some(&sessions), Some(&groups));

        assert_eq!(
            access.scope_for("a"),
            McpScope::ReadOnly,
            "the session's own entry is the highest priority"
        );
        assert_eq!(
            access.scope_for("b"),
            McpScope::SafeWrite,
            "its neighbour in the same group keeps the group's scope"
        );
    }

    #[test]
    fn a_nested_group_inherits_the_nearest_scope_up_the_tree() {
        let groups = vec![group("work", None), group("rack", Some("work"))];
        let sessions = vec![
            saved_session("in-rack", Some("rack")),
            saved_session("in-work", Some("work")),
        ];
        let settings = policy_with(
            McpScope::ReadOnly,
            &[("work", McpScope::HighRisk), ("rack", McpScope::SafeWrite)],
            &[],
            None,
        );

        let access = resolve_agent_access(&settings, Some(&sessions), Some(&groups));

        assert_eq!(
            access.scope_for("in-rack"),
            McpScope::SafeWrite,
            "the folder nearest the session decides"
        );
        assert_eq!(
            access.scope_for("in-work"),
            McpScope::HighRisk,
            "the parent's own sessions keep the parent's scope"
        );
    }

    #[test]
    fn a_locked_vault_still_applies_visibility_and_session_overrides() {
        // The vault cannot say which saved session is in which group while it is locked, so the
        // group fold is impossible — but an allowlist and a session's own scope are session ids in
        // the settings and do not need the vault at all.
        let settings = policy_with(
            McpScope::ReadOnly,
            &[("work", McpScope::SafeWrite)],
            &[("a", McpScope::HighRisk)],
            Some(&["a"]),
        );

        let access = resolve_agent_access(&settings, None, None);

        assert_eq!(access.scope_for("a"), McpScope::HighRisk);
        assert_eq!(
            access.scope_for("b"),
            McpScope::ReadOnly,
            "no group was folded, so everything else is the default"
        );
        assert!(access.visible("a"));
        assert!(
            !access.visible("b"),
            "the allowlist applies while the vault is locked too"
        );
    }

    #[test]
    fn applying_a_policy_replaces_the_snapshot_and_the_disabled_set_together() {
        // The two halves the settings page changes must move in one pass: a tool switched off while
        // a scope stays behind is a page that disagrees with the bridge.
        let directory = tempfile::tempdir().expect("a temp directory");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("master").expect("a new vault");
        vault
            .save_session(saved_session("a", Some("work")), None)
            .expect("a saved session");
        vault.save_group(group("work", None)).expect("a saved group");

        let (core, events) = Core::new(Arc::clone(&vault));
        drop(events);
        let shared = SharedAgent::with_audit(core, Arc::new(SharedAudit::at(None)));

        let mut settings = policy_with(McpScope::ReadOnly, &[("work", McpScope::SafeWrite)], &[], None);
        settings
            .mcp_tool_policy
            .disabled_tools
            .insert("sshell_exec_run".to_owned());

        apply_access_policy(&shared, &vault, &settings);

        assert_eq!(shared.access().scope_for("a"), McpScope::SafeWrite);
        assert!(shared.registry().is_disabled("sshell_exec_run"));
        assert!(shared.registry().descriptor("sshell_exec_run").is_none());

        // Locking the vault removes what only the vault could say: the group scope falls back to the
        // default, while the switched-off tool stays switched off.
        vault.lock();
        apply_access_policy(&shared, &vault, &settings);
        assert_eq!(
            shared.access().scope_for("a"),
            McpScope::ReadOnly,
            "a locked vault cannot resolve groups; the default is the honest answer until it opens"
        );
        assert!(shared.registry().is_disabled("sshell_exec_run"));
    }
}

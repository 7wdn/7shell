//! What the shell keeps between commands.
//!
//! Three long-lived things: the vault (one file, one lock state), the core (the registry of open
//! terminals, plus the receiver their output arrives on), and the pieces the tool layer's two
//! callers share — the MCP listener and the assistant ([`SharedAgent`]). All are created once, in
//! `setup`, and handed to every command through managed state.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use sevenshell_api::local::ConnectionCache;
use sevenshell_api::{AuditRecord, AuditSink, ToolRegistry};
use sevenshell_core::Core;
use sevenshell_vault::Vault;

use crate::ai::AiRuntime;
use crate::dropped::PendingDrops;
use crate::mcp::McpRuntime;
use crate::mcp::approval::ApprovalRegistry;
use crate::mcp::audit::AuditLog;
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
    /// The loopback listener an MCP bridge calls, and the token this run rotates for it.
    ///
    /// Always present, listening or not: a command that turns it on needs the same object a command
    /// that reads its status does, and an absent runtime would make "off" and "not built yet" look
    /// alike.
    mcp: Arc<McpRuntime>,
    /// The assistant panel's conversation and the run it is answering right now.
    ///
    /// In memory only, deliberately: a conversation is the person's own typing and the terminal
    /// text they chose to attach, and neither is worth a second encrypted store of its own.
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
    pub fn new() -> (Self, tokio::sync::mpsc::Receiver<sevenshell_core::CoreEvent>) {
        let path = sevenshell_vault::default_path().unwrap_or_else(|| {
            log::warn!("the configuration directory could not be located; the vault will not persist");
            std::path::PathBuf::from("vault.json")
        });

        let vault = Arc::new(Vault::open_at(path));
        let (core, events) = Core::new(Arc::clone(&vault));
        let shared = Arc::new(SharedAgent::new(Arc::clone(&core)));
        let mcp = Arc::new(McpRuntime::new(Arc::clone(&shared)));
        let ai = Arc::new(AiRuntime::new(Arc::clone(&vault), Arc::clone(&shared)));
        (
            Self {
                vault,
                core,
                mcp,
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

    /// The agent interface's listener and the questions it is waiting on.
    pub fn mcp(&self) -> &McpRuntime {
        &self.mcp
    }

    /// A handle to the agent interface for the one path that outlives a command: the window closing.
    pub fn mcp_arc(&self) -> Arc<McpRuntime> {
        Arc::clone(&self.mcp)
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
}

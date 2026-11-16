//! What the policy matrix decided, and when: one line per decision, on this machine.
//!
//! The seventh invariant is only worth something if the decisions are written down: a matrix that
//! refuses an action has to leave the fact that it refused. `sevenshell-core` decides and hands each
//! event to a [`PolicyJournal`]; this module is the file that journal writes to — the same JSONL
//! mechanics as the agent audit ([`crate::jsonl`]), in the same directory, under its own name.
//!
//! Nothing here adds a secret. [`PolicyAuditEvent`] carries the operation, the risk, the verdict,
//! the layer that produced it and the target's names; the subject is whatever core passed, which is a
//! rule id or an already-redacted command. The event is written exactly as core built it — there is
//! no second place deciding what is safe to keep — and nothing in this module ever formats an event
//! into a log line: the trail is for the file, not for the console.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use sevenshell_error::ShellError;
use sevenshell_policy::{PolicyAuditEvent, PolicyJournal};
use sevenshell_types::MCP_AUDIT_RETENTION_DAYS_DEFAULT;

use crate::jsonl::JsonlFile;

/// The directory inside the configuration directory that holds the trail.
pub const AUDIT_DIRECTORY: &str = "audit";

/// The trail's file name.
pub const AUDIT_FILE_NAME: &str = "policy-audit.jsonl";

/// Where the trail lives, whether or not it exists yet.
pub fn default_path() -> Result<PathBuf, ShellError> {
    let directory = sevenshell_platform::config_dir()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this machine"))?;
    Ok(path_in(&directory))
}

/// The trail's place inside one configuration directory.
///
/// Beside the agent audit rather than inside it: the two answer different questions and the settings
/// page reads them separately, but one directory is the unit whose ownership the platform keeps.
pub fn path_in(directory: &Path) -> PathBuf {
    directory.join(AUDIT_DIRECTORY).join(AUDIT_FILE_NAME)
}

/// The policy trail, shared by every caller that makes a decision.
///
/// One instance per process, behind an `Arc`: rotation and append must be one step, and two writers
/// sharing no lock is how a rename and an append interleave into a line that ends up in neither file.
/// The retention window is replaced in place when the settings say so; the writer itself is not
/// recreated, so a settings save cannot leave a second trail behind.
pub struct PolicyLog {
    path: Option<PathBuf>,
    max_bytes: u64,
    file: Mutex<Option<JsonlFile>>,
}

impl PolicyLog {
    pub fn new(path: PathBuf) -> Self {
        Self::at(Some(path))
    }

    /// A log that may have nowhere to write, for a machine with no configuration directory.
    pub fn at(path: Option<PathBuf>) -> Self {
        Self {
            path,
            max_bytes: crate::jsonl::MAX_FILE_BYTES,
            file: Mutex::new(None),
        }
    }

    /// Where the trail is, for the command that reads it back. `None` only when this machine has no
    /// configuration directory, which is also when nothing is recorded.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Points the trail at this installation's retention window, and applies it once.
    ///
    /// Called from the same pass that hands the matrix to the core, so the file and the rules move
    /// together. Repeated applies with the same window are a no-op: rebuilding the writer on every
    /// settings save would be a prune per save and, worse, a second object that could race the first.
    pub fn configure(&self, retention_days: u32) {
        let Some(path) = self.path.clone() else {
            log::warn!("the policy audit trail has nowhere to live, so nothing is recorded");
            return;
        };
        let mut guard = self.lock();
        if guard
            .as_ref()
            .is_some_and(|file| file.retention_days() == retention_days)
        {
            return;
        }
        let file = JsonlFile::with_max_bytes(path, retention_days, self.max_bytes);
        file.prune(SystemTime::now());
        log::debug!("the policy audit trail is {}", file.path().display());
        *guard = Some(file);
    }

    fn lock(&self) -> MutexGuard<'_, Option<JsonlFile>> {
        match self.file.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// The path alone: an event holds a subject, and a `Debug` that printed the log's contents would put
/// a remote path or a command line into every diagnostic that touches the log.
impl fmt::Debug for PolicyLog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PolicyLog")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PolicyJournal for PolicyLog {
    fn record(&self, event: PolicyAuditEvent) {
        let Some(path) = self.path.clone() else {
            log::warn!("a policy decision could not be audited: this machine has no configuration directory");
            return;
        };

        let mut guard = self.lock();
        if guard.is_none() {
            // A decision can arrive before the settings were applied — the first tool call may beat
            // the startup pass. It is written under the default window rather than dropped: evidence
            // must not depend on which caller happened to speak first.
            let file = JsonlFile::with_max_bytes(path, MCP_AUDIT_RETENTION_DAYS_DEFAULT, self.max_bytes);
            *guard = Some(file);
        }

        if let Some(file) = guard.as_ref()
            && let Err(error) = file.append_record(&event)
        {
            // The action has already happened and the decision was already made; a failed write
            // cannot undo either. It is said out loud and the call goes on — and the event is never
            // formatted here, because its subject can be a remote path or a command line.
            log::warn!("a policy audit line could not be written: {error}");
        }
    }
}

/// The last `limit` decisions, newest last, from the active file and its rotated predecessor.
///
/// The cap is the MCP audit's: this answers a page, not a log viewer, and a caller asking for more
/// than a page is a caller that should be paging.
pub fn read_tail(path: &Path, limit: usize) -> Vec<PolicyAuditEvent> {
    crate::jsonl::read_tail(path, limit)
}

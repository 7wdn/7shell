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

    /// A log with a smaller rotation point, so rotation is testable without writing eight mebibytes.
    #[cfg(test)]
    fn with_max_bytes(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            path: Some(path),
            max_bytes,
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

    /// Applies the retention rule at `now`; the tests use this to age a file without waiting.
    #[cfg(test)]
    fn prune_at(&self, now: SystemTime) {
        let guard = self.lock();
        if let Some(file) = guard.as_ref() {
            file.prune(now);
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

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_types::{PolicyAction, RiskAssessment};
    use std::time::Duration;

    fn event(at: u64, action: &str) -> PolicyAuditEvent {
        PolicyAuditEvent {
            at,
            action: action.to_owned(),
            risk: RiskAssessment::safe(),
            decision: PolicyAction::Ask,
            source: "host:web".to_owned(),
            subject: Some("a command with a secret-shaped argument".to_owned()),
            session_id: Some("session-1".to_owned()),
            group_id: Some("lab".to_owned()),
            host: Some("web".to_owned()),
            directory: Some("/srv/data".to_owned()),
        }
    }

    fn rotated(path: &Path) -> PathBuf {
        crate::jsonl::rotated_path(path)
    }

    #[test]
    fn a_core_decision_reaches_the_trail() {
        // The file is only worth its mechanics if the core's decision actually arrives through the
        // trait the host hands it. This is the one test that crosses that seam: a real core, the
        // real `PolicyJournal` implementation, one line on disk.
        use sevenshell_policy::PolicyTarget;
        use std::sync::Arc;

        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = Arc::new(PolicyLog::new(path.clone()));
        let (core, events) = sevenshell_core::Core::new(Arc::new(sevenshell_vault::Vault::open_at(
            directory.path().join("vault.json"),
        )));
        drop(events);
        let journal: Arc<dyn PolicyJournal> = log.clone();
        core.set_policy(sevenshell_types::PolicyMatrix::default(), Some(journal));

        let decision = core.policy_decision(
            "sshell_batch_exec",
            &RiskAssessment::safe(),
            PolicyTarget {
                host: Some("web"),
                ..PolicyTarget::default()
            },
            Some("session-1"),
            Some("uptime"),
        );

        assert_eq!(decision.action, PolicyAction::Allow);
        let tail = read_tail(&path, 10);
        assert_eq!(tail.len(), 1, "one decision, one line");
        assert_eq!(tail[0].action, "sshell_batch_exec");
        assert_eq!(tail[0].source, "global");
        assert_eq!(tail[0].session_id.as_deref(), Some("session-1"));
        assert_eq!(tail[0].host.as_deref(), Some("web"));
        assert_eq!(tail[0].subject.as_deref(), Some("uptime"));
    }

    #[test]
    fn the_trail_lives_beside_the_agent_audit_in_the_configuration_directory() {
        let directory = tempfile::tempdir().expect("a temp directory");

        let path = path_in(directory.path());

        assert_eq!(path, directory.path().join("audit").join("policy-audit.jsonl"));
        assert_eq!(path, directory.path().join(AUDIT_DIRECTORY).join(AUDIT_FILE_NAME));
    }

    #[test]
    fn one_decision_is_one_camel_case_json_line() {
        // The interface deserializes these into the same wire type the core decided with, so the
        // keys have to be the type's camelCase ones and nothing else.
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = PolicyLog::new(path.clone());

        log.record(event(1_700_000_000, "sshell_batch_exec"));

        let body = std::fs::read_to_string(&path).expect("the trail was written");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 1, "one event is exactly one line");
        let rendered: serde_json::Value = serde_json::from_str(lines[0]).expect("a JSON line");
        for key in [
            "at",
            "action",
            "risk",
            "decision",
            "source",
            "subject",
            "sessionId",
            "groupId",
            "host",
            "directory",
        ] {
            assert!(rendered.get(key).is_some(), "the interface reads `{key}`: {rendered}");
        }
        for key in ["session_id", "group_id"] {
            assert!(rendered.get(key).is_none(), "`{key}` is not the wire shape: {rendered}");
        }
        assert_eq!(rendered["at"], serde_json::json!(1_700_000_000));
        assert_eq!(rendered["decision"], serde_json::json!("ask"));
        assert_eq!(rendered["risk"]["level"], serde_json::json!("safe"));
        assert_eq!(rendered["source"], serde_json::json!("host:web"));

        assert_eq!(
            read_tail(&path, 10),
            vec![event(1_700_000_000, "sshell_batch_exec")],
            "a line reads back as the event that was recorded"
        );
    }

    #[test]
    fn the_tail_fills_from_the_rotated_file_and_the_active_one() {
        // Small enough that every event after the first rotates: rotation is a rule about the file,
        // not about eight mebibytes being written in a test.
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = PolicyLog::with_max_bytes(path.clone(), 64);

        log.record(event(1, "sshell_batch_exec"));
        log.record(event(2, "sshell_exec_run"));
        log.record(event(3, "sshell_sftp_list"));

        assert!(rotated(&path).is_file(), "the older decision moved aside");
        let tail = read_tail(&path, 10);
        assert_eq!(tail.len(), 2, "the rotated file comes before the active one");
        assert_eq!(tail[0].at, 2);
        assert_eq!(tail[1].at, 3);
        assert_eq!(tail[1].action, "sshell_sftp_list");
        assert_eq!(
            read_tail(&path, 1).len(),
            1,
            "the newest line is what a short tail sees"
        );
    }

    #[test]
    fn a_trail_older_than_the_retention_window_is_dropped_with_its_rotation() {
        // The window comes from `mcp_audit_retention_days`, the setting the agent audit already uses;
        // there is no second knob to keep in step.
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = PolicyLog::with_max_bytes(path.clone(), 64);
        log.configure(1);
        log.record(event(1, "sshell_batch_exec"));
        log.record(event(2, "sshell_exec_run"));

        log.prune_at(SystemTime::now() + Duration::from_secs(2 * 24 * 60 * 60));

        assert!(!path.exists(), "the active file had expired");
        assert!(!rotated(&path).exists(), "the rotated file had expired too");
        assert!(read_tail(&path, 10).is_empty());
    }

    #[test]
    fn a_decision_that_arrives_before_the_settings_is_still_written() {
        // The first tool call can beat the startup settings pass. Dropping its decision would make
        // the trail depend on which caller spoke first, which is the one thing evidence must not do.
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = PolicyLog::new(path.clone());

        log.record(event(1, "sshell_batch_exec"));

        assert_eq!(read_tail(&path, 10).len(), 1);
    }

    #[test]
    fn a_tail_asks_for_no_more_than_the_cap() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = PolicyLog::new(path.clone());

        for at in 0..(crate::jsonl::MAX_TAIL_LINES as u64 + 5) {
            log.record(event(at, "sshell_batch_exec"));
        }

        let tail = read_tail(&path, usize::MAX);
        assert_eq!(tail.len(), crate::jsonl::MAX_TAIL_LINES);
        assert_eq!(
            tail.last().map(|event| event.at),
            Some(crate::jsonl::MAX_TAIL_LINES as u64 + 4),
            "the cap keeps the newest lines"
        );
    }

    #[test]
    fn a_trail_that_cannot_be_written_is_reported_and_the_decision_still_stands() {
        // A configuration directory that is a file where the audit directory should be: every write
        // fails, and none of that may turn into a panic or a stolen decision.
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        std::fs::write(directory.path().join(AUDIT_DIRECTORY), b"not a directory").expect("the blocker");
        let log = PolicyLog::new(path.clone());

        log.record(event(1, "sshell_batch_exec"));

        assert!(
            read_tail(&path, 10).is_empty(),
            "there is nowhere to read from, and no panic"
        );
    }

    #[test]
    fn the_logs_debug_never_carries_a_subject() {
        // A diagnostic that printed the log would put a command line into every log line that names
        // it. The path is the only thing on this object a diagnostic may show.
        let directory = tempfile::tempdir().expect("a temp directory");
        let log = PolicyLog::new(path_in(directory.path()));
        log.record(event(1, "sshell_batch_exec"));

        let rendered = format!("{log:?}");

        assert!(rendered.contains("PolicyLog"));
        assert!(
            !rendered.contains("a command with a secret-shaped argument"),
            "a subject never reaches a diagnostic: {rendered}"
        );
    }
}

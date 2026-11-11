//! What the agent interface leaves behind: one line per call, on this machine.
//!
//! The registry writes a record for every call — the ones that ran, the ones that failed and the
//! ones the scope refused — so the file answers "what was asked" and not only "what worked". This
//! module is the file: a JSONL append, a size-bounded rotation, and a retention rule so a machine
//! nobody looked at for a year does not carry a year of history.
//!
//! Nothing added here is a secret. The subject is a command line or a remote path and the tool
//! inputs hold no credentials by construction; the record is written exactly as the registry built
//! it, so there is no second place that decides what is safe to keep.
//!
//! Owner-only, like the endpoint file: a trail of which servers somebody's agent talked to is not
//! something another user on the machine has any business reading. Windows inherits the access
//! control list of the configuration directory for the same reason the endpoint file does (see
//! `endpoint`).

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sevenshell_api::{AuditRecord, AuditSink};
use sevenshell_error::ShellError;

/// The directory inside the configuration directory that holds the trail.
pub const AUDIT_DIRECTORY: &str = "audit";

/// The trail's file name.
pub const AUDIT_FILE_NAME: &str = "mcp-audit.jsonl";

/// How large the trail may grow before it is rotated.
///
/// Eight mebibytes is roughly forty thousand lines of the records this interface writes: weeks of
/// ordinary use, and small enough that reading the tail costs nothing. The rotated file is kept as
/// `.1` and nothing older: the retention setting decides whether even that one stays.
pub const MAX_AUDIT_BYTES: u64 = 8 * 1024 * 1024;

/// The most lines a tail read may ask for.
///
/// The interface draws a panel, not a log viewer; a request for more than this is a caller that
/// should be paging, and answering it would put the whole file through IPC.
pub const MAX_TAIL_LINES: usize = 1_000;

/// Where the trail lives, whether or not it exists yet.
pub fn default_path() -> Result<PathBuf, ShellError> {
    let directory = sevenshell_platform::config_dir()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this machine"))?;
    Ok(path_in(&directory))
}

/// The trail's place inside one configuration directory.
pub fn path_in(directory: &Path) -> PathBuf {
    directory.join(AUDIT_DIRECTORY).join(AUDIT_FILE_NAME)
}

/// One audit file, with the two limits this installation set for it.
#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
    retention_days: u32,
    max_bytes: u64,
}

impl AuditLog {
    pub fn new(path: PathBuf, retention_days: u32) -> Self {
        Self {
            path,
            retention_days,
            max_bytes: MAX_AUDIT_BYTES,
        }
    }

    /// A log with a smaller rotation point, so rotation is testable without writing eight mebibytes.
    #[cfg(test)]
    fn with_max_bytes(path: PathBuf, retention_days: u32, max_bytes: u64) -> Self {
        Self {
            path,
            retention_days,
            max_bytes,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Applies the retention rule once, at the start of a run.
    ///
    /// Both files are judged by when they were last written. The active one is only dropped when the
    /// whole file is older than the window — every record in it has expired, and keeping it would be
    /// keeping a trail this run is about to append next to. A fresh active file is left alone however
    /// many records it holds: it is the one the next call extends.
    pub fn prune(&self, now: SystemTime) {
        if let Ok(metadata) = std::fs::metadata(&self.path)
            && metadata
                .modified()
                .is_ok_and(|modified| retention_expired(modified, now, self.retention_days))
        {
            match std::fs::remove_file(&self.path) {
                Ok(()) => log::info!("the agent audit trail was older than its retention window and was dropped"),
                Err(error) => log::warn!("{} could not be dropped: {error}", self.path.display()),
            }
        }

        let rotated = rotated_path(&self.path);
        if let Ok(metadata) = std::fs::metadata(&rotated)
            && metadata
                .modified()
                .is_ok_and(|modified| retention_expired(modified, now, self.retention_days))
        {
            let _ = std::fs::remove_file(&rotated);
        }
    }

    fn append(&self, record: &AuditRecord) {
        match self.try_append(record) {
            Ok(()) => {}
            // Infallible by contract: the action has already happened and an audit write that failed
            // the call could not undo it. The failure is said out loud and the call goes on.
            Err(error) => log::warn!("the agent audit line could not be written: {error}"),
        }
    }

    fn try_append(&self, record: &AuditRecord) -> Result<(), ShellError> {
        let mut line = serde_json::to_vec(record)
            .map_err(|error| ShellError::store_io(format!("an audit record could not be serialized: {error}")))?;
        line.push(b'\n');

        if let Some(directory) = self.path.parent() {
            std::fs::create_dir_all(directory)
                .map_err(|error| ShellError::store_io(format!("cannot create {}: {error}", directory.display())))?;
        }

        self.rotate_if_needed(line.len() as u64)?;

        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }

        let mut file = options
            .open(&self.path)
            .map_err(|error| ShellError::store_io(format!("cannot open {}: {error}", self.path.display())))?;

        // A file somebody else created — restored from a backup, copied in by hand — keeps its old
        // mode through an append. Tightened here rather than only at creation, because the rule is
        // about what is in the file, not about which call made it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        }

        file.write_all(&line)
            .map_err(|error| ShellError::store_io(format!("cannot write {}: {error}", self.path.display())))?;
        file.flush()
            .map_err(|error| ShellError::store_io(format!("cannot flush {}: {error}", self.path.display())))?;
        Ok(())
    }

    /// Moves the active file aside when this line would take it over the limit.
    ///
    /// One rotated file, replaced rather than numbered: the retention window makes a chain
    /// pointless, and a directory that grows a file per week is a directory somebody has to clean up
    /// by hand.
    fn rotate_if_needed(&self, incoming: u64) -> Result<(), ShellError> {
        let current = std::fs::metadata(&self.path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if current == 0 || current.saturating_add(incoming) <= self.max_bytes {
            return Ok(());
        }

        let rotated = rotated_path(&self.path);
        match std::fs::remove_file(&rotated) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ShellError::store_io(format!(
                    "the previous audit file {} could not be replaced: {error}",
                    rotated.display()
                )));
            }
        }
        std::fs::rename(&self.path, &rotated).map_err(|error| {
            ShellError::store_io(format!(
                "the audit trail {} could not be rotated: {error}",
                self.path.display()
            ))
        })?;
        Ok(())
    }
}

impl AuditSink for AuditLog {
    fn record(&self, record: AuditRecord) {
        self.append(&record);
    }
}

/// The last `limit` records, newest last, from one audit file.
///
/// A line that does not deserialize is skipped rather than failing the read: a file interrupted by a
/// power cut ends in half a line, and that half line must not hide the ninety-nine whole ones before
/// it.
pub fn read_tail(path: &Path, limit: usize) -> Vec<AuditRecord> {
    let limit = limit.min(MAX_TAIL_LINES);
    if limit == 0 {
        return Vec::new();
    }

    let mut records = read_records(path);
    // The active file may be shorter than the window somebody asked for, especially on a fresh run
    // right after a rotation. The rotated file is older, so it goes in front.
    if records.len() < limit {
        let mut older = read_records(&rotated_path(path));
        older.append(&mut records);
        records = older;
    }

    let start = records.len().saturating_sub(limit);
    records[start..].to_vec()
}

/// Every readable record in one file.
fn read_records(path: &Path) -> Vec<AuditRecord> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!("{} could not be read: {error}", path.display());
            return Vec::new();
        }
    };

    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<AuditRecord>(line).ok())
        .collect()
}

fn rotated_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

/// Whether a file last written at `modified` is outside the retention window at `now`.
///
/// A separate function rather than inline arithmetic so the rule can be checked with two timestamps
/// instead of by waiting a month.
pub(crate) fn retention_expired(modified: SystemTime, now: SystemTime, days: u32) -> bool {
    let window = Duration::from_secs(u64::from(days) * 24 * 60 * 60);
    now.duration_since(modified).is_ok_and(|age| age > window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_api::{AuditStatus, ToolAudit};
    use sevenshell_types::McpScope;

    fn now() -> SystemTime {
        SystemTime::now()
    }

    fn record(tool: &str) -> AuditRecord {
        AuditRecord::new(tool, McpScope::ReadOnly, AuditStatus::Succeeded)
            .with_duration_ms(12)
            .with_audit(Some(ToolAudit::for_session("session-1").with_subject("ls -la")))
    }

    #[test]
    fn the_tail_is_the_last_lines_in_the_order_they_were_written() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = AuditLog::new(path.clone(), 30);

        for tool in ["sshell_sessions_list", "sshell_sftp_list", "sshell_exec_run"] {
            log.record(record(tool));
        }

        let tail = read_tail(&path, 2);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].tool, "sshell_sftp_list");
        assert_eq!(tail[1].tool, "sshell_exec_run");
        assert_eq!(tail[1].subject.as_deref(), Some("ls -la"));
    }

    #[test]
    fn a_line_interrupted_in_the_middle_is_skipped_and_the_rest_still_reads() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("the directory");
        let mut body = serde_json::to_string(&record("sshell_sessions_list")).expect("json");
        body.push('\n');
        body.push_str("{\"at\":1,\"tool\":\"sshell_exec");
        std::fs::write(&path, body).expect("written");

        let tail = read_tail(&path, 10);

        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].tool, "sshell_sessions_list");
    }

    #[test]
    fn a_tail_of_a_file_that_was_never_written_is_empty() {
        let directory = tempfile::tempdir().expect("a temp directory");
        assert!(read_tail(&path_in(directory.path()), 10).is_empty());
    }

    #[test]
    fn the_file_rotates_rather_than_growing_without_bound() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        // Small enough that every record after the first rotates: rotation is a rule about the file,
        // not about eight mebibytes being written in a test.
        let log = AuditLog::with_max_bytes(path.clone(), 30, 64);

        log.record(record("sshell_sessions_list"));
        log.record(record("sshell_sftp_list"));
        log.record(record("sshell_exec_run"));

        let rotated = rotated_path(&path);
        assert!(rotated.is_file(), "the older records moved aside");
        assert_eq!(read_tail(&rotated, 10).len(), 1);
        assert_eq!(
            read_tail(&path, 10).len(),
            2,
            "the tail fills from the rotated file too"
        );
        assert_eq!(
            read_tail(&path, 10).last().map(|record| record.tool.as_str()),
            Some("sshell_exec_run")
        );
        assert_eq!(read_tail(&path, 2).len(), 2, "a tail fills from the rotated file too");
    }

    #[cfg(unix)]
    #[test]
    fn the_audit_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        AuditLog::new(path.clone(), 30).record(record("sshell_sessions_list"));

        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_wider_mode_on_an_existing_file_is_tightened_by_the_next_write() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("the directory");
        std::fs::write(&path, "").expect("created");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("a wide mode");

        AuditLog::new(path.clone(), 30).record(record("sshell_sessions_list"));

        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_file_older_than_the_retention_window_is_expired() {
        let written = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 24 * 60 * 60);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(131 * 24 * 60 * 60);

        assert!(retention_expired(written, now, 30));
        assert!(!retention_expired(written, now, 31));
        assert!(!retention_expired(now, now, 30));
    }

    #[test]
    fn pruning_keeps_a_fresh_active_file() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = AuditLog::with_max_bytes(path.clone(), 30, 200);
        // Two records in a 200-byte file: the second rotates the first aside.
        log.record(record("sshell_sessions_list"));
        log.record(record("sshell_sftp_list"));

        log.prune(now());

        assert!(path.is_file(), "the file the next call appends to is not dropped");
    }

    #[test]
    fn pruning_drops_every_file_outside_the_window() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());
        let log = AuditLog::with_max_bytes(path.clone(), 1, 200);
        log.record(record("sshell_sessions_list"));
        log.record(record("sshell_sftp_list"));

        // A day later than the window: both the active file and the rotated one have expired.
        log.prune(now() + Duration::from_secs(2 * 24 * 60 * 60));

        assert!(!path.exists());
        assert!(!rotated_path(&path).exists());
    }
}

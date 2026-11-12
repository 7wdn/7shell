//! One append-only JSONL file, and the two limits every trail in this shell shares.
//!
//! The agent audit and the policy audit answer different questions — "what did an agent ask for"
//! and "what did this installation's policy decide" — but they are the same file mechanics: one
//! complete line per record, owner-only, a size-bounded rotation that keeps exactly one previous
//! file, and a retention window judged by when a file was last written. The mechanics live here
//! once: a rotation fixed in one copy and not the other would be a trail that grows without bound
//! on one side only.
//!
//! Owner-only, like the endpoint file: which hosts a policy refused and which servers an agent
//! talked to are both things another user on the machine has no business reading. Windows inherits
//! the access control list of the configuration directory, for the same reason the endpoint file
//! does (see `mcp::endpoint`).

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde::de::DeserializeOwned;
use sevenshell_error::ShellError;

/// How large one trail may grow before it is rotated.
///
/// Eight mebibytes is roughly forty thousand lines of the records these files hold: weeks of
/// ordinary use, and small enough that reading the tail costs nothing. The rotated file is kept as
/// `.1` and nothing older: the retention setting decides whether even that one stays.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// The most lines one tail read may ask for.
///
/// The interface draws a panel, not a log viewer; a request for more than this is a caller that
/// should be paging, and answering it would put a whole file through IPC.
pub const MAX_TAIL_LINES: usize = 1_000;

/// One JSONL file, with the two limits this installation set for it.
#[derive(Debug, Clone)]
pub struct JsonlFile {
    path: PathBuf,
    retention_days: u32,
    max_bytes: u64,
}

impl JsonlFile {
    pub fn new(path: PathBuf, retention_days: u32) -> Self {
        Self {
            path,
            retention_days,
            max_bytes: MAX_FILE_BYTES,
        }
    }

    /// A file with a smaller rotation point, so rotation is testable without writing eight
    /// mebibytes.
    pub fn with_max_bytes(path: PathBuf, retention_days: u32, max_bytes: u64) -> Self {
        Self {
            path,
            retention_days,
            max_bytes,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn retention_days(&self) -> u32 {
        self.retention_days
    }

    /// Applies the retention rule once.
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
                Ok(()) => log::info!("a trail was older than its retention window and was dropped"),
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

    /// Appends one record as one complete line.
    ///
    /// The caller decides what a record is; serialization is the only place a record's content is
    /// ever looked at, so no other part of this module can accidentally log it.
    pub fn append_record<T: Serialize + ?Sized>(&self, record: &T) -> Result<(), ShellError> {
        let mut line = serde_json::to_vec(record)
            .map_err(|error| ShellError::store_io(format!("a JSONL record could not be serialized: {error}")))?;
        line.push(b'\n');
        self.append_line(&line)
    }

    fn append_line(&self, line: &[u8]) -> Result<(), ShellError> {
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

        file.write_all(line)
            .map_err(|error| ShellError::store_io(format!("cannot write {}: {error}", self.path.display())))?;
        // Flushed rather than left to the buffer: the next thing that happens may be the process
        // ending, and a trail that only existed in a buffer is no evidence at all.
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
                    "the previous trail {} could not be replaced: {error}",
                    rotated.display()
                )));
            }
        }
        std::fs::rename(&self.path, &rotated).map_err(|error| {
            ShellError::store_io(format!(
                "the trail {} could not be rotated: {error}",
                self.path.display()
            ))
        })?;
        Ok(())
    }
}

/// The last `limit` records, newest last, from one trail and its rotated predecessor.
///
/// A line that does not deserialize is skipped rather than failing the read: a file interrupted by a
/// power cut ends in half a line, and that half line must not hide the ninety-nine whole ones before
/// it.
pub fn read_tail<T: DeserializeOwned>(path: &Path, limit: usize) -> Vec<T> {
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
    records.split_off(start)
}

/// Every readable record in one file.
fn read_records<T: DeserializeOwned>(path: &Path) -> Vec<T> {
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
        .filter_map(|line| serde_json::from_str::<T>(line).ok())
        .collect()
}

pub(crate) fn rotated_path(path: &Path) -> PathBuf {
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

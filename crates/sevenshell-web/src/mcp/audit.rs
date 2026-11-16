//! The agent call trail for the browser host: one JSONL line per tool call per installation.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sevenshell_api::{AuditRecord, AuditSink};
use sevenshell_error::ShellError;

use crate::jsonl::{JsonlFile, MAX_FILE_BYTES};

pub const AUDIT_DIRECTORY: &str = "audit";
pub const AUDIT_FILE_NAME: &str = "mcp-audit.jsonl";

pub fn default_path() -> Result<PathBuf, ShellError> {
    let directory = sevenshell_platform::config_dir()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this machine"))?;
    Ok(path_in(&directory))
}

pub fn path_in(directory: &Path) -> PathBuf {
    directory.join(AUDIT_DIRECTORY).join(AUDIT_FILE_NAME)
}

#[derive(Debug, Clone)]
pub struct AuditLog {
    file: JsonlFile,
}

impl AuditLog {
    pub fn new(path: PathBuf, retention_days: u32) -> Self {
        Self {
            file: JsonlFile::with_max_bytes(path, retention_days, MAX_FILE_BYTES),
        }
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }

    pub fn prune(&self, now: SystemTime) {
        self.file.prune(now);
    }
}

impl AuditSink for AuditLog {
    fn record(&self, record: AuditRecord) {
        if let Err(error) = self.file.append_record(&record) {
            log::warn!("the agent audit line could not be written: {error}");
        }
    }
}

pub fn read_tail(path: &Path, limit: usize) -> Vec<AuditRecord> {
    crate::jsonl::read_tail(path, limit)
}

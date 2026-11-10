//! Running a container command on the far side of an SSH connection.
//!
//! An SSH exec channel takes a command line, not an argument vector, so this is the one place where
//! the argv is joined back into a string. Every argument is quoted on the way; nothing here may
//! interpolate unquoted text.
#![allow(dead_code)]

use std::sync::Arc;

use sevenshell_error::ShellError;
use sevenshell_ssh::SshSession;

use crate::runner::{LineStream, PtySession};

pub(crate) async fn run(_session: &Arc<SshSession>, _argv: &[String]) -> Result<String, ShellError> {
    Err(ShellError::app_unsupported("the remote container runner is not implemented yet"))
}

pub(crate) async fn run_stdin(
    _session: &Arc<SshSession>,
    _argv: &[String],
    _input: Vec<u8>,
) -> Result<String, ShellError> {
    Err(ShellError::app_unsupported("the remote container runner is not implemented yet"))
}

pub(crate) async fn stream(_session: &Arc<SshSession>, _argv: &[String]) -> Result<LineStream, ShellError> {
    Err(ShellError::app_unsupported("the remote container runner is not implemented yet"))
}

pub(crate) async fn open_pty(
    _session: &Arc<SshSession>,
    _argv: &[String],
    _columns: u32,
    _rows: u32,
) -> Result<PtySession, ShellError> {
    Err(ShellError::app_unsupported("the remote container terminal is not implemented yet"))
}

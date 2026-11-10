//! Where a container command runs.
//!
//! Two places are possible: a process on this machine, and an SSH exec channel on a host that has
//! the runtime installed. The difference stops at this file. Above it, an operation is an argument
//! vector; below it, one variant spawns that vector and the other quotes each argument into a
//! command string, because an SSH exec channel takes a command line and not an argv.

use std::sync::Arc;

use sevenshell_error::ShellError;
use sevenshell_ssh::SshSession;
use tokio::sync::mpsc;

/// A command that keeps producing lines after it was started: logs, stats, a pull or a push.
pub struct LineStream {
    pub(crate) lines: mpsc::Receiver<String>,
    stop: Box<dyn Fn() + Send + Sync>,
}

impl LineStream {
    /// The next line, or nothing when the command has finished.
    pub async fn next(&mut self) -> Option<String> {
        self.lines.recv().await
    }

    /// Stops the command and everything reading it. Calling it twice is allowed.
    pub fn stop(&self) {
        (self.stop)();
    }
}

/// A terminal inside a container: bytes in both directions, a window size, and a close.
///
/// The shape is deliberately the same as every other terminal this application carries, so the
/// session registry can forward it without learning where the bytes come from.
pub struct PtySession {
    pub(crate) output: mpsc::Receiver<Vec<u8>>,
    pub(crate) control: mpsc::Sender<PtyControl>,
}

// The variants are written by the local and SSH runners as they land; nothing constructs one yet.
#[allow(dead_code)]
pub(crate) enum PtyControl {
    Write(Vec<u8>),
    Resize { columns: u32, rows: u32 },
    Close,
}

impl PtySession {
    /// The next chunk of output, or nothing when the terminal has closed.
    pub async fn next_output(&mut self) -> Option<Vec<u8>> {
        self.output.recv().await
    }

    pub async fn write(&self, data: &[u8]) -> Result<(), ShellError> {
        self.control
            .send(PtyControl::Write(data.to_vec()))
            .await
            .map_err(|_| ShellError::app_unsupported("the container terminal has already closed"))
    }

    pub async fn resize(&self, columns: u32, rows: u32) -> Result<(), ShellError> {
        self.control
            .send(PtyControl::Resize { columns, rows })
            .await
            .map_err(|_| ShellError::app_unsupported("the container terminal has already closed"))
    }

    pub async fn close(&self) -> Result<(), ShellError> {
        let _ = self.control.send(PtyControl::Close).await;
        Ok(())
    }
}

/// Which side of the connection a container command runs on.
pub enum Runner {
    /// A process on this machine. The runtime's binary is found on `PATH`.
    Local,
    /// An exec channel on an SSH connection that is already open and verified.
    Ssh(Arc<SshSession>),
}

impl Runner {
    /// Runs a command and returns everything it wrote to stdout.
    pub async fn run(&self, _argv: &[String]) -> Result<String, ShellError> {
        Err(ShellError::app_unsupported(
            "the container runner is not implemented yet",
        ))
    }

    /// Runs a command with bytes on stdin, for `load`, `login` and packed uploads.
    pub async fn run_stdin(&self, _argv: &[String], _input: Vec<u8>) -> Result<String, ShellError> {
        Err(ShellError::app_unsupported(
            "the container runner is not implemented yet",
        ))
    }

    /// Runs a command whose output keeps arriving after it starts.
    pub async fn stream(&self, _argv: &[String]) -> Result<LineStream, ShellError> {
        Err(ShellError::app_unsupported(
            "the container runner is not implemented yet",
        ))
    }

    /// Runs a command attached to a pseudo terminal.
    pub async fn open_pty(&self, _argv: &[String], _columns: u32, _rows: u32) -> Result<PtySession, ShellError> {
        Err(ShellError::app_unsupported(
            "the container terminal is not implemented yet",
        ))
    }
}

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

use crate::{local, remote};

/// A command that keeps producing lines after it was started: logs, stats, a pull or a push.
pub struct LineStream {
    pub(crate) lines: mpsc::Receiver<String>,
    stop: Arc<dyn Fn() + Send + Sync>,
}

impl LineStream {
    /// A stream whose producer is already running. Crate-internal: the two runners are the only
    /// things that know how to attach a process to it.
    pub(crate) fn new(lines: mpsc::Receiver<String>, stop: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            lines,
            stop: Arc::new(stop),
        }
    }

    /// The next line, or nothing when the command has finished.
    pub async fn next(&mut self) -> Option<String> {
        self.lines.recv().await
    }

    /// A handle that stops the command without ending the stream that is being read here.
    ///
    /// Two things need it: a stop button that arrives on another task, and a registry that has to
    /// let go of a stream it cannot borrow any more.
    pub fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        Arc::clone(&self.stop)
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

// The local runner drives one directly; the remote runner is being written against the same shape.
#[allow(dead_code)]
pub(crate) enum PtyControl {
    Write(Vec<u8>),
    Resize { columns: u32, rows: u32 },
    Close,
}

impl PtySession {
    /// Takes the output half, once, leaving a closed channel behind.
    ///
    /// The terminal registry owns the one reader there is; a second reader would be a second
    /// interpretation of the same bytes. The placeholder is closed, so an accidental second call
    /// ends rather than waits forever.
    pub fn take_output(&mut self) -> mpsc::Receiver<Vec<u8>> {
        let (placeholder, closed) = mpsc::channel(1);
        drop(placeholder);
        std::mem::replace(&mut self.output, closed)
    }

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
///
/// Cloned rather than shared: a second user of the same connection — the file browser an exec tab
/// opened — has to run its commands there too, and both variants are cheap to copy (a unit, and an
/// `Arc` of the session that is already open).
#[derive(Clone)]
pub enum Runner {
    /// A process on this machine. The runtime's binary is found on `PATH`.
    Local,
    /// An exec channel on an SSH connection that is already open and verified.
    Ssh(Arc<SshSession>),
}

impl Runner {
    /// Runs a command and returns everything it wrote to stdout.
    pub async fn run(&self, argv: &[String]) -> Result<String, ShellError> {
        match self {
            Self::Local => local::run(argv).await,
            Self::Ssh(session) => remote::run(session, argv).await,
        }
    }

    /// Runs a command with bytes on stdin, for `load`, `login` and packed uploads.
    pub async fn run_stdin(&self, argv: &[String], input: Vec<u8>) -> Result<String, ShellError> {
        match self {
            Self::Local => local::run_stdin(argv, input).await,
            Self::Ssh(session) => remote::run_stdin(session, argv, input).await,
        }
    }

    /// Runs a command whose output keeps arriving after it starts.
    pub async fn stream(&self, argv: &[String]) -> Result<LineStream, ShellError> {
        match self {
            Self::Local => local::stream(argv).await,
            Self::Ssh(session) => remote::stream(session, argv).await,
        }
    }

    /// Runs a command attached to a pseudo terminal.
    pub async fn open_pty(&self, argv: &[String], columns: u32, rows: u32) -> Result<PtySession, ShellError> {
        match self {
            Self::Local => local::open_pty(argv, columns, rows).await,
            Self::Ssh(session) => remote::open_pty(session, argv, columns, rows).await,
        }
    }
}

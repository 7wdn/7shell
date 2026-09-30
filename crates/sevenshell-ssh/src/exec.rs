//! Running one command and collecting what it said.
//!
//! This is the non-interactive half of SSH, and it is deliberately not built on the terminal
//! channel: a terminal is a pty with a size, an echo setting and a shell that waits for input, and a
//! command that is supposed to finish wants none of that. The server runs the command directly, its
//! output arrives as two streams, and the exit status is the answer.
//!
//! What comes back is text, deliberately: the caller of this is a person reading the output of a
//! command they ran, or an interface showing it to them.

use std::time::Duration;

use russh::{Channel, ChannelMsg};
use sevenshell_error::ShellError;
use tokio::time::Instant;

use crate::SshSession;

/// How much output one command may produce before it is stopped.
///
/// Not a policy about what people may run: it is the difference between a command that printed too
/// much and a process that is holding a connection open until somebody notices.
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// What a command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    pub stdout: String,
    pub stderr: String,
    /// The exit status the server reported, when it reported one. A command killed by a signal, or
    /// a connection that ended first, leaves this empty — which is not the same as a failure, and
    /// is why it is an `Option` rather than a zero.
    pub exit_code: Option<u32>,
    /// How long it took, measured here.
    pub duration: Duration,
}

impl ExecOutcome {
    /// Whether the command reported success. A missing status is not success.
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

impl SshSession {
    /// Runs one command and waits for it to finish.
    ///
    /// The timeout covers the whole command, because a command that never returns is the failure
    /// this exists to prevent; the connection is left open afterwards, so one connection can run
    /// several commands in a row.
    pub async fn exec(&self, command: &str, timeout: Duration) -> Result<ExecOutcome, ShellError> {
        let started = Instant::now();
        let mut channel = self.open_command_channel().await?;

        channel
            .exec(true, command.to_owned())
            .await
            .map_err(|error| self.channel_failure(&format!("`{command}` could not be started"), &error))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code = None;

        let collect = async {
            // Read until the channel closes. An `Eof` arrives before the exit status often enough
            // that stopping there would lose the status, which is the part the caller needs.
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                    // Stream 1 is stderr; the others are nothing this talks to.
                    ChannelMsg::ExtendedData { data, ext: 1 } => stderr.extend_from_slice(&data),
                    ChannelMsg::ExitStatus { exit_status } => exit_code = Some(exit_status),
                    ChannelMsg::Close => break,
                    _ => {}
                }

                if stdout.len() + stderr.len() > MAX_OUTPUT_BYTES {
                    break;
                }
            }
        };

        match tokio::time::timeout(timeout, collect).await {
            Ok(()) => {}
            Err(_) => {
                // The channel is abandoned rather than closed politely: whatever is running on the
                // other side is not going to answer, and waiting for it is what the timeout is for.
                return Err(ShellError::ssh_timeout(&self.host, self.port, whole_seconds(timeout)));
            }
        }

        let too_much = stdout.len() + stderr.len() > MAX_OUTPUT_BYTES;

        Ok(ExecOutcome {
            stdout: as_text(stdout, too_much),
            stderr: as_text(stderr, too_much),
            exit_code,
            duration: started.elapsed(),
        })
    }

    /// Opens a channel for a command, which is a session channel with nothing requested on it yet.
    async fn open_command_channel(&self) -> Result<Channel<russh::client::Msg>, ShellError> {
        self.open_session_channel_for("a command").await
    }
}

/// Decodes output as text, replacing anything that is not.
///
/// A command's output is bytes, and some of those bytes are not text: a `cat` of a binary file, a
/// locale the server does not have. Showing replacement characters is better than refusing to show
/// the output at all, and it says the same thing the person would see in a terminal.
fn as_text(bytes: Vec<u8>, truncated: bool) -> String {
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated || bytes.len() > MAX_OUTPUT_BYTES {
        text.push_str("\n… output was cut off: the command produced more than this will keep\n");
    }
    text
}

/// Rounds a duration up to whole seconds, and never says zero.
///
/// A timeout of "0 seconds" reads like no timeout at all, which is the opposite of what happened.
pub(crate) fn whole_seconds(duration: Duration) -> u64 {
    duration.as_secs().max(1)
}

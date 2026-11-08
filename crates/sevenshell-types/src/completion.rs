//! One completion generator command, and what it said.
//!
//! The interface never runs a generator itself: a generator is an argv array declared by a command
//! specification, and it is executed on the machine the terminal is connected to. This is the shape
//! of that request and its answer, deliberately narrow — one argv, no shell string, a short
//! deadline and a small output ceiling.

use serde::{Deserialize, Serialize};

/// The shortest deadline a generator may ask for, in milliseconds.
pub const COMPLETION_TIMEOUT_MIN_MS: u64 = 200;
/// The longest deadline a generator may ask for, in milliseconds.
pub const COMPLETION_TIMEOUT_MAX_MS: u64 = 3_000;
/// The deadline used when the request does not name one.
pub const COMPLETION_DEFAULT_TIMEOUT_MS: u64 = 1_200;
/// The most output one stream may produce, in bytes.
pub const COMPLETION_MAX_OUTPUT_BYTES: usize = 256 * 1024;
/// The most arguments a generator may carry.
pub const COMPLETION_MAX_ARGS: usize = 32;
/// The longest single argument or working directory a generator may name.
pub const COMPLETION_MAX_TEXT_BYTES: usize = 4 * 1024;

/// A request to run one completion generator against an open terminal's connection.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionExecuteRequest {
    /// The terminal handle whose SSH connection runs the generator.
    pub handle: String,
    /// The program to run. Validated as a literal program name, never handed to a shell as code.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// The directory to run in, when the interface knows it. Absolute POSIX paths only.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Clamped to [`COMPLETION_TIMEOUT_MIN_MS`]..=[`COMPLETION_TIMEOUT_MAX_MS`].
    #[serde(default)]
    pub timeout_ms: u64,
    /// Clamped to at most [`COMPLETION_MAX_OUTPUT_BYTES`].
    #[serde(default)]
    pub max_output_bytes: usize,
}

/// What a generator said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionExecuteResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub timed_out: bool,
}

impl CompletionExecuteResult {
    /// A generator that ran out of time; there is no output to speak of.
    pub fn timed_out() -> Self {
        Self {
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            truncated: false,
            timed_out: true,
        }
    }
}

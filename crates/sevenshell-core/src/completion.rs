//! Running one completion generator on the machine a terminal is connected to.
//!
//! A generator is a program and an argv array declared by a command specification. It is not a
//! shell line: the program name is validated as a literal name, every argument is quoted, and the
//! policy classifier is asked about the line before anything runs. What comes back is bounded in
//! time and size, because this runs between keystrokes rather than because somebody asked for it.
//!
//! The connection is the terminal's own: a generator asks a machine that is already connected, never
//! opens a second one, and never writes into the interactive channel.

use std::time::Duration;

use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_policy::classify;
use sevenshell_types::{
    COMPLETION_DEFAULT_TIMEOUT_MS, COMPLETION_MAX_ARGS, COMPLETION_MAX_OUTPUT_BYTES, COMPLETION_MAX_TEXT_BYTES,
    COMPLETION_TIMEOUT_MAX_MS, COMPLETION_TIMEOUT_MIN_MS, CompletionExecuteRequest, CompletionExecuteResult, RiskLevel,
};

use crate::Core;
use crate::shell_quote;

impl Core {
    /// Runs one completion generator against the connection behind an open terminal.
    pub async fn run_completion(
        &self,
        request: CompletionExecuteRequest,
    ) -> Result<CompletionExecuteResult, ShellError> {
        let validated = match validate(request) {
            Ok(validated) => validated,
            Err(error) => {
                log::warn!("completion request refused: {:?} {:?}", error.code(), error.detail());
                return Err(error);
            }
        };
        let session = self.registry.session_of(&validated.handle).await?;

        let line = command_line(&validated.command, &validated.args, validated.cwd.as_deref());
        log::debug!("completion generator: {line}");
        let captured = match session
            .exec_capture(&line, validated.timeout, validated.max_output_bytes)
            .await
        {
            Ok(captured) => captured,
            Err(error) if error.code() == ErrorCode::SshTimeout => return Ok(CompletionExecuteResult::timed_out()),
            Err(error) => {
                log::warn!("completion generator failed: {error}");
                return Err(error);
            }
        };
        log::debug!(
            "completion generator answered: exit={:?} bytes={} truncated={}",
            captured.exit_code,
            captured.stdout.len(),
            captured.truncated
        );

        Ok(CompletionExecuteResult {
            exit_code: captured.exit_code.and_then(|code| i32::try_from(code).ok()),
            stdout: captured.stdout,
            stderr: captured.stderr,
            truncated: captured.truncated,
            timed_out: false,
        })
    }
}

/// The request, after the clamps and refusals that make it safe to run.
#[derive(Debug)]
struct Validated {
    handle: String,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    timeout: Duration,
    max_output_bytes: usize,
}

fn validate(request: CompletionExecuteRequest) -> Result<Validated, ShellError> {
    let handle = request.handle.trim().to_owned();
    if handle.is_empty() {
        return Err(ShellError::policy_denied("a completion generator without a terminal"));
    }

    let command = request.command.trim().to_owned();
    if !is_program_name(&command) {
        return Err(ShellError::policy_denied(
            "a completion generator that is not a program name",
        ));
    }

    if request.args.len() > COMPLETION_MAX_ARGS {
        return Err(ShellError::policy_denied(
            "a completion generator with too many arguments",
        ));
    }
    for argument in &request.args {
        if argument.len() > COMPLETION_MAX_TEXT_BYTES || argument.contains('\0') {
            return Err(ShellError::policy_denied(
                "a completion generator with an argument that cannot be run",
            ));
        }
    }

    if let Some(cwd) = request.cwd.as_deref()
        && !cwd.is_empty()
        && (!cwd.starts_with('/') || cwd.len() > COMPLETION_MAX_TEXT_BYTES || cwd.contains('\0'))
    {
        return Err(ShellError::policy_denied(
            "a completion generator outside an absolute directory",
        ));
    }

    // The classifier reads the line without quoting, so a destructive word cannot hide behind the
    // quotes the execution line will carry.
    let policy_line = std::iter::once(command.as_str())
        .chain(request.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    if classify(&policy_line).level != RiskLevel::Safe {
        return Err(ShellError::policy_denied(
            "a completion generator that is not read-only",
        ));
    }

    let timeout_ms = if request.timeout_ms == 0 {
        COMPLETION_DEFAULT_TIMEOUT_MS
    } else {
        request
            .timeout_ms
            .clamp(COMPLETION_TIMEOUT_MIN_MS, COMPLETION_TIMEOUT_MAX_MS)
    };
    let max_output_bytes = if request.max_output_bytes == 0 || request.max_output_bytes > COMPLETION_MAX_OUTPUT_BYTES {
        COMPLETION_MAX_OUTPUT_BYTES
    } else {
        request.max_output_bytes
    };
    let cwd = request.cwd.filter(|cwd| !cwd.is_empty());

    Ok(Validated {
        handle,
        command,
        args: request.args,
        cwd,
        timeout: Duration::from_millis(timeout_ms),
        max_output_bytes,
    })
}

/// The literal line the remote shell will run.
///
/// The program name and every argument are quoted separately: an argument is data, not syntax, and
/// one of them is allowed to contain a space or a semicolon. `cd` is short-circuited so a directory
/// that cannot be entered produces no candidate at all rather than candidates from the wrong place.
fn command_line(command: &str, args: &[String], cwd: Option<&str>) -> String {
    let mut line = shell_quote(command);
    for argument in args {
        line.push(' ');
        line.push_str(&shell_quote(argument));
    }
    match cwd {
        Some(cwd) => format!("cd {} 2>/dev/null && {line}", shell_quote(cwd)),
        None => line,
    }
}

/// A program name, not a shell fragment: letters, digits and the punctuation a file name uses.
fn is_program_name(command: &str) -> bool {
    !command.is_empty()
        && command.len() <= COMPLETION_MAX_TEXT_BYTES
        && !command.contains('\0')
        && command
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '+' | '-' | '/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(command: &str, args: &[&str]) -> CompletionExecuteRequest {
        CompletionExecuteRequest {
            handle: "terminal-1".to_owned(),
            command: command.to_owned(),
            args: args.iter().map(|argument| (*argument).to_owned()).collect(),
            cwd: None,
            timeout_ms: 0,
            max_output_bytes: 0,
        }
    }

    #[test]
    fn program_names_are_literal_names() {
        assert!(is_program_name("git"));
        assert!(is_program_name("systemctl"));
        assert!(is_program_name("/usr/bin/docker"));
        assert!(!is_program_name(""));
        assert!(!is_program_name("git branch"));
        assert!(!is_program_name("git;rm"));
        assert!(!is_program_name("sh -c"));
        assert!(!is_program_name("$(whoami)"));
    }

    #[test]
    fn every_argument_is_quoted_including_a_quote_of_its_own() {
        let line = command_line("git", &["branch".to_owned(), "a b'c".to_owned()], None);
        assert_eq!(line, "'git' 'branch' 'a b'\\''c'");
    }

    #[test]
    fn a_known_directory_is_entered_before_the_generator_runs() {
        let line = command_line("git", &["branch".to_owned()], Some("/srv/app data"));
        assert_eq!(line, "cd '/srv/app data' 2>/dev/null && 'git' 'branch'");
    }

    #[test]
    fn a_relative_directory_is_refused() {
        let mut request = req("git", &["branch"]);
        request.cwd = Some("srv/app".to_owned());
        assert_eq!(
            validate(request).unwrap_err().code(),
            ErrorCode::PolicyDenied,
            "a relative cwd has no meaning on the far side"
        );
    }

    #[test]
    fn a_timeout_is_clamped_to_the_window_a_keystroke_can_wait() {
        let mut request = req("git", &["branch"]);
        request.timeout_ms = 50_000;
        assert_eq!(
            validate(request).unwrap().timeout,
            Duration::from_millis(COMPLETION_TIMEOUT_MAX_MS)
        );

        let mut request = req("git", &["branch"]);
        request.timeout_ms = 1;
        assert_eq!(
            validate(request).unwrap().timeout,
            Duration::from_millis(COMPLETION_TIMEOUT_MIN_MS)
        );

        let request = req("git", &["branch"]);
        assert_eq!(
            validate(request).unwrap().timeout,
            Duration::from_millis(COMPLETION_DEFAULT_TIMEOUT_MS)
        );
    }

    #[test]
    fn the_output_ceiling_is_the_completion_one_unless_a_smaller_one_is_named() {
        let request = req("git", &["branch"]);
        assert_eq!(validate(request).unwrap().max_output_bytes, COMPLETION_MAX_OUTPUT_BYTES);

        let mut request = req("git", &["branch"]);
        request.max_output_bytes = 4096;
        assert_eq!(validate(request).unwrap().max_output_bytes, 4096);
    }

    #[test]
    fn a_generator_that_is_not_read_only_is_refused() {
        let mut request = req("rm", &["-rf", "/tmp/example"]);
        request.cwd = None;
        assert_eq!(validate(request).unwrap_err().code(), ErrorCode::PolicyDenied);
    }

    #[test]
    fn a_shell_string_is_not_a_program_name() {
        let request = req("sh", &["-c", "rm -rf /"]);
        // `sh` is a program name, but the classifier knows what a shell does with its string.
        assert_eq!(validate(request).unwrap_err().code(), ErrorCode::PolicyDenied);
    }

    #[test]
    fn too_many_arguments_are_refused() {
        let arguments: Vec<String> = (0..COMPLETION_MAX_ARGS + 1).map(|index| index.to_string()).collect();
        let request = CompletionExecuteRequest {
            handle: "terminal-1".to_owned(),
            command: "git".to_owned(),
            args: arguments,
            cwd: None,
            timeout_ms: 0,
            max_output_bytes: 0,
        };
        assert_eq!(validate(request).unwrap_err().code(), ErrorCode::PolicyDenied);
    }

    #[test]
    fn a_terminal_handle_is_required() {
        let mut request = req("git", &["branch"]);
        request.handle = "  ".to_owned();
        assert_eq!(validate(request).unwrap_err().code(), ErrorCode::PolicyDenied);
    }
}

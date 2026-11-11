//! Running a container command on the far side of an SSH connection.
//!
//! An SSH exec channel takes a command line, not an argument vector, so this is the one place where
//! the argv is joined back into a string. Every argument is quoted on the way; nothing here may
//! interpolate unquoted text.
//!
//! The connection arrives already open, already verified and already authenticated: the caller
//! decided which host to trust and which credential to offer before it got here, and nothing in this
//! module touches either.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_ssh::{ExecCapture, MAX_OUTPUT_BYTES, SshSession};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use crate::runner::{ByteStream, InputStream, LineStream, PtyControl, PtySession};

/// How long a command that is supposed to finish may take.
///
/// Generous, because the far side is doing the work — starting a stopped container, inspecting an
/// image — and a smaller budget would report a timeout for a command that was merely slow. The
/// streaming path deliberately has no budget: a log follows a container until it is stopped.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// The terminal type asked of the far side, the same one the rest of the application asks for: a
/// program inside the container looks up its terminfo entry by this name.
const PTY_TERMINAL_TYPE: &str = "xterm-256color";

/// How many lines wait for their reader before the command's own output has to wait.
const LINE_BACKLOG: usize = 64;

/// How many control messages a terminal accepts before the caller has to wait for one to be applied.
const CONTROL_BACKLOG: usize = 32;

/// How much output is read from a channel at once.
const READ_BUFFER_BYTES: usize = 16 * 1024;

/// A line longer than this is delivered as it is rather than held back.
///
/// A command that never writes a newline — a progress bar redrawing one line, a log of binary
/// rubbish — would otherwise be held in memory until the command exits, or forever.
const MAX_LINE_BYTES: usize = 256 * 1024;

/// How much of a command's own output is kept for an error message.
///
/// An error is not a place for a data dump: the reason a command failed is in the first line or two
/// of what it said, and the rest is what the person can read in the log.
const MAX_DETAIL_CHARS: usize = 400;

/// Runs a command and returns everything it wrote to standard output.
pub(crate) async fn run(session: &Arc<SshSession>, argv: &[String]) -> Result<String, ShellError> {
    let command = command_line(argv);
    let capture = session
        .exec_capture(&command, COMMAND_TIMEOUT, MAX_OUTPUT_BYTES)
        .await?;
    finished(argv, capture)
}

/// Runs a command with bytes on its standard input, for `load`, `login` and packed uploads.
pub(crate) async fn run_stdin(
    session: &Arc<SshSession>,
    argv: &[String],
    input: Vec<u8>,
) -> Result<String, ShellError> {
    let command = command_line(argv);
    let capture = session
        .exec_with_input(&command, &input, COMMAND_TIMEOUT, MAX_OUTPUT_BYTES)
        .await?;
    finished(argv, capture)
}

/// Runs a command whose standard input is still being produced, chunk by chunk.
///
/// There is deliberately no time budget here: the input is a file or a tree on this machine, and a
/// budget that is generous for a runtime call would cut off a large transfer half way. The stream
/// ends when the producer does, or when the command stops reading — whichever comes first.
pub(crate) async fn run_stream_stdin(
    session: &Arc<SshSession>,
    argv: &[String],
    input: &mut InputStream,
) -> Result<String, ShellError> {
    let command = command_line(argv);
    let (stdin, mut output) = session.open_exec_command(&command).await?.split();

    // The answer is drained on its own task while the input is written. A command that copies its
    // input to its output fills the channel's window on the first megabyte, and every write after
    // that waits for a reader that does not exist yet.
    let reading = tokio::spawn(async move {
        let mut stdout = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = output.next().await {
            if stdout.len() + chunk.len() > MAX_OUTPUT_BYTES {
                truncated = true;
                break;
            }
            stdout.extend_from_slice(&chunk);
        }
        (stdout, truncated, output.exit_code(), output.stderr())
    });

    let mut chunks = input.take_chunks();
    while let Some(chunk) = chunks.recv().await {
        if stdin.write(&chunk).await.is_err() {
            // The command is gone; its exit status is the explanation, not a broken channel.
            break;
        }
    }
    let _ = stdin.end().await;
    drop(stdin);

    let (stdout, truncated, exit_code, stderr) = match reading.await {
        Ok(captured) => captured,
        Err(error) => (Vec::new(), false, None, error.to_string()),
    };
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    if truncated {
        return Err(ShellError::container_command_failed(
            &operation(argv),
            format!("the output was larger than {MAX_OUTPUT_BYTES} bytes and was cut off"),
        ));
    }
    if exit_code != Some(0) {
        return Err(failed(argv, &stderr, &stdout, exit_code));
    }
    Ok(stdout)
}

/// Runs a command whose standard output is bytes rather than text.
///
/// The channel is handed over as it is; the command reads nothing, so its input is ended at once and
/// a `tar cf -` sees the end of input it never looks at.
pub(crate) async fn stream_bytes(session: &Arc<SshSession>, argv: &[String]) -> Result<ByteStream, ShellError> {
    let command = command_line(argv);
    let (stdin, output) = session.open_exec_command(&command).await?.split();
    let _ = stdin.end().await;
    drop(stdin);
    Ok(ByteStream::remote(output))
}

/// Runs a command whose output keeps arriving after it starts.
///
/// One line per item, with the carriage return of a terminal-style line ending removed. A stop
/// closes the command's channel, which is what ends the process on the far side; the same happens
/// when the returned [`LineStream`] is dropped, because the task holding the channel ends with it.
pub(crate) async fn stream(session: &Arc<SshSession>, argv: &[String]) -> Result<LineStream, ShellError> {
    let command = command_line(argv);
    let mut source = session.open_exec_stream(&command).await?;

    let (line_sender, lines) = mpsc::channel(LINE_BACKLOG);
    let (stop_sender, mut stop) = mpsc::channel::<()>(1);

    tokio::spawn(async move {
        let mut pending = Vec::new();
        let mut buffer = vec![0u8; READ_BUFFER_BYTES];
        loop {
            tokio::select! {
                // Biased, so a stop is not starved by a command that never stops talking.
                biased;
                stopped = stop.recv() => {
                    // `None` means the handle that could stop this was dropped, which is the same
                    // wish expressed by going away. Breaking drops the channel, and dropping the
                    // channel is what closes the command on the far side.
                    let _ = stopped;
                    break;
                }
                read = source.read(&mut buffer) => match read {
                    // No more bytes: the command has ended.
                    Ok(0) => break,
                    Ok(count) => {
                        pending.extend_from_slice(&buffer[..count]);
                        if !emit_lines(&mut pending, &line_sender).await {
                            return;
                        }
                    }
                    // A channel that broke is an end, not a line: the caller's stream ends and the
                    // connection stays usable for the next command.
                    Err(_) => break,
                },
            }
        }
        // A last line without a newline is still a line. Dropping it would hide the last word of
        // every failing command.
        if !pending.is_empty() {
            let _ = line_sender.send(text_line(&pending)).await;
        }
    });

    Ok(LineStream::new(
        lines,
        Box::new(move || {
            // On a channel of one, the first stop wins, a second finds it full, and neither ever
            // waits: a stop may be called from anywhere, including a window that is closing.
            let _ = stop_sender.try_send(());
        }),
    ))
}

/// Runs a command attached to a pseudo terminal of the given size.
pub(crate) async fn open_pty(
    session: &Arc<SshSession>,
    argv: &[String],
    columns: u32,
    rows: u32,
) -> Result<PtySession, ShellError> {
    let command = command_line(argv);
    let mut terminal = session
        .open_pty_exec(&command, PTY_TERMINAL_TYPE, columns, rows)
        .await?;

    // The output leaves before the control side starts, so neither side ever waits on the other's
    // lock: one task writes, one reader draws.
    let output = terminal.take_output();
    let (control, mut requests) = mpsc::channel(CONTROL_BACKLOG);

    tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            let applied = match request {
                PtyControl::Write(data) => terminal.write(&data).await,
                PtyControl::Resize { columns, rows } => terminal.resize(columns, rows).await,
                PtyControl::Close => break,
            };
            if applied.is_err() {
                // The terminal is gone; there is nowhere left to put the rest of the keystrokes.
                break;
            }
        }
        // The terminal is over either way — closed by the person, or dropped with the pane that was
        // drawing it — so the command on the far side ends too. A `docker exec` left running
        // because a tab closed is a process nobody would think to look for.
        let _ = terminal.close().await;
    });

    Ok(PtySession { output, control })
}

/// What a finished command left behind, or the envelope that says why it is not an answer.
fn finished(argv: &[String], capture: ExecCapture) -> Result<String, ShellError> {
    // Truncation is this client's ceiling, not the command's failure, and half of a listing parsed
    // as a whole one is worse than saying the listing was too large.
    if capture.truncated {
        return Err(ShellError::container_command_failed(
            &operation(argv),
            format!("the output was larger than {MAX_OUTPUT_BYTES} bytes and was cut off"),
        ));
    }
    // The exit status decides, not the presence of words on standard error: a runtime that warns
    // and carries on has still answered.
    if capture.exit_code != Some(0) {
        return Err(failed(argv, &capture.stderr, &capture.stdout, capture.exit_code));
    }
    Ok(capture.stdout)
}

/// The envelope for a command that ran and did not succeed.
///
/// The command's own words become the detail when it wrote any: "no such container" is what the
/// person can act on, and an exit status alone is not.
fn failed(argv: &[String], stderr: &str, stdout: &str, exit_code: Option<u32>) -> ShellError {
    let detail = detail_from(stderr)
        .or_else(|| detail_from(stdout))
        .unwrap_or_else(|| match exit_code {
            Some(code) => format!("the command exited with status {code}"),
            None => "the command ended without reporting an exit status".to_owned(),
        });
    ShellError::container_command_failed(&operation(argv), detail)
}

/// The first useful thing a command said, short enough to show.
fn detail_from(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= MAX_DETAIL_CHARS {
        return Some(trimmed.to_owned());
    }
    let mut capped: String = trimmed.chars().take(MAX_DETAIL_CHARS).collect();
    capped.push('…');
    Some(capped)
}

/// The words the interface shows for a command that failed.
///
/// The runtime and its verb, not the whole argument vector: an argument is usually an identifier
/// the person did not type, and an error message is not a place to dump a command line.
fn operation(argv: &[String]) -> String {
    match argv {
        [] => "the container command".to_owned(),
        [only] => only.clone(),
        [runtime, verb, ..] => format!("{runtime} {verb}"),
    }
}

/// Joins an argument vector into one POSIX shell command line.
///
/// This is the security boundary of the whole remote path. A container name, a namespace or an image
/// reference is text that came from elsewhere, and the shell on the far side runs whatever this
/// produces; so every argument is quoted, and nothing is concatenated into a word.
fn command_line(argv: &[String]) -> String {
    argv.iter()
        .map(|argument| quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One argument, quoted so that the shell hands it back as the same string.
///
/// Characters every POSIX shell reads as themselves are left bare, so a command stays readable in a
/// log. Everything else — empty strings, quoting, whitespace, anything not ASCII — is wrapped in
/// single quotes, which is the one quoting form with no escapes inside it. A single quote cannot
/// appear inside single quotes, so it is written as the end of one quoted run, a backslash-escaped
/// quote, and the start of the next.
fn quote(argument: &str) -> String {
    if argument.is_empty() {
        return "''".to_owned();
    }
    if argument.bytes().all(is_plain) {
        return argument.to_owned();
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('\'');
    quoted.push_str(&argument.replace('\'', "'\\''"));
    quoted.push('\'');
    quoted
}

/// Whether a byte means the same thing to every POSIX shell, so quoting it would only add noise.
fn is_plain(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._/:={}+".contains(&byte)
}

/// Sends every complete line in `pending`, leaving whatever follows the last newline.
///
/// `false` means the reader is gone, which is the caller's signal to stop reading.
async fn emit_lines(pending: &mut Vec<u8>, sender: &mpsc::Sender<String>) -> bool {
    loop {
        let split_at = match pending.iter().position(|byte| *byte == b'\n') {
            Some(index) => index + 1,
            None if pending.len() > MAX_LINE_BYTES => MAX_LINE_BYTES,
            None => return true,
        };
        let line: Vec<u8> = pending.drain(..split_at).collect();
        if sender.send(text_line(&line)).await.is_err() {
            return false;
        }
    }
}

/// One line of output as text: the newline goes, and so does the carriage return a program wrote
/// for a terminal that is no longer there.
fn text_line(raw: &[u8]) -> String {
    let without_newline = raw.strip_suffix(b"\n").unwrap_or(raw);
    let without_return = without_newline.strip_suffix(b"\r").unwrap_or(without_newline);
    String::from_utf8_lossy(without_return).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_error::ErrorCode;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn an_argument_of_plain_characters_is_left_alone() {
        assert_eq!(quote("docker"), "docker");
        assert_eq!(
            quote("registry.example:5000/team/app:v1.2.3"),
            "registry.example:5000/team/app:v1.2.3"
        );
        assert_eq!(quote("a=b{+}-_/"), "a=b{+}-_/");
    }

    #[test]
    fn an_empty_argument_is_quoted_so_it_stays_an_argument() {
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn every_metacharacter_arrives_quoted() {
        let cases = [
            ("web; rm -rf /", "'web; rm -rf /'"),
            ("$(id)", "'$(id)'"),
            ("`id`", "'`id`'"),
            ("web\nuname -a", "'web\nuname -a'"),
            ("it's", "'it'\\''s'"),
            ("my container", "'my container'"),
            ("a && b", "'a && b'"),
            ("a | b", "'a | b'"),
            ("a > /etc/passwd", "'a > /etc/passwd'"),
            ("\"quoted\"", "'\"quoted\"'"),
            ("a\\b", "'a\\b'"),
            ("", "''"),
        ];
        for (argument, expected) in cases {
            assert_eq!(quote(argument), expected, "quoting {argument:?}");
        }
    }

    #[test]
    fn the_command_line_quotes_each_argument_and_joins_them_with_spaces() {
        let line = command_line(&argv(&[
            "docker",
            "exec",
            "-it",
            "my container",
            "sh",
            "-c",
            "id; uname -a",
        ]));
        assert_eq!(line, "docker exec -it 'my container' sh -c 'id; uname -a'");
    }

    #[test]
    fn a_command_line_that_is_empty_stays_empty() {
        assert_eq!(command_line(&[]), "");
    }

    #[tokio::test]
    async fn a_line_split_across_two_chunks_is_delivered_once() {
        let (sender, mut lines) = mpsc::channel(8);
        let mut pending = Vec::new();

        pending.extend_from_slice(b"first-part");
        assert!(emit_lines(&mut pending, &sender).await);
        assert!(lines.try_recv().is_err(), "an unfinished line must not be delivered");

        pending.extend_from_slice(b"-second\r\nnext\n");
        assert!(emit_lines(&mut pending, &sender).await);
        assert_eq!(lines.recv().await.as_deref(), Some("first-part-second"));
        assert_eq!(lines.recv().await.as_deref(), Some("next"));
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn a_command_that_never_writes_a_newline_still_produces_lines() {
        let (sender, mut lines) = mpsc::channel(8);
        let mut pending = vec![b'x'; MAX_LINE_BYTES + 10];

        assert!(emit_lines(&mut pending, &sender).await);
        assert_eq!(lines.recv().await.map(|line| line.len()), Some(MAX_LINE_BYTES));
        assert_eq!(pending.len(), 10, "the rest is kept for the chunk that completes it");
    }

    #[test]
    fn a_line_loses_its_terminator_and_any_carriage_return() {
        assert_eq!(text_line(b"one\n"), "one");
        assert_eq!(text_line(b"one\r\n"), "one");
        assert_eq!(text_line(b"one"), "one");
        assert_eq!(text_line(b"\r\n"), "");
    }

    #[test]
    fn a_failure_names_the_runtime_and_its_verb_but_not_its_arguments() {
        let error = failed(
            &argv(&["docker", "inspect", "sha256:deadbeef"]),
            "no such object",
            "",
            Some(1),
        );
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        assert_eq!(
            error.message_params().get("operation").map(String::as_str),
            Some("docker inspect")
        );
        assert!(
            !error.to_string().contains("deadbeef"),
            "the arguments belong in the log, not in the message: {error}"
        );
        assert!(error.detail().unwrap_or_default().contains("no such object"));
    }

    #[test]
    fn a_failure_with_nothing_to_quote_names_the_exit_status() {
        let error = failed(&argv(&["docker", "ps"]), "", "", Some(17));
        assert!(
            error.detail().unwrap_or_default().contains("17"),
            "the status is the only thing left to say: {error:?}"
        );
    }

    #[test]
    fn a_failure_that_reported_no_status_says_so() {
        let error = failed(&argv(&["docker", "ps"]), "", "", None);
        assert!(error.detail().unwrap_or_default().contains("without reporting"));
    }

    #[test]
    fn a_failure_falls_back_to_what_the_command_printed() {
        // Not every runtime writes its refusal to standard error; `docker logs` on a container that
        // cannot start answers on standard output.
        let error = failed(
            &argv(&["docker", "logs", "web"]),
            "   ",
            "container is restarting\n",
            Some(1),
        );
        assert!(error.detail().unwrap_or_default().contains("container is restarting"));
    }

    #[test]
    fn a_detail_too_long_to_show_is_cut_with_an_ellipsis() {
        let long = "x".repeat(MAX_DETAIL_CHARS * 2);
        let detail = detail_from(&long).expect("a detail");
        assert_eq!(detail.chars().count(), MAX_DETAIL_CHARS + 1);
        assert!(detail.ends_with('…'));
    }

    #[test]
    fn a_failure_from_a_command_with_no_words_does_not_pretend_to_have_any() {
        assert_eq!(detail_from(" \n\t "), None);
    }

    #[test]
    fn a_failure_from_a_command_that_cannot_be_named_is_still_named() {
        let error = failed(&[], "", "", None);
        assert_eq!(
            error.message_params().get("operation").map(String::as_str),
            Some("the container command")
        );
    }

    #[test]
    fn a_command_that_succeeded_answers_with_standard_output_even_if_it_warned() {
        let capture = ExecCapture {
            stdout: "row\n".to_owned(),
            stderr: "warning: something it carried on without\n".to_owned(),
            exit_code: Some(0),
            duration: Duration::ZERO,
            truncated: false,
        };
        assert_eq!(finished(&argv(&["docker", "ps"]), capture).expect("a listing"), "row\n");
    }

    #[test]
    fn output_that_was_cut_off_is_an_error_rather_than_a_short_answer() {
        // Half of a listing parsed as a whole one is worse than saying the listing was too large.
        let capture = ExecCapture {
            stdout: "half a list".to_owned(),
            stderr: String::new(),
            exit_code: Some(0),
            duration: Duration::ZERO,
            truncated: true,
        };
        let error = finished(&argv(&["docker", "ps"]), capture).expect_err("a cut-off listing is not a listing");
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        assert!(error.detail().unwrap_or_default().contains("cut off"));
    }
}

/// The tests that need a real server, taken from the environment exactly like the SSH crate's own
/// live tests:
///
/// ```text
/// SEVENSHELL_TEST_HOST  SEVENSHELL_TEST_PORT  SEVENSHELL_TEST_USER  SEVENSHELL_TEST_PASSWORD
/// ```
///
/// They are ignored by default, so a normal `cargo test` needs nothing but a machine. Run them with
/// the lab from `deploy/lab/openssh`:
///
/// ```text
/// cargo test -p sevenshell-container -- --ignored --test-threads=1
/// ```
#[cfg(test)]
mod live_tests {
    use super::*;
    use sevenshell_error::ErrorCode;
    use sevenshell_ssh::{Credential, OpenRequest, Purpose, Route, Target, TerminalSize, probe_host_key};

    const OPEN_TIMEOUT: Duration = Duration::from_secs(20);

    /// The environment's server, or a note that the test cannot run.
    async fn lab() -> Option<Arc<SshSession>> {
        let (Some(host), Some(port), Some(user), Some(password)) = (
            std::env::var("SEVENSHELL_TEST_HOST").ok(),
            std::env::var("SEVENSHELL_TEST_PORT").ok(),
            std::env::var("SEVENSHELL_TEST_USER").ok(),
            std::env::var("SEVENSHELL_TEST_PASSWORD").ok(),
        ) else {
            eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
            return None;
        };
        let Ok(port) = port.parse::<u16>() else {
            eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
            return None;
        };

        let target = Target { host, port, user };
        let presented = probe_host_key(&target.host, target.port, OPEN_TIMEOUT)
            .await
            .expect("probe the host key");
        let request = OpenRequest {
            target,
            credential: Credential::Password(password),
            terminal_type: String::new(),
            keepalive_seconds: 0,
            forward_agent: false,
            timeout: OPEN_TIMEOUT,
            expected_host_key: presented,
            size: TerminalSize::default(),
            route: Route::Direct,
            // A carrier: this connection runs commands, it does not hold a shell.
            purpose: Purpose::Carrier,
        };
        let (events, _unused) = mpsc::channel(1);
        let connection = SshSession::open(request, events).await.expect("connect to the lab");
        Some(Arc::new(connection))
    }

    #[tokio::test]
    #[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
    async fn a_remote_command_returns_what_it_printed() {
        let Some(session) = lab().await else {
            return;
        };
        let output = run(
            &session,
            &["sh".to_owned(), "-c".to_owned(), "printf 'one\\ntwo\\n'".to_owned()],
        )
        .await
        .expect("a command that succeeds");
        assert_eq!(output, "one\ntwo\n");
    }

    #[tokio::test]
    #[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
    async fn a_name_with_a_semicolon_in_it_reaches_the_command_as_one_argument() {
        let Some(session) = lab().await else {
            return;
        };
        // The whole point of the quoting: the far side's shell must see one word, not `web` and a
        // second command.
        let output = run(
            &session,
            &["printf".to_owned(), "%s\n".to_owned(), "web; id".to_owned()],
        )
        .await
        .expect("a command that succeeds");
        assert_eq!(output, "web; id\n");
    }

    #[tokio::test]
    #[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
    async fn a_command_that_fails_is_reported_with_what_it_said() {
        let Some(session) = lab().await else {
            return;
        };
        let error = run(
            &session,
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "echo no-such-container 1>&2; exit 3".to_owned(),
            ],
        )
        .await
        .expect_err("a failing command must not look like an answer");

        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        assert!(error.detail().unwrap_or_default().contains("no-such-container"));
        assert_eq!(
            error.message_params().get("operation").map(String::as_str),
            Some("sh -c")
        );
    }

    #[tokio::test]
    #[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
    async fn bytes_are_fed_to_the_command_on_its_standard_input() {
        let Some(session) = lab().await else {
            return;
        };
        // A password on a pipe rather than in the command line, which is what `--password-stdin`
        // exists for.
        let output = run_stdin(
            &session,
            &["sh".to_owned(), "-c".to_owned(), "cat".to_owned()],
            b"a secret nobody should log\n".to_vec(),
        )
        .await
        .expect("a command that succeeds");
        assert_eq!(output, "a secret nobody should log\n");
    }

    #[tokio::test]
    #[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
    async fn a_streamed_input_crosses_the_channel_byte_for_byte() {
        let Some(session) = lab().await else {
            return;
        };
        // Larger than the channel's window: the writes and the reads have to happen at the same
        // time, which is what the split halves exist for.
        let payload: Vec<u8> = (0..4 * 1024 * 1024).map(|index| (index % 251) as u8).collect();
        let expected = payload.len();
        let mut input = InputStream::from_producer(move |sink| {
            std::io::Write::write_all(sink, &payload)
                .map_err(|error| ShellError::container_command_failed("the test input", error.to_string()))
        });
        let output = run_stream_stdin(
            &session,
            &["sh".to_owned(), "-c".to_owned(), "wc -c".to_owned()],
            &mut input,
        )
        .await
        .expect("a command that counts its input");
        assert_eq!(output.trim(), expected.to_string(), "not every byte arrived");
    }

    #[tokio::test]
    #[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
    async fn streamed_output_arrives_as_a_stream_and_ends_with_its_status() {
        let Some(session) = lab().await else {
            return;
        };
        let mut stream = stream_bytes(&session, &["sh".to_owned(), "-c".to_owned(), "seq 1 20000".to_owned()])
            .await
            .expect("a command that prints a lot");
        let mut received = Vec::new();
        let mut chunks = 0usize;
        while let Some(chunk) = stream.next().await {
            chunks += 1;
            received.extend_from_slice(&chunk);
        }
        assert!(chunks > 1, "the answer arrived in one chunk, so nothing was streamed");
        assert_eq!(stream.failure(), None, "a command that succeeded must not look failed");
        let text = String::from_utf8(received).expect("seq prints digits");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&"1"));
        assert_eq!(lines.last(), Some(&"20000"));
    }

    #[tokio::test]
    #[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
    async fn a_stream_delivers_each_line_and_then_ends_when_it_is_stopped() {
        let Some(session) = lab().await else {
            return;
        };
        // A command that prints and then keeps running: the shape of a log follow.
        let mut lines = stream(
            &session,
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf 'one\\ntwo\\n'; while :; do sleep 1; done".to_owned(),
            ],
        )
        .await
        .expect("a command that starts");

        let first = tokio::time::timeout(Duration::from_secs(15), lines.next())
            .await
            .expect("the first line arrives")
            .expect("the stream is still open");
        assert_eq!(first, "one");
        let second = tokio::time::timeout(Duration::from_secs(15), lines.next())
            .await
            .expect("the second line arrives")
            .expect("the stream is still open");
        assert_eq!(second, "two");

        lines.stop();
        let ended = tokio::time::timeout(Duration::from_secs(15), lines.next())
            .await
            .expect("a stopped stream ends instead of following forever");
        assert_eq!(ended, None);
    }

    #[tokio::test]
    #[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
    async fn a_terminal_reports_its_size_echoes_input_and_resizes() {
        let Some(session) = lab().await else {
            return;
        };
        let mut terminal = open_pty(&session, &["sh".to_owned()], 100, 40)
            .await
            .expect("a terminal on the far side");

        // A shell on a pty answers `stty size` with rows and columns: nothing else proves that the
        // size was requested rather than merely stored here.
        terminal.write(b"stty size\n").await.expect("a write");
        assert!(
            output_until(&mut terminal, "40 100").await.contains("40 100"),
            "the pty did not start at the size it was asked for"
        );

        terminal.resize(120, 50).await.expect("a resize");
        terminal.write(b"stty size\n").await.expect("a write");
        assert!(
            output_until(&mut terminal, "50 120").await.contains("50 120"),
            "the pty did not take the new size"
        );

        terminal.close().await.expect("a close");
        // The command ends with the channel: the output stream must end too, rather than waiting
        // for a shell that is no longer there.
        let ended = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if terminal.next_output().await.is_none() {
                    return;
                }
            }
        })
        .await;
        assert!(
            ended.is_ok(),
            "the terminal kept the output stream open after it was closed"
        );
        // Closing twice is what a tab does when it is closed twice.
        terminal.close().await.expect("a second close");
    }

    /// Everything the terminal says until it says the needle, or the wait runs out.
    async fn output_until(terminal: &mut PtySession, needle: &str) -> String {
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, terminal.next_output()).await {
                Ok(Some(data)) => {
                    seen.push_str(&String::from_utf8_lossy(&data));
                    if seen.contains(needle) {
                        return seen;
                    }
                }
                Ok(None) => panic!("the terminal ended before it said {needle}: {seen}"),
                Err(_) => panic!("the terminal never said {needle}: {seen}"),
            }
        }
    }
}

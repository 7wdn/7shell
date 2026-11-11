//! The program as a client actually launches it: arguments in, a session on the standard streams.
//!
//! The other suites test the library; this one runs the binary, because the parts that only exist in
//! `main` are the ones a client's configuration meets first — where the endpoint file comes from,
//! which stream usage and errors go to, and what the exit code means. A client cannot see function
//! calls, only these.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

use sevenshell_mcp::{DESKTOP_UNAVAILABLE, ENDPOINT_FILE_ENV, LATEST_PROTOCOL_VERSION};

const BINARY: &str = env!("CARGO_BIN_EXE_sshell-mcp");

fn bridge() -> Command {
    Command::new(BINARY)
}

/// Runs the binary with the given arguments, environment and input, and waits for it to exit.
fn run(arguments: &[&str], environment: &[(&str, &str)], input: &[String]) -> Output {
    let mut command = bridge();
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in environment {
        command.env(key, value);
    }

    let mut child = command.spawn().expect("the bridge starts");
    {
        let stdin = child.stdin.as_mut().expect("a stdin pipe");
        for line in input {
            stdin.write_all(line.as_bytes()).expect("the bridge reads");
            stdin.write_all(b"\n").expect("the bridge reads");
        }
    }
    // Closing the pipe is how a client ends the session; the bridge has to exit on its own.
    drop(child.stdin.take());

    child.wait_with_output().expect("the bridge exits")
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

/// Every line of stdout, required to be JSON: this is the assertion that keeps a stray log line out
/// of the protocol, because a client parses each line and stops at the first one that is not.
fn answers(output: &Output) -> Vec<Value> {
    stdout_text(output)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("stdout carried a line that is not JSON: {line:?} ({error})"))
        })
        .collect()
}

/// The answer carrying this id, wherever it landed in the stream.
fn answer_for(answers: &[Value], id: i64) -> &Value {
    answers
        .iter()
        .find(|answer| answer["id"] == json!(id))
        .unwrap_or_else(|| panic!("no answer for id {id}: {answers:?}"))
}

fn path_argument(directory: &Path, name: &str) -> String {
    directory.join(name).to_string_lossy().into_owned()
}

// -- Arguments ----------------------------------------------------------

#[test]
fn version_prints_the_build_and_exits_zero() {
    let output = bridge().arg("--version").output().expect("the bridge runs");

    assert!(output.status.success(), "exit status: {:?}", output.status.code());
    let stdout = stdout_text(&output);
    assert!(stdout.contains("sshell-mcp"), "{stdout}");
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")), "{stdout}");
    assert!(output.stderr.is_empty(), "a requested answer does not belong on stderr");
}

#[test]
fn help_names_the_endpoint_file_options_and_exits_zero() {
    let output = bridge().arg("--help").output().expect("the bridge runs");

    assert!(output.status.success());
    let stdout = stdout_text(&output);
    assert!(stdout.contains("--endpoint-file"), "{stdout}");
    assert!(stdout.contains(ENDPOINT_FILE_ENV), "{stdout}");
    assert!(stdout.contains("mcp-endpoint.json"), "{stdout}");
}

#[test]
fn an_unknown_argument_is_reported_on_stderr_with_exit_code_two() {
    let output = bridge().arg("--verbose").output().expect("the bridge runs");

    assert_eq!(output.status.code(), Some(2), "a misconfigured launch is not a session");
    assert!(
        stderr_text(&output).contains("--verbose"),
        "the refusal has to name the argument: {}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).is_empty(),
        "a refusal must not open the protocol stream: {}",
        stdout_text(&output)
    );
}

#[test]
fn an_endpoint_file_without_a_path_is_reported_on_stderr_with_exit_code_two() {
    let output = bridge().arg("--endpoint-file").output().expect("the bridge runs");

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr_text(&output).contains("--endpoint-file"));
    assert!(stdout_text(&output).is_empty());
}

// -- A session with the desktop missing --------------------------------

#[test]
fn a_session_without_a_desktop_answers_every_request_and_exits_zero() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let missing = path_argument(directory.path(), "mcp-endpoint.json");

    let input = vec![
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": LATEST_PROTOCOL_VERSION, "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }
        })
        .to_string(),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }).to_string(),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "sshell_sessions_list", "arguments": {} }
        })
        .to_string(),
    ];

    let output = run(&["--endpoint-file", &missing], &[], &input);

    assert!(
        output.status.success(),
        "the bridge did its job: exit status {:?}, stderr {}",
        output.status.code(),
        stderr_text(&output)
    );

    let answers = answers(&output);
    assert_eq!(answers.len(), 3, "one answer per request: {answers:?}");

    // Requests are answered concurrently, so the lines are not guaranteed to be in request order;
    // the id is what ties an answer to its request. Assuming order here made this test depend on
    // which task the runtime happened to schedule first.
    assert_eq!(answer_for(&answers, 1)["id"], json!(1));
    assert_eq!(
        answer_for(&answers, 2)["result"]["tools"]
            .as_array()
            .expect("a list of tools")
            .len(),
        10
    );
    let call = answer_for(&answers, 3);
    assert_eq!(
        call["error"]["code"],
        json!(DESKTOP_UNAVAILABLE),
        "a call without a desktop is the desktop's own failure, not a tool's: {call}"
    );

    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("mcp-endpoint.json"),
        "a person debugging a client needs to know which file was looked for: {stderr}"
    );
}

// -- Where the endpoint file comes from --------------------------------

#[test]
fn the_environment_variable_is_used_when_no_argument_names_a_file() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let from_environment = path_argument(directory.path(), "from-environment.json");

    let output = run(&[], &[(ENDPOINT_FILE_ENV, &from_environment)], &[]);

    assert!(output.status.success(), "stderr: {}", stderr_text(&output));
    assert!(
        stderr_text(&output).contains("from-environment.json"),
        "the bridge did not read the variable: {}",
        stderr_text(&output)
    );
}

#[test]
fn the_argument_wins_over_the_environment_variable() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let from_environment = path_argument(directory.path(), "from-environment.json");
    let from_argument = path_argument(directory.path(), "from-argument.json");

    let output = run(
        &["--endpoint-file", &from_argument],
        &[(ENDPOINT_FILE_ENV, &from_environment)],
        &[],
    );

    let stderr = stderr_text(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(
        stderr.contains("from-argument.json"),
        "the argument is the first choice: {stderr}"
    );
    assert!(
        !stderr.contains("from-environment.json"),
        "the environment was read even though the argument named a file: {stderr}"
    );
}

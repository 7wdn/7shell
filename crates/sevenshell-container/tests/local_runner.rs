//! The local runner against real processes.
//!
//! These use only programs every supported platform has, and never a container engine: what is
//! being tested is the plumbing between an argv and a pipe, not Docker. One of them failing while
//! the parser tests pass means the process side broke, which is exactly the split worth keeping.

use std::time::Duration;

use sevenshell_container::Runner;
use sevenshell_error::ErrorCode;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

/// One command line for the shell this machine has, as an argv for the runner.
fn shell(script: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        argv(&["cmd", "/c", script])
    }
    #[cfg(not(windows))]
    {
        argv(&["sh", "-c", script])
    }
}

#[tokio::test]
async fn a_harmless_program_comes_back_with_what_it_printed() {
    let output = Runner::Local
        .run(&shell("echo runner-ok"))
        .await
        .expect("echo should run");
    assert!(output.contains("runner-ok"), "{output:?}");
}

#[tokio::test]
async fn a_program_that_exits_non_zero_comes_back_as_a_command_failure() {
    let error = Runner::Local.run(&shell("exit 3")).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
}

#[tokio::test]
async fn the_reason_a_program_failed_travels_in_the_detail() {
    #[cfg(windows)]
    let command = shell("echo runner-refused 1>&2 & exit 7");
    #[cfg(not(windows))]
    let command = shell("echo runner-refused >&2; exit 7");

    let error = Runner::Local.run(&command).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
    let detail = error.detail().unwrap_or_default();
    assert!(detail.contains("runner-refused"), "detail was {detail:?}");
}

#[tokio::test]
async fn a_program_that_does_not_exist_is_reported_as_unavailable_and_not_as_a_generic_failure() {
    let error = Runner::Local
        .run(&argv(&["seven-shell-no-such-runtime-program"]))
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ContainerUnavailable);
}

#[tokio::test]
async fn standard_input_reaches_the_program_and_its_answer_comes_back() {
    // `sort` reads all of standard input and prints it ordered, which proves both directions
    // without assuming anything about the machine beyond its base utilities.
    #[cfg(windows)]
    let command = argv(&["cmd", "/c", "sort"]);
    #[cfg(not(windows))]
    let command = argv(&["sort"]);

    let output = Runner::Local
        .run_stdin(&command, b"beta\nalpha\n".to_vec())
        .await
        .expect("sort should run");
    let lines: Vec<&str> = output.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    assert_eq!(lines, vec!["alpha", "beta"]);
}

#[tokio::test]
async fn a_stream_yields_one_line_at_a_time_and_then_ends() {
    #[cfg(windows)]
    let command = shell("echo one & echo two");
    #[cfg(not(windows))]
    let command = shell("printf 'one\\ntwo\\n'");

    let mut stream = Runner::Local.stream(&command).await.expect("stream should start");
    let mut lines = Vec::new();
    loop {
        let line = tokio::time::timeout(Duration::from_secs(20), stream.next())
            .await
            .expect("the stream should keep answering");
        match line {
            Some(line) => lines.push(line.trim().to_owned()),
            None => break,
        }
    }
    assert_eq!(lines, vec!["one", "two"]);
}

#[tokio::test]
async fn stopping_a_stream_ends_it_and_its_reader() {
    // A program that loops by itself, not a shell that starts one: the stop kills the child, and a
    // grandchild would keep the pipe open and hide whether the stop worked.
    #[cfg(windows)]
    let command = argv(&["ping", "-n", "30", "127.0.0.1"]);
    #[cfg(not(windows))]
    let command = argv(&["sh", "-c", "while true; do echo tick; sleep 0.2; done"]);

    let mut stream = Runner::Local.stream(&command).await.expect("stream should start");
    let first = tokio::time::timeout(Duration::from_secs(20), stream.next())
        .await
        .expect("the stream should produce a line");
    assert!(first.is_some(), "the stream ended before producing anything");
    stream.stop();
    // The kill closes the pipe; the channel then reports the end instead of hanging.
    let ended = tokio::time::timeout(Duration::from_secs(20), async {
        while stream.next().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "a stopped stream kept producing output");
}

#[tokio::test]
async fn a_terminal_carries_bytes_in_both_directions() {
    #[cfg(windows)]
    let (command, enter) = (argv(&["cmd"]), "\r\n");
    #[cfg(not(windows))]
    let (command, enter) = (argv(&["sh"]), "\n");

    let mut terminal = Runner::Local
        .open_pty(&command, 100, 30)
        .await
        .expect("a terminal should open");
    terminal.resize(120, 40).await.expect("resize should be accepted");
    let line = format!("echo pty-ok{enter}");
    terminal.write(line.as_bytes()).await.expect("write should be accepted");

    let mut seen = Vec::new();
    let found = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(chunk) = terminal.next_output().await {
            seen.extend_from_slice(&chunk);
            if String::from_utf8_lossy(&seen).contains("pty-ok") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(
        found,
        "the terminal never echoed what was typed: {:?}",
        String::from_utf8_lossy(&seen)
    );

    terminal.close().await.expect("close should be accepted");
}

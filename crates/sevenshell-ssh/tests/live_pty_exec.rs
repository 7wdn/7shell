//! A command on a pseudo terminal, against a real server.
//!
//! The parts that can only be wrong on a real server are the ones this checks: that the pty is
//! created with the size that was asked for (the far side answers `stty size`), that keystrokes
//! reach the program, that a resize reaches it as well, that a command which exits nonzero reports
//! its own status, and that closing the channel ends the output rather than waiting for a shell that
//! is already gone.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 \
//! SEVENSHELL_TEST_USER=lab SEVENSHELL_TEST_PASSWORD=lab \
//!   cargo test -p sevenshell-ssh --test live_pty_exec -- --ignored --nocapture
//! ```
//!
//! The lab is `deploy/lab/openssh`; without those four variables every test here skips.

use std::sync::Arc;
use std::time::Duration;

use sevenshell_ssh::{
    Credential, OpenRequest, PtyExecChannel, Purpose, Route, SshSession, Target, TerminalSize, probe_host_key,
};

const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
/// Generous, because a lab container that is still starting up is slow rather than broken.
const WAIT: Duration = Duration::from_secs(20);

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// Connects to the lab, or says why the test cannot run.
async fn lab() -> Option<Arc<SshSession>> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
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
        // A carrier, not a terminal: the channels this test opens are the terminals.
        purpose: Purpose::Carrier,
    };
    let (events, _unused) = tokio::sync::mpsc::channel(1);
    let connection = SshSession::open(request, events).await.expect("connect to the lab");
    Some(Arc::new(connection))
}

/// Everything the terminal says until it says the needle, or the wait runs out.
async fn output_until(terminal: &mut PtyExecChannel, needle: &str) -> String {
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, terminal.next_output()).await {
            Ok(Some(data)) => {
                seen.push_str(&String::from_utf8_lossy(&data));
                if seen.contains(needle) {
                    return seen;
                }
            }
            Ok(None) => panic!("the terminal closed before it said {needle}: {seen}"),
            Err(_) => panic!("the terminal never said {needle}: {seen}"),
        }
    }
}

/// Reads until the channel ends, and returns what it said.
async fn output_until_end(terminal: &mut PtyExecChannel) -> String {
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, terminal.next_output()).await {
            Ok(Some(data)) => seen.push_str(&String::from_utf8_lossy(&data)),
            Ok(None) => return seen,
            Err(_) => panic!("the terminal never ended: {seen}"),
        }
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn a_pty_command_starts_at_the_size_it_was_given_and_exits_cleanly() {
    let Some(session) = lab().await else {
        return;
    };

    // `stty size` asks the terminal itself, so the answer is the size the far side really has. With
    // a pty, standard output is a terminal — which is the difference between this and `exec`.
    let mut terminal = session
        .open_pty_exec("sh -c 'printf hello; stty size'", "xterm-256color", 100, 40)
        .await
        .expect("a terminal on the far side");

    let seen = output_until_end(&mut terminal).await;
    assert!(seen.contains("hello"), "the command's output is missing: {seen:?}");
    assert!(
        seen.contains("40 100"),
        "the pty was not started at 40 rows by 100 columns: {seen:?}"
    );
    assert_eq!(terminal.exit_code(), Some(0), "the command exited cleanly");
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn a_pty_command_that_fails_reports_its_own_status() {
    let Some(session) = lab().await else {
        return;
    };

    let mut terminal = session
        .open_pty_exec("sh -c 'echo about-to-fail; exit 7'", "xterm-256color", 80, 24)
        .await
        .expect("a terminal on the far side");

    let seen = output_until_end(&mut terminal).await;
    assert!(
        seen.contains("about-to-fail"),
        "output before the exit is still output: {seen:?}"
    );
    assert_eq!(
        terminal.exit_code(),
        Some(7),
        "a terminal that ends is not the same as a command that succeeded"
    );
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn a_pty_terminal_echoes_what_is_written_to_it_and_takes_a_resize() {
    let Some(session) = lab().await else {
        return;
    };

    let mut terminal = session
        .open_pty_exec("sh", "xterm-256color", 80, 24)
        .await
        .expect("a shell on a terminal");

    // A pty echoes by default, so the marker coming back proves both directions without the shell
    // having to run anything.
    terminal.write(b"marker-through-the-pty\n").await.expect("a write");
    output_until(&mut terminal, "marker-through-the-pty").await;

    terminal.resize(120, 50).await.expect("a resize");
    terminal.write(b"stty size\n").await.expect("a write");
    assert!(
        output_until(&mut terminal, "50 120").await.contains("50 120"),
        "the program inside never saw the new size"
    );
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn closing_a_pty_terminal_ends_its_output_and_can_be_done_twice() {
    let Some(session) = lab().await else {
        return;
    };

    let mut terminal = session
        .open_pty_exec("sh", "xterm-256color", 80, 24)
        .await
        .expect("a shell on a terminal");

    terminal.close().await.expect("a close");
    terminal.close().await.expect("closing again is allowed");

    // The output stream has to end: the shell is gone, and a reader waiting for it would wait
    // forever.
    let ended = tokio::time::timeout(WAIT, output_until_end(&mut terminal)).await;
    assert!(ended.is_ok(), "the output stream outlived the terminal");

    // And writing to a closed terminal says so rather than disappearing.
    let error = terminal
        .write(b"too late\n")
        .await
        .expect_err("a closed terminal takes nothing");
    assert_eq!(error.code(), sevenshell_error::ErrorCode::SshChannelFailed);
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn bytes_written_to_a_command_reach_its_standard_input() {
    let Some(session) = lab().await else {
        return;
    };

    // The shape of `docker load` and `login --password-stdin`: the data is a stream, and the answer
    // is what the command printed after reading it.
    let capture = session
        .exec_with_input("sh -c 'cat'", b"first line\nsecond line\n", WAIT, 1024 * 1024)
        .await
        .expect("a command that reads its input");

    assert_eq!(capture.stdout, "first line\nsecond line\n");
    assert_eq!(capture.stderr, "");
    assert_eq!(capture.exit_code, Some(0));
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn a_command_that_answers_only_after_reading_its_input_still_ends() {
    let Some(session) = lab().await else {
        return;
    };

    // Reading until the input ends and only then answering is the order `docker login` uses. An
    // implementation that waited for output before closing the input would hang here.
    let outcome = tokio::time::timeout(
        WAIT,
        session.exec_with_input("sh -c 'wc -c'", b"1234567890", WAIT / 2, 1024 * 1024),
    )
    .await
    .expect("the command ends rather than waiting for input nobody will send")
    .expect("a command that reads its input");

    assert_eq!(outcome.stdout.trim(), "10");
    assert_eq!(outcome.exit_code, Some(0));
}

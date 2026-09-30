//! The exec channel against a real server.
//!
//! A non-interactive command has no pty and no shell to read: the server runs it and reports an
//! exit status. The parts that can only be wrong on a real server are the ones this checks — that
//! the command's output arrives, that its standard error arrives *separately*, that the exit code
//! is the command's own, and that a command which never exits still ends.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 \
//! SEVENSHELL_TEST_USER=lab SEVENSHELL_TEST_PASSWORD=lab \
//!   cargo test -p sevenshell-ssh --test live_exec -- --ignored --nocapture
//! ```
//!
//! The lab is `deploy/lab/openssh`; without those four variables every test here skips.

use std::sync::Arc;
use std::time::Duration;

use sevenshell_ssh::{Credential, OpenRequest, Purpose, Route, SshSession, Target, TerminalSize, probe_host_key};

const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
/// Generous, because a lab container that is still starting up is slow rather than broken.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

struct Lab {
    connection: Arc<SshSession>,
}

impl Lab {
    async fn exec(&self, command: &str, timeout: Duration) -> sevenshell_ssh::ExecOutcome {
        self.connection
            .exec(command, timeout)
            .await
            .expect("the command should run")
    }
}

/// Connects to the lab, or says why the test cannot run.
async fn lab() -> Option<Lab> {
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
    // A carrier, not a terminal: what this test drives has no pty.
    let presented = probe_host_key(&target.host, target.port, OPEN_TIMEOUT)
        .await
        .expect("probe the host key");

    let request = OpenRequest {
        target,
        credential: Credential::Password(password),
        terminal_type: String::new(),
        keepalive_seconds: 0,
        timeout: OPEN_TIMEOUT,
        expected_host_key: presented,
        size: TerminalSize::default(),
        route: Route::Direct,
        purpose: Purpose::Carrier,
    };

    let (events, _unused) = tokio::sync::mpsc::channel(1);
    let connection = SshSession::open(request, events).await.expect("connect to the lab");

    Some(Lab {
        connection: Arc::new(connection),
    })
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn standard_output_comes_back() {
    let Some(lab) = lab().await else {
        return;
    };

    let outcome = lab.exec("echo hello-from-seven-shell", RUN_TIMEOUT).await;

    assert_eq!(outcome.stdout, "hello-from-seven-shell\n");
    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.exit_code, Some(0));
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn standard_error_does_not_arrive_as_standard_output() {
    let Some(lab) = lab().await else {
        return;
    };

    // Both streams are written to, in an order that would be confusing if they were merged.
    let outcome = lab.exec("echo to-stdout; echo to-stderr 1>&2", RUN_TIMEOUT).await;

    assert_eq!(
        outcome.stdout, "to-stdout\n",
        "stdout must not contain what went to stderr"
    );
    assert_eq!(
        outcome.stderr, "to-stderr\n",
        "stderr must not contain what went to stdout"
    );
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn a_failing_command_reports_its_own_exit_code() {
    let Some(lab) = lab().await else {
        return;
    };

    let outcome = lab.exec("echo about-to-fail; exit 42", RUN_TIMEOUT).await;

    assert_eq!(outcome.exit_code, Some(42));
    // The output written before the exit is still the caller's to read: a batch across ten hosts
    // is only useful if the hosts that failed also say why.
    assert_eq!(outcome.stdout, "about-to-fail\n");
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn the_command_runs_without_a_terminal() {
    let Some(lab) = lab().await else {
        return;
    };

    // `test -t 1` asks whether standard output is a terminal. With a pty it is, and this is the
    // difference between a batch command and a terminal command: no prompt, no echo, no colour.
    let outcome = lab
        .exec("if test -t 1; then echo tty; else echo not-a-tty; fi", RUN_TIMEOUT)
        .await;

    assert_eq!(outcome.stdout, "not-a-tty\n");
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn a_command_that_never_finishes_is_cut_off_by_the_timeout() {
    let Some(lab) = lab().await else {
        return;
    };

    let started = std::time::Instant::now();
    let error = lab
        .connection
        .exec("sleep 600", Duration::from_secs(2))
        .await
        .expect_err("a command that never returns has to end in a timeout");

    // A timeout is its own answer, not an unknown exit code: the caller has to be able to say
    // "this one was still running when I gave up" rather than "this one failed".
    assert_eq!(error.code(), sevenshell_error::ErrorCode::SshTimeout);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the timeout has to end the wait, took {:?}",
        started.elapsed()
    );

    // And the connection is still usable: the timeout ends that command's channel, not the session.
    let after = lab.exec("echo still-here", RUN_TIMEOUT).await;
    assert_eq!(after.stdout, "still-here\n");
    assert_eq!(after.exit_code, Some(0));
}

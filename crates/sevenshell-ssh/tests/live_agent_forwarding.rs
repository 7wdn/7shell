//! Agent forwarding: the far side using a key that never leaves this machine.
//!
//! The proof is a chain, and every link is somewhere else: `ssh` running *on the server* is told to
//! use `$SSH_AUTH_SOCK` — which sshd creates only when forwarding was offered and accepted — and
//! that socket is bridged by this build to the agent here. The server then connects to itself and
//! authenticates with a key that exists only in this machine's agent.
//!
//! That last part is why the test is worth its length. Anything weaker — "the request was sent",
//! "the socket exists" — would pass with a bridge that drops every byte, and a bridge that drops
//! every byte is exactly the kind of thing that looks finished.
//!
//! Skips, loudly, without the lab or without an agent holding a key. The test arranges the server
//! side itself, with the password: the account accepts the agent's public key for the length of the
//! run, and the account is put back the way it was at the end.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 \
//! SEVENSHELL_TEST_USER=labuser SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-ssh --test live_agent_forwarding -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Duration;

use sevenshell_ssh::{
    ChannelEvent, Credential, OpenRequest, Purpose, Route, SshSession, Target, TerminalSize, agent_public_keys,
    probe_host_key,
};
use tokio::sync::mpsc;

const TIMEOUT: Duration = Duration::from_secs(20);

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

struct Coordinates {
    host: String,
    port: u16,
    user: String,
    password: String,
}

fn lab() -> Option<Coordinates> {
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

    Some(Coordinates {
        host,
        port,
        user,
        password,
    })
}

/// The reason to skip when this machine cannot lend an agent at all.
async fn agent_key() -> Option<String> {
    match agent_public_keys().await {
        Ok(keys) if !keys.is_empty() => keys.first().cloned(),
        Ok(_) => {
            eprintln!("the key agent holds no keys; skipping (ssh-add one first)");
            None
        }
        Err(error) => {
            eprintln!("no key agent on this machine ({error}); skipping");
            None
        }
    }
}

/// A connection with a terminal, which is the channel forwarding is offered on.
async fn terminal(coordinates: &Coordinates, forward_agent: bool) -> (Arc<SshSession>, mpsc::Receiver<ChannelEvent>) {
    let target = Target {
        host: coordinates.host.clone(),
        port: coordinates.port,
        user: coordinates.user.clone(),
    };
    let presented = probe_host_key(&target.host, target.port, TIMEOUT).await.expect("probe");

    let (events, output) = mpsc::channel(64);
    let request = OpenRequest {
        target,
        credential: Credential::Password(coordinates.password.clone()),
        terminal_type: "xterm-256color".to_owned(),
        keepalive_seconds: 0,
        forward_agent,
        timeout: TIMEOUT,
        expected_host_key: presented,
        size: TerminalSize::default(),
        route: Route::Direct,
        purpose: Purpose::Terminal,
    };

    let session = SshSession::open(request, events).await.expect("the terminal opens");
    (Arc::new(session), output)
}

/// Types a line into the terminal and reads until `marker` comes back.
async fn type_and_wait(
    session: &SshSession,
    events: &mut mpsc::Receiver<ChannelEvent>,
    line: &str,
    marker: &str,
) -> String {
    session
        .write(format!("{line}\r").as_bytes())
        .await
        .expect("the line is typed");

    let mut collected = String::new();
    let read = tokio::time::timeout(TIMEOUT, async {
        while let Some(event) = events.recv().await {
            match event {
                ChannelEvent::Data(bytes) => {
                    collected.push_str(&String::from_utf8_lossy(&bytes));
                    if collected.contains(marker) {
                        break;
                    }
                }
                ChannelEvent::Closed { .. } => break,
            }
        }
    })
    .await;

    if read.is_err() {
        eprintln!("timed out waiting for {marker}; collected: {collected}");
    }
    collected
}

/// The half that needs no agent: forwarding is what creates the socket, so a session that does not
/// offer it leaves the far side with nothing to use.
///
/// Worth its own case because it is the half that can be checked anywhere — and because it is the
/// *safe* half: it is the difference between lending the keys and not lending them.
#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_session_that_does_not_offer_the_agent_gets_no_socket_for_it() {
    let Some(coordinates) = lab() else {
        return;
    };

    let (plain, mut events) = terminal(&coordinates, false).await;
    // The marker is what the *shell* prints, not what was typed: the terminal echoes the command
    // back, so a marker that appears in the command itself would be found before the command ran.
    // `${SSH_AUTH_SOCK}` echoed literally is not `[]` — only the expansion makes the marker.
    let seen = type_and_wait(&plain, &mut events, "echo \"socket=[${SSH_AUTH_SOCK}]\"", "socket=[]").await;

    assert!(
        seen.contains("socket=[]"),
        "without forwarding there is no socket, got: {seen}"
    );
    plain.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs the SSH lab and a running key agent: see the module comment"]
async fn the_far_side_can_authenticate_with_a_key_that_never_left_this_machine() {
    let Some(coordinates) = lab() else {
        return;
    };
    let Some(key) = agent_key().await else {
        return;
    };

    // Arrange the server side: the account accepts the agent's key while this test runs.
    let (arranging, _output) = terminal(&coordinates, false).await;
    let allowed = arranging
        .exec(
            &format!(
                "mkdir -p ~/.ssh && chmod 700 ~/.ssh && printf '%s\\n' '{key}' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && echo arranged"
            ),
            TIMEOUT,
        )
        .await
        .expect("the password opens an exec channel");
    assert_eq!(allowed.stdout.trim(), "arranged");

    // Forwarding on: the server connects to itself and authenticates through the agent here.
    let (forwarding, mut events) = terminal(&coordinates, true).await;
    // `forwarded-"ok"` for the same reason: the quote is in the command and not in what the far
    // side prints, so the marker can only come from the remote command having run.
    let remote = format!(
        "ssh -p {} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o IdentitiesOnly=yes -o BatchMode=yes {}@127.0.0.1 echo forwarded-\"ok\"",
        coordinates.port, coordinates.user
    );
    let output = type_and_wait(&forwarding, &mut events, &remote, "forwarded-ok").await;

    assert!(
        output.contains("forwarded-ok"),
        "the server authenticated with this machine's agent; collected: {output}"
    );

    // And put the account back the way it was.
    let cleaned = forwarding
        .exec(
            &format!(
                "grep -v -F '{key}' ~/.ssh/authorized_keys > ~/.ssh/authorized_keys.tmp && mv ~/.ssh/authorized_keys.tmp ~/.ssh/authorized_keys && echo cleaned"
            ),
            TIMEOUT,
        )
        .await
        .expect("cleanup runs");
    assert_eq!(cleaned.stdout.trim(), "cleaned");

    forwarding.close().await.expect("close");
}

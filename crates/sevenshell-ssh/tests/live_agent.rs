//! Authentication with a key the machine's agent holds.
//!
//! What this proves that a unit test cannot: that a server accepts a signature made by an agent
//! which never handed the private key over. The key material exists only inside the agent, and the
//! test never learns it — it asks the agent for the *public* half, arranges for the account to
//! accept that key, and then connects with nothing but the agent's signature.
//!
//! It arranges the server side itself, with the password it is given: the lab account is a fixture,
//! and a test that needed somebody to have prepared it by hand would be a test that runs nowhere.
//!
//! ```text
//! # a running agent that holds at least one key, then:
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 \
//! SEVENSHELL_TEST_USER=labuser SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-ssh --test live_agent -- --ignored --nocapture
//! ```
//!
//! Skips when the lab is not configured, and when no agent is reachable or it holds no keys —
//! both are conditions of the machine, not failures of the code.

use std::sync::Arc;
use std::time::Duration;

use sevenshell_ssh::{
    Credential, ExecOutcome, OpenRequest, Purpose, Route, SshSession, Target, TerminalSize, agent_public_keys,
    probe_host_key,
};

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

/// Opens a connection to the lab with the given credential.
async fn connect(
    coordinates: &Coordinates,
    credential: Credential,
) -> Result<Arc<SshSession>, sevenshell_error::ShellError> {
    let target = Target {
        host: coordinates.host.clone(),
        port: coordinates.port,
        user: coordinates.user.clone(),
    };
    let presented = probe_host_key(&target.host, target.port, TIMEOUT).await.expect("probe");

    let request = OpenRequest {
        target,
        credential,
        terminal_type: String::new(),
        keepalive_seconds: 0,
        timeout: TIMEOUT,
        expected_host_key: presented,
        size: TerminalSize::default(),
        route: Route::Direct,
        purpose: Purpose::Carrier,
    };

    let (events, _unused) = tokio::sync::mpsc::channel(1);
    SshSession::open(request, events).await.map(Arc::new)
}

/// One command over an open connection, with its answer.
async fn exec(connection: &SshSession, command: &str) -> ExecOutcome {
    connection.exec(command, TIMEOUT).await.expect("the command should run")
}

#[tokio::test]
#[ignore = "needs the SSH lab and a running key agent: see the module comment"]
async fn a_key_the_agent_holds_authenticates_where_the_key_was_never_given_to_us() {
    let Some(coordinates) = lab() else {
        return;
    };

    let keys = match agent_public_keys().await {
        Ok(keys) => keys,
        Err(error) => {
            eprintln!("no key agent on this machine ({error}); skipping");
            return;
        }
    };
    let Some(key) = keys.first() else {
        eprintln!("the key agent holds no keys; skipping (ssh-add one first)");
        return;
    };

    // The server side, arranged with the password: this account may accept the agent's key.
    let password_connection = connect(&coordinates, Credential::Password(coordinates.password.clone()))
        .await
        .expect("the password should still open the session");
    let allowed = exec(
        &password_connection,
        // Single quotes: an authorized_keys line is a key type, a key and a comment, and none of
        // those three can contain a quote — the format has no way to escape one.
        &format!(
            "mkdir -p ~/.ssh && chmod 700 ~/.ssh && printf '%s\\n' '{key}' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && echo arranged"
        ),
    )
    .await;
    assert_eq!(allowed.stdout.trim(), "arranged");
    drop(password_connection);

    // And now with nothing but the agent: no password, no key file, nothing this test knows.
    let agent_connection = connect(&coordinates, Credential::Agent).await;
    let connection = match agent_connection {
        Ok(connection) => connection,
        Err(error) => panic!("the agent's key was refused: {error:?}"),
    };

    let whoami = exec(&connection, "id -un").await;
    assert_eq!(
        whoami.stdout.trim(),
        coordinates.user,
        "the shell belongs to the account asked for"
    );
    assert_eq!(whoami.exit_code, Some(0));

    // Put the account back the way it was, so the next run starts where this one did.
    let cleaned = exec(
        &connection,
        &format!("grep -v -F '{key}' ~/.ssh/authorized_keys > ~/.ssh/authorized_keys.tmp && mv ~/.ssh/authorized_keys.tmp ~/.ssh/authorized_keys && echo cleaned"),
    )
    .await;
    assert_eq!(cleaned.stdout.trim(), "cleaned");
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn an_agent_that_cannot_be_reached_is_reported_as_that() {
    let Some(coordinates) = lab() else {
        return;
    };

    // The whole point of this case: the failure names the agent rather than looking like a
    // rejected password. It runs wherever the test runs, because a wrong socket is a socket.
    let previous = std::env::var_os("SSH_AUTH_SOCK");
    // SAFETY: this test sets and restores one environment variable around its own connection. It
    // is the only test in this file that touches the environment, and the live suite runs one at a
    // time.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", r"\\.\pipe\seven-shell-agent-that-does-not-exist") };

    let error = match connect(&coordinates, Credential::Agent).await {
        Ok(_) => {
            if let Some(previous) = &previous {
                unsafe { std::env::set_var("SSH_AUTH_SOCK", previous) };
            } else {
                unsafe { std::env::remove_var("SSH_AUTH_SOCK") };
            }
            panic!("an agent that does not exist cannot have answered");
        }
        Err(error) => error,
    };

    if let Some(previous) = &previous {
        unsafe { std::env::set_var("SSH_AUTH_SOCK", previous) };
    } else {
        unsafe { std::env::remove_var("SSH_AUTH_SOCK") };
    }

    assert_eq!(error.code(), sevenshell_error::ErrorCode::SshAuthFailed);
    let detail = error.detail().unwrap_or_default();
    assert!(
        detail.contains("agent") || detail.contains("could not be reached"),
        "the failure has to name the agent, got: {detail}"
    );
}

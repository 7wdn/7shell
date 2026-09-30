//! A connection to a real server, from the vault to the first bytes of output.
//!
//! Ignored by default: it needs a running sshd. `deploy/lab/` has one and `make lab-up` starts it.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 SEVENSHELL_TEST_USER=labuser \
//!   SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-core --test live_connection -- --ignored
//! ```
//!
//! This is the test that proves the pieces fit: a vault that stores a session and its password, a
//! host key that has to be trusted before anything is sent, a terminal that opens, and output that
//! comes back through the registry.

use std::sync::Arc;
use std::time::Duration;

use sevenshell_core::Core;
use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_session::TerminalEvent;
use sevenshell_types::{AuthMethod, KnownHostEntry, OpenOutcome, Session, SessionAuth, TerminalSettings};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;
use tokio::time::timeout;

struct Lab {
    /// Owned so the vault file outlives the test.
    _directory: TempDir,
    vault: Arc<Vault>,
    core: Arc<Core>,
    session_id: String,
    events: mpsc::Receiver<sevenshell_core::CoreEvent>,
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// The lab's coordinates, or a message saying why this test cannot run.
fn lab_credentials() -> Option<(String, u16, String, String)> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return None;
    };

    match port.parse() {
        Ok(port) => Some((host, port, user, password)),
        Err(_) => {
            eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
            None
        }
    }
}

/// A vault holding one session for the lab, with its password stored.
///
/// `password_override` lets a test store a password the server will refuse, so the authentication
/// failure path can be exercised without a second server.
fn lab_with_session(password_override: Option<&str>) -> Option<Lab> {
    let (host, port, user, password) = lab_credentials()?;
    let password = password_override.map(str::to_owned).unwrap_or(password);

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    let session = Session {
        id: "lab-session".to_owned(),
        name: "lab".to_owned(),
        group_id: None,
        protocol: sevenshell_types::Protocol::Ssh,
        host,
        port,
        user,
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        updated_at: 0,
    };
    vault
        .save_session(session, Some(SessionSecret::Password(password)))
        .expect("save session");

    let (core, events) = Core::new(Arc::clone(&vault));
    Some(Lab {
        _directory: directory,
        vault,
        core,
        session_id: "lab-session".to_owned(),
        events,
    })
}

/// Drains events for one terminal until the marker appears, so assertions are about output rather
/// than about how long the machine happened to take.
async fn collect_until(events: &mut mpsc::Receiver<sevenshell_core::CoreEvent>, handle: &str, marker: &str) -> String {
    let mut collected = String::new();

    let finished = timeout(Duration::from_secs(20), async {
        while let Some(event) = events.recv().await {
            if event.handle != handle {
                continue;
            }
            match event.event {
                TerminalEvent::Data(bytes) => {
                    collected.push_str(&String::from_utf8_lossy(&bytes));
                    if collected.contains(marker) {
                        break;
                    }
                }
                TerminalEvent::Closed { reason } => {
                    eprintln!("the server closed the channel: {reason:?}");
                    break;
                }
            }
        }
    })
    .await;

    if finished.is_err() {
        eprintln!("timed out waiting for {marker}");
    }
    collected
}

async fn expect_error(open: Result<OpenOutcome, ShellError>) -> ShellError {
    match open {
        Ok(outcome) => panic!("expected a failure, got {outcome:?}"),
        Err(error) => error,
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_new_host_key_is_a_decision_and_then_the_session_opens() {
    let Some(mut lab) = lab_with_session(None) else {
        return;
    };

    // The first attempt reaches a decision. The outcome itself is the proof that nothing was sent:
    // the core returns it instead of connecting, and the probe it runs sends no credential.
    let key = match lab.core.open_terminal(&lab.session_id).await.expect("probe") {
        OpenOutcome::HostKeyUnknown { key } => key,
        other => panic!("expected an unknown host key on the first attempt, got {other:?}"),
    };
    assert!(
        key.fingerprint.starts_with("SHA256:"),
        "the fingerprint reads like OpenSSH prints it: {}",
        key.fingerprint
    );
    assert_eq!(key.key_type, "ssh-ed25519", "the lab server presents an ed25519 key");

    // Trusting is what a person does after comparing that fingerprint.
    lab.core.trust_host_key(&key).expect("trust");

    // Now it opens, and asking a second time needs no decision.
    let handle = match lab.core.open_terminal(&lab.session_id).await.expect("open") {
        OpenOutcome::Opened { handle, .. } => handle,
        other => panic!("expected the session to open once the key is trusted, got {other:?}"),
    };

    let marker = "7shell-live-marker";
    lab.core
        .write(&handle, format!("echo {marker}\r\n").as_bytes())
        .await
        .expect("write the command");

    let output = collect_until(&mut lab.events, &handle, marker).await;
    assert!(
        output.contains(marker),
        "the command's output never arrived; collected: {output}"
    );

    lab.core.close_terminal(&handle).await.expect("close");
    assert!(
        lab.core.open_terminals().await.is_empty(),
        "a closed terminal is not still open"
    );
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_changed_host_key_stops_the_connection_before_anything_is_sent() {
    let Some(lab) = lab_with_session(None) else {
        return;
    };

    let (host, port) = (
        lab.vault.sessions().expect("sessions")[0].host.clone(),
        lab.vault.sessions().expect("sessions")[0].port,
    );

    // Pretend a different key was trusted for this host earlier.
    lab.vault
        .trust_host(KnownHostEntry {
            host,
            port,
            key_type: "ssh-ed25519".to_owned(),
            public_key: "AAAAB3NzaC1lZDI1NTE5AAAAIthisisnotthekeytheserverhas".to_owned(),
            fingerprint: "SHA256:notthekeytheserverhas".to_owned(),
            added_at: 0,
        })
        .expect("trust a stale key");

    match lab.core.open_terminal(&lab.session_id).await.expect("probe") {
        OpenOutcome::HostKeyMismatch { key, trusted } => {
            assert_ne!(
                key.public_key, trusted.public_key,
                "the two keys are shown side by side so a person can compare"
            );
            assert_eq!(trusted.fingerprint, "SHA256:notthekeytheserverhas");
        }
        other => panic!("a changed host key must stop the connection, got {other:?}"),
    }

    // The server never saw an authentication attempt. That is the point of the ordering, and it can
    // be confirmed from outside the test: `docker logs sevenshell-lab-sshd | grep -i password`.
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_rejected_password_says_so_and_admits_that_it_was_sent() {
    let Some(lab) = lab_with_session(Some("this is not the lab password")) else {
        return;
    };

    // Trust the real key first, so the failure under test is authentication and not the key.
    let key = match lab.core.open_terminal(&lab.session_id).await.expect("probe") {
        OpenOutcome::HostKeyUnknown { key } => key,
        other => panic!("expected an unknown host key first, got {other:?}"),
    };
    lab.core.trust_host_key(&key).expect("trust");

    let error = expect_error(lab.core.open_terminal(&lab.session_id).await).await;

    assert_eq!(error.code(), ErrorCode::SshAuthFailed);
    assert_eq!(
        error.outcome(),
        sevenshell_error::OperationOutcome::Sent,
        "a rejected password was sent, and the envelope says so"
    );
    assert!(
        !error.to_string().contains("this is not the lab password"),
        "the password is not in the error"
    );
}

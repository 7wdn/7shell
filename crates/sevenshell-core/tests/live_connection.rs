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
use sevenshell_types::{
    AuthMethod, ForwardMode, KnownHostEntry, LocalForward, OpenOutcome, Session, SessionAuth, TerminalSettings,
};
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

impl Lab {
    /// Opens the session, trusting whatever host key it is asked about, the way a person who has
    /// compared the fingerprint would.
    async fn open_trusting_everything(&self) -> String {
        for _ in 0..4 {
            match self.core.open_terminal(&self.session_id).await.expect("open") {
                OpenOutcome::Opened { handle, .. } => return handle,
                OpenOutcome::HostKeyUnknown { key } => self.core.trust_host_key(&key).expect("trust"),
                other => panic!("expected the session to open, got {other:?}"),
            }
        }
        panic!("the route never finished asking");
    }
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
        host: host.clone(),
        port,
        user: user.clone(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    };
    // A second saved session pointing at the same server, which is what a jump host is: the lab
    // container is both ends of the chain, and the plumbing is the same either way.
    let mut hop = session.clone();
    hop.id = "lab-hop".to_owned();
    hop.name = "lab (hop)".to_owned();

    vault
        .save_session(session, Some(SessionSecret::Password(password.clone())))
        .expect("save session");
    vault
        .save_session(hop, Some(SessionSecret::Password(password)))
        .expect("save hop");

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

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_terminal_works_through_a_jump_host() {
    let Some(mut lab) = lab_with_session(None) else {
        return;
    };

    let mut through = lab.vault.sessions().expect("sessions")[0].clone();
    through.id = "lab-through".to_owned();
    through.name = "lab (through the hop)".to_owned();
    through.jump = vec!["lab-hop".to_owned()];
    lab.vault
        .save_session(
            through,
            Some(SessionSecret::Password(
                environment("SEVENSHELL_TEST_PASSWORD").unwrap_or_default(),
            )),
        )
        .expect("save");

    lab.session_id = "lab-through".to_owned();
    let handle = lab.open_trusting_everything().await;

    let marker = "7shell-jump-marker";
    lab.core
        .write(
            &handle,
            format!(
                "echo {marker}
"
            )
            .as_bytes(),
        )
        .await
        .expect("write");

    let output = collect_until(&mut lab.events, &handle, marker).await;
    assert!(
        output.contains(marker),
        "the command ran through the hop; collected: {output}"
    );

    lab.core.close_terminal(&handle).await.expect("close");
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_saved_forward_answers_a_local_port_from_the_far_side() {
    let Some(mut lab) = lab_with_session(None) else {
        return;
    };

    let mut forwarding = lab.vault.sessions().expect("sessions")[0].clone();
    forwarding.id = "lab-forwarding".to_owned();
    forwarding.forwards = vec![LocalForward {
        id: "f1".to_owned(),
        // Port zero: the system picks one, and the outcome says which.
        bind_host: "127.0.0.1".to_owned(),
        bind_port: 0,
        mode: ForwardMode::Local,
        target_host: "127.0.0.1".to_owned(),
        target_port: lab.vault.sessions().expect("sessions")[0].port,
    }];
    lab.vault
        .save_session(
            forwarding,
            Some(SessionSecret::Password(
                environment("SEVENSHELL_TEST_PASSWORD").unwrap_or_default(),
            )),
        )
        .expect("save");

    lab.session_id = "lab-forwarding".to_owned();

    let mut bound = None;
    for _ in 0..4 {
        match lab.core.open_terminal(&lab.session_id).await.expect("open") {
            OpenOutcome::Opened {
                handle,
                tunnels,
                tunnel_failures,
                ..
            } => {
                assert!(tunnel_failures.is_empty(), "the forward came up: {tunnel_failures:?}");
                assert_eq!(tunnels.len(), 1);
                bound = Some((handle, tunnels[0].bind_port));
                break;
            }
            OpenOutcome::HostKeyUnknown { key } => lab.core.trust_host_key(&key).expect("trust"),
            other => panic!("expected the session to open, got {other:?}"),
        }
    }

    let (handle, port) = bound.expect("the forward is up");

    // The forwarded port answers with the far side's SSH banner, which is the proof that bytes
    // went out through the connection and came back.
    let banner = tokio::time::timeout(Duration::from_secs(10), async {
        use tokio::io::AsyncReadExt as _;
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect to the tunnel");
        let mut buffer = [0_u8; 64];
        let read = stream.read(&mut buffer).await.expect("read the banner");
        String::from_utf8_lossy(&buffer[..read]).into_owned()
    })
    .await
    .expect("the tunnel answered in time");

    assert!(
        banner.starts_with("SSH-"),
        "the tunnel carried an SSH banner, got: {banner:?}"
    );

    let tunnels = lab.core.tunnels().await;
    assert_eq!(tunnels.len(), 1);
    assert!(tunnels[0].connections >= 1, "the connection was counted");

    lab.core.close_terminal(&handle).await.expect("close");
    assert!(
        lab.core.tunnels().await.is_empty(),
        "closing the terminal closes its forwards"
    );
}

/// A dynamic forward is a SOCKS5 proxy on this machine whose destinations are reached from the
/// far side.
///
/// The proof is the same shape as the local forward's and deliberately stronger: the test speaks
/// SOCKS5 to the port it just opened, asks for the lab's *own sshd port*, and reads the SSH banner
/// back — which can only have come through the connection, because the address it asked for means
/// nothing to anything but the far side. A domain name is exercised as well, since passing one on
/// unread is the whole reason a tunnel resolves names where it lands.
#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_dynamic_forward_answers_what_a_client_asks_for() {
    let Some(mut lab) = lab_with_session(None) else {
        return;
    };

    let mut forwarding = lab.vault.sessions().expect("sessions")[0].clone();
    forwarding.id = "lab-socks".to_owned();
    forwarding.forwards = vec![LocalForward {
        id: "socks".to_owned(),
        bind_host: "127.0.0.1".to_owned(),
        // Port zero: the system picks one, and the outcome says which.
        bind_port: 0,
        mode: ForwardMode::Dynamic,
        // Left empty on purpose: a dynamic forward is told where to go by each client, and an
        // address saved here would be a destination nobody asked for.
        target_host: String::new(),
        target_port: 0,
    }];
    lab.vault
        .save_session(
            forwarding,
            Some(SessionSecret::Password(
                environment("SEVENSHELL_TEST_PASSWORD").unwrap_or_default(),
            )),
        )
        .expect("save");
    lab.session_id = "lab-socks".to_owned();

    let mut bound = None;
    for _ in 0..4 {
        match lab.core.open_terminal(&lab.session_id).await.expect("open") {
            OpenOutcome::Opened { handle, tunnels, .. } => {
                assert_eq!(tunnels.len(), 1);
                assert_eq!(tunnels[0].mode, ForwardMode::Dynamic);
                bound = Some((handle, tunnels[0].bind_port));
                break;
            }
            OpenOutcome::HostKeyUnknown { key } => lab.core.trust_host_key(&key).expect("trust"),
            other => panic!("expected the session to open, got {other:?}"),
        }
    }

    let (handle, port) = bound.expect("the forward is up");
    let lab_port = lab.vault.sessions().expect("sessions")[0].port;

    // A name rather than an address: `localhost` means the far side, whichever machine this is.
    let banner = socks5_ssh_banner(port, "localhost", lab_port).await;
    assert!(
        banner.starts_with("SSH-"),
        "the SOCKS5 proxy reached the far side's sshd; got {banner:?}"
    );

    // The count is the proof the connection was carried rather than refused somewhere.
    let tunnels = lab.core.tunnels().await;
    let socks = tunnels
        .iter()
        .find(|tunnel| tunnel.id.ends_with("socks"))
        .expect("the tunnel");
    assert!(socks.connections >= 1, "connections: {}", socks.connections);

    // And a destination the far side cannot reach is refused as one rather than as a broken tunnel.
    let refused = socks5_refusal(port, 1).await;
    assert_eq!(refused, 0x04, "host unreachable: the far side said no");

    lab.core.close_terminal(&handle).await.expect("close");
    assert!(
        lab.core.tunnels().await.is_empty(),
        "a tunnel does not outlive the terminal it belongs to"
    );
}

/// Speaks SOCKS5 to `port`, asks for `host:target`, and reads what the destination says first.
async fn socks5_ssh_banner(port: u16, host: &str, target: u16) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to the SOCKS5 listener");

    stream.write_all(&[0x05, 0x01, 0x00]).await.expect("the greeting");
    let mut method = [0_u8; 2];
    stream.read_exact(&mut method).await.expect("the method");
    assert_eq!(method, [0x05, 0x00]);

    let mut request = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&target.to_be_bytes());
    stream.write_all(&request).await.expect("the request");

    let mut reply = [0_u8; 10];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut reply))
        .await
        .expect("the reply arrives")
        .expect("ten bytes of it");
    assert_eq!(reply[1], 0x00, "the connection was granted");

    let mut banner = vec![0_u8; 64];
    let read = stream.read(&mut banner).await.expect("the banner");
    banner.truncate(read);
    String::from_utf8_lossy(&banner).into_owned()
}

/// The reply code a request gets when the destination cannot be reached from the far side.
async fn socks5_refusal(port: u16, target: u16) -> u8 {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to the SOCKS5 listener");
    stream.write_all(&[0x05, 0x01, 0x00]).await.expect("the greeting");
    let mut method = [0_u8; 2];
    stream.read_exact(&mut method).await.expect("the method");

    let mut request = vec![0x05, 0x01, 0x00, 0x01];
    request.extend_from_slice(&[127, 0, 0, 1]);
    request.extend_from_slice(&target.to_be_bytes());
    stream.write_all(&request).await.expect("the request");

    let mut reply = [0_u8; 10];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut reply))
        .await
        .expect("the refusal arrives")
        .expect("ten bytes of it");
    reply[1]
}

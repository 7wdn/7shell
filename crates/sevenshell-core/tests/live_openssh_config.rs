//! An OpenSSH alias, a real configuration file and a real known_hosts.
//!
//! Ignored by default: it needs a running sshd. `deploy/lab/` has one and `make lab-up` starts it.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 SEVENSHELL_TEST_USER=labuser \
//!   SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-core --test live_openssh_config -- --ignored --test-threads=1
//! ```
//!
//! What this proves that a unit test cannot: the alias is resolved *before* anything is dialled,
//! the resolved account and port are the ones the server accepts, and a key already recorded in the
//! machine's own `known_hosts` removes the trust question without this application writing that
//! file.

use std::sync::Arc;
use std::time::Duration;

use sevenshell_core::Core;
use sevenshell_error::ErrorCode;
use sevenshell_ssh::{Credential, OpenRequest, Purpose, Route, SshSession, Target, TerminalSize};
use sevenshell_types::{AuthMethod, KeyAlgorithm, OpenOutcome, Protocol, Session, SessionAuth, TerminalSettings};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};

const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

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

/// A vault with one session that stores the alias rather than the coordinates.
fn lab_with_alias() -> Option<(TempDir, Arc<Vault>, Arc<Core>, String)> {
    let (host, port, user, password) = lab_credentials()?;

    let directory = tempdir().expect("temp dir");
    let ssh = directory.path().join("ssh");
    std::fs::create_dir_all(&ssh).expect("ssh dir");
    std::fs::write(
        ssh.join("config"),
        format!("Host lab\n  HostName {host}\n  Port {port}\n  User {user}\n"),
    )
    .expect("config");
    // Safety: this test binary is run with one thread, and the directory is this test's own.
    unsafe { std::env::set_var(sevenshell_platform::SSH_DIR_OVERRIDE, &ssh) };

    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    let session = Session {
        id: "openssh-lab".to_owned(),
        name: "lab through its alias".to_owned(),
        group_id: None,
        order: None,
        protocol: Protocol::Ssh,
        // Deliberately wrong: the configuration is the authority, and these values must not be
        // the ones the server sees.
        host: "lab".to_owned(),
        port: 1,
        user: "definitely-not-the-user".to_owned(),
        auth: SessionAuth {
            method: AuthMethod::Password,
            key: None,
            has_secret: false,
            ssh_config: true,
        },
        terminal: TerminalSettings::default(),
        desktop: sevenshell_types::DesktopSettings::default(),
        telnet: sevenshell_types::TelnetSettings::default(),
        serial: sevenshell_types::SerialSettings::default(),
        local: sevenshell_types::LocalSettings::default(),
        wsl: sevenshell_types::WslSettings::default(),
        container: sevenshell_types::ContainerSettings::default(),
        kubernetes: sevenshell_types::KubernetesSettings::default(),
        webdav: sevenshell_types::WebDavSettings::default(),
        ftp: sevenshell_types::FtpSettings::default(),
        vmware: sevenshell_types::VmwareSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    };

    vault
        .save_session(session, Some(SessionSecret::Password(password)))
        .expect("save session");
    let (core, _events) = Core::new(Arc::clone(&vault));
    Some((directory, vault, core, "openssh-lab".to_owned()))
}

#[tokio::test]
#[ignore = "needs the SSH lab: see the module comment"]
async fn an_alias_resolves_and_a_system_known_host_needs_no_question() {
    let Some((directory, _vault, core, session_id)) = lab_with_alias() else {
        return;
    };

    // The first attempt must reach the host key step: if the alias had not resolved, the wrong
    // stored user or port would have failed first.
    let presented = match core.open_terminal(&session_id).await.expect("the first open answers") {
        OpenOutcome::HostKeyUnknown { key } => key,
        other => panic!("expected a host key question, got {other:?}"),
    };

    let ssh = directory.path().join("ssh");
    let name = if presented.port == 22 {
        presented.host.clone()
    } else {
        format!("[{}]:{}", presented.host, presented.port)
    };
    std::fs::write(
        ssh.join("known_hosts"),
        format!("{} {} {}\n", name, presented.key_type, presented.public_key),
    )
    .expect("known_hosts");

    // No vault trust was recorded above. The system's own file is what answers now.
    match core.open_terminal(&session_id).await.expect("the second open answers") {
        OpenOutcome::Opened { handle, .. } => {
            core.close_terminal(&handle).await.expect("the shell closes");
        }
        OpenOutcome::HostKeyUnknown { .. } => {
            panic!("a key already in the system known_hosts was asked about again");
        }
        other => panic!("expected the session to open, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab: see the module comment"]
async fn a_changed_key_in_the_system_known_hosts_is_a_hard_stop() {
    let Some((directory, _vault, core, session_id)) = lab_with_alias() else {
        return;
    };

    // Learn the algorithm the server uses, so the wrong key written below is a *changed* key
    // rather than a different algorithm (which OpenSSH treats as "not seen").
    let presented = match core.open_terminal(&session_id).await.expect("the first open answers") {
        OpenOutcome::HostKeyUnknown { key } => key,
        other => panic!("expected a host key question, got {other:?}"),
    };
    if presented.key_type != "ssh-ed25519" {
        eprintln!(
            "the lab presents {}, so this test cannot build a same-algorithm wrong key; skipping",
            presented.key_type
        );
        return;
    }

    let generated =
        sevenshell_ssh::generate(sevenshell_types::KeyAlgorithm::Ed25519, "test", None).expect("a generated key");
    let mut parts = generated.public_line().split_whitespace();
    let _key_type = parts.next().expect("a key type");
    let wrong = parts.next().expect("a public key");

    let ssh = directory.path().join("ssh");
    let name = if presented.port == 22 {
        presented.host.clone()
    } else {
        format!("[{}]:{}", presented.host, presented.port)
    };
    std::fs::write(ssh.join("known_hosts"), format!("{name} ssh-ed25519 {wrong}\n")).expect("known_hosts");

    let error = core
        .open_terminal(&session_id)
        .await
        .expect_err("a changed key must stop the connection");
    assert_eq!(error.code(), ErrorCode::SshHostKeySystemMismatch);
}

#[tokio::test]
#[ignore = "needs the SSH lab: see the module comment"]
async fn an_identity_file_in_the_config_authenticates_the_session() {
    let Some((host, port, user, password)) = lab_credentials() else {
        return;
    };

    // One password connection, used only to put this run's public half on the server. The session
    // under test never sees the password.
    let presented = sevenshell_ssh::probe_host_key(&host, port, OPEN_TIMEOUT)
        .await
        .expect("the host key is readable");
    let request = OpenRequest {
        target: Target {
            host: host.clone(),
            port,
            user: user.clone(),
        },
        credential: Credential::Password(password),
        terminal_type: String::new(),
        keepalive_seconds: 0,
        forward_agent: false,
        timeout: OPEN_TIMEOUT,
        expected_host_key: presented.clone(),
        size: TerminalSize::default(),
        route: Route::Direct,
        purpose: Purpose::Carrier,
    };
    let (events, _unused) = tokio::sync::mpsc::channel(1);
    let installing = SshSession::open(request, events)
        .await
        .expect("the password connection opens");

    let comment = format!("7shell-config-key-{}", std::process::id());
    let made = sevenshell_ssh::generate(KeyAlgorithm::Ed25519, &comment, None).expect("a generated key");
    installing
        .exec(
            &format!(
                "mkdir -p ~/.ssh && chmod 700 ~/.ssh && printf '%s\\n' '{}' >> ~/.ssh/authorized_keys && \
                 chmod 600 ~/.ssh/authorized_keys",
                made.public_line()
            ),
            OPEN_TIMEOUT,
        )
        .await
        .expect("the public half goes into authorized_keys");

    let directory = tempdir().expect("temp dir");
    let ssh = directory.path().join("ssh");
    std::fs::create_dir_all(&ssh).expect("ssh dir");
    std::fs::write(
        ssh.join("config"),
        format!("Host lab-key\n  HostName {host}\n  Port {port}\n  User {user}\n  IdentityFile ~/.ssh/id_config\n"),
    )
    .expect("config");
    std::fs::write(ssh.join("id_config"), made.private_text()).expect("the key file");
    let name = if port == 22 {
        host.clone()
    } else {
        format!("[{host}]:{port}")
    };
    std::fs::write(
        ssh.join("known_hosts"),
        format!("{name} {} {}\n", presented.key_type, presented.public_key),
    )
    .expect("known_hosts");
    // Safety: this test binary is run with one thread, and the directory is this test's own.
    unsafe { std::env::set_var(sevenshell_platform::SSH_DIR_OVERRIDE, &ssh) };

    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");
    let session = Session {
        id: "openssh-key".to_owned(),
        name: "lab through its key".to_owned(),
        group_id: None,
        order: None,
        protocol: Protocol::Ssh,
        host: "lab-key".to_owned(),
        port: 1,
        user: "definitely-not-the-user".to_owned(),
        auth: SessionAuth {
            method: AuthMethod::PublicKey,
            key: None,
            has_secret: false,
            ssh_config: true,
        },
        terminal: TerminalSettings::default(),
        desktop: sevenshell_types::DesktopSettings::default(),
        telnet: sevenshell_types::TelnetSettings::default(),
        serial: sevenshell_types::SerialSettings::default(),
        local: sevenshell_types::LocalSettings::default(),
        wsl: sevenshell_types::WslSettings::default(),
        container: sevenshell_types::ContainerSettings::default(),
        kubernetes: sevenshell_types::KubernetesSettings::default(),
        webdav: sevenshell_types::WebDavSettings::default(),
        ftp: sevenshell_types::FtpSettings::default(),
        vmware: sevenshell_types::VmwareSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    };
    vault.save_session(session, None).expect("save session");
    let (core, _events) = Core::new(vault);

    let result = core.open_terminal("openssh-key").await;
    let _ = installing
        .exec(
            &format!("sed -i '/{comment}/d' ~/.ssh/authorized_keys; echo removed"),
            OPEN_TIMEOUT,
        )
        .await;

    match result.expect("the configuration's key authenticates") {
        OpenOutcome::Opened { handle, .. } => {
            core.close_terminal(&handle).await.expect("the shell closes");
        }
        other => panic!("expected the session to open, got {other:?}"),
    }
}

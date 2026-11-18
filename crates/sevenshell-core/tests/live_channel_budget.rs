//! What happens when one connection has more channels asked of it than its server allows.
//!
//! Ignored by default: it needs the SSH lab. The lab's sshd runs the OpenSSH default
//! `MaxSessions 10`, so a terminal plus ten file browsers is one more channel than one connection
//! will carry — and a file browser is expected to open a connection of its own rather than fail.

use std::sync::Arc;

use sevenshell_core::Core;
use sevenshell_types::{AuthMethod, OpenOutcome, Session, SessionAuth, SftpOutcome, TerminalSettings};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::tempdir;

struct Lab {
    _directory: tempfile::TempDir,
    /// Held open on purpose: a registry whose event receiver is gone drops the terminals it knows.
    _events: tokio::sync::mpsc::Receiver<sevenshell_core::CoreEvent>,
    core: Arc<Core>,
    session_id: String,
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn lab() -> Option<Lab> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return None;
    };

    let Ok(port) = port.parse() else {
        eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
        return None;
    };

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the vault password, which is not the one under test")
        .expect("create");

    let session = Session {
        id: "lab-channels".to_owned(),
        name: "lab channels".to_owned(),
        group_id: None,
        order: None,
        protocol: sevenshell_types::Protocol::Ssh,
        host,
        port,
        user,
        auth: SessionAuth::with_password(AuthMethod::Password),
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
        .expect("save");

    let (core, events) = Core::new(vault);
    Some(Lab {
        _directory: directory,
        _events: events,
        core,
        session_id: "lab-channels".to_owned(),
    })
}

#[tokio::test]
#[ignore = "needs the SSH lab"]
async fn many_samples_on_one_terminal_do_not_use_up_its_channels() {
    let Some(lab) = lab() else {
        return;
    };

    let opened = lab
        .core
        .open_terminal(&lab.session_id)
        .await
        .expect("the terminal opens");
    let handle = match opened {
        OpenOutcome::Opened { handle, .. } => handle,
        OpenOutcome::HostKeyUnknown { key } => {
            lab.core.trust_host_key(&key).expect("trust");
            let OpenOutcome::Opened { handle, .. } = lab
                .core
                .open_terminal(&lab.session_id)
                .await
                .expect("the terminal opens")
            else {
                panic!("the lab host key is still not trusted");
            };
            handle
        }
        other => panic!("unexpected open outcome: {other:?}"),
    };

    for index in 1..=25 {
        lab.core.sample_host(&handle).await.unwrap_or_else(|error| {
            panic!(
                "sample {index} failed: {} ({})",
                error.code(),
                error.detail().unwrap_or("")
            )
        });
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab"]
async fn a_file_browser_opens_past_the_terminal_connections_channel_budget() {
    let Some(lab) = lab() else {
        return;
    };

    let opened = lab
        .core
        .open_terminal(&lab.session_id)
        .await
        .expect("the terminal opens");
    let handle = match opened {
        OpenOutcome::Opened { handle, .. } => handle,
        OpenOutcome::HostKeyUnknown { key } => {
            lab.core.trust_host_key(&key).expect("trust");
            let OpenOutcome::Opened { handle, .. } = lab
                .core
                .open_terminal(&lab.session_id)
                .await
                .expect("the terminal opens")
            else {
                panic!("the lab host key is still not trusted");
            };
            handle
        }
        other => panic!("unexpected open outcome: {other:?}"),
    };

    let mut failures = Vec::new();
    for index in 1..=12 {
        match lab.core.open_files(&lab.session_id).await {
            Ok(SftpOutcome::Opened { .. }) => eprintln!("browser {index}: opened"),
            Ok(SftpOutcome::HostKeyUnknown { key }) => {
                lab.core.trust_host_key(&key).expect("trust");
                match lab.core.open_files(&lab.session_id).await {
                    Ok(SftpOutcome::Opened { .. }) => eprintln!("browser {index}: opened after trust"),
                    Ok(other) => failures.push(format!("browser {index}: decision {other:?}")),
                    Err(error) => failures.push(format!(
                        "browser {index}: {} ({})",
                        error.code(),
                        error.detail().unwrap_or("")
                    )),
                }
            }
            Ok(other) => failures.push(format!("browser {index}: decision {other:?}")),
            Err(error) => failures.push(format!(
                "browser {index}: {} ({})",
                error.code(),
                error.detail().unwrap_or("")
            )),
        }
    }

    match lab.core.sample_host(&handle).await {
        Ok(_) => eprintln!("sample: ok"),
        Err(error) => failures.push(format!("sample: {} ({})", error.code(), error.detail().unwrap_or(""))),
    }

    assert!(
        failures.is_empty(),
        "channels were exhausted instead of a new connection opening:\n{}",
        failures.join("\n")
    );
}

//! A server with no SFTP subsystem, browsed through the shell and SCP.
//!
//! Ignored by default: it needs the SSH lab with SFTP deliberately switched off. The proof that
//! the fallback happened is the `packed` flag on the opened browser — SFTP sets it, SCP cannot.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=3222 SEVENSHELL_TEST_REMOTE_PORT=2222 \
//! SEVENSHELL_TEST_USER=labuser SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-core --test live_scp -- --ignored
//! ```

use std::sync::Arc;
use std::time::Duration;

use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, EntryKind, FtpSettings, Protocol, Session, SessionAuth, SftpOutcome, TerminalSettings, TransferState,
    WebDavSettings, WslSettings,
};
use sevenshell_vault::{SessionSecret, Vault};

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn lab_session() -> Option<Session> {
    let host = environment("SEVENSHELL_TEST_HOST")?;
    let port = environment("SEVENSHELL_TEST_PORT")?.parse().ok()?;
    let user = environment("SEVENSHELL_TEST_USER")?;
    Some(Session {
        id: "lab-scp".to_owned(),
        name: "lab scp".to_owned(),
        group_id: None,
        protocol: Protocol::Ssh,
        host,
        port,
        user,
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        desktop: sevenshell_types::DesktopSettings::default(),
        telnet: sevenshell_types::TelnetSettings::default(),
        serial: sevenshell_types::SerialSettings::default(),
        local: sevenshell_types::LocalSettings::default(),
        wsl: WslSettings::default(),
        container: sevenshell_types::ContainerSettings::default(),
        kubernetes: sevenshell_types::KubernetesSettings::default(),
        webdav: WebDavSettings::default(),
        ftp: FtpSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 1_700_000_000,
    })
}

async fn settle(core: &Core, id: &str) -> TransferState {
    for _ in 0..300 {
        if let Some(info) = core.transfers().await.into_iter().find(|info| info.id == id)
            && !matches!(info.state, TransferState::Running | TransferState::Paused)
        {
            return info.state;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the transfer did not settle");
}

#[tokio::test]
#[ignore = "needs the SSH lab with SFTP disabled: see deploy/lab/README.md"]
async fn a_server_without_sftp_is_browsed_through_the_shell_and_scp() {
    // The lab usually has SFTP. This test is meaningless there, and the CI lab run must not fail
    // because of it: the switch is what says the server was prepared for the fallback.
    if environment("SEVENSHELL_TEST_SCP_FALLBACK").as_deref() != Some("1") {
        return;
    }
    let Some(session) = lab_session() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("master password").expect("a vault");
    vault
        .save_session(
            session,
            Some(SessionSecret::Password(
                environment("SEVENSHELL_TEST_PASSWORD").unwrap_or_default(),
            )),
        )
        .expect("a session");
    let (core, _events) = Core::new(vault);

    let mut opened = None;
    for _ in 0..4 {
        match core.open_files("lab-scp").await.expect("open") {
            SftpOutcome::Opened {
                handle,
                listing,
                packed,
                ..
            } => {
                opened = Some((handle, listing, packed));
                break;
            }
            SftpOutcome::HostKeyUnknown { key } => core.trust_host_key(&key).expect("trust"),
            other => panic!("expected a browser, got {other:?}"),
        }
    }
    let (handle, listing, packed) = opened.expect("the browser opened");
    assert!(
        !packed,
        "SFTP was available: this test only means something with the subsystem switched off"
    );
    assert!(
        listing.entries.iter().any(|entry| entry.name == "big-directory"),
        "the shell listing did not show the lab fixture: {:?}",
        listing.entries.iter().map(|entry| &entry.name).collect::<Vec<_>>()
    );

    let name = format!("7shell-scp-live-{}.txt", std::process::id());
    let local = directory.path().join(&name);
    std::fs::write(&local, b"scp fallback").expect("write");
    let upload = core
        .start_upload(&handle, local, &listing.path)
        .await
        .expect("an upload");
    assert_eq!(settle(&core, &upload).await, TransferState::Done);

    let target = directory.path().join("returned.txt");
    let remote = format!("{}/{}", listing.path, name);
    let download = core
        .start_download(&handle, &remote, target.clone())
        .await
        .expect("a download");
    assert_eq!(settle(&core, &download).await, TransferState::Done);
    assert_eq!(
        std::fs::read_to_string(&target).expect("the file landed"),
        "scp fallback",
        "SCP did not carry the bytes both ways"
    );

    core.delete_entry(&handle, &remote, EntryKind::File)
        .await
        .expect("the uploaded file was removed");
}

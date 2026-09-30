//! The file browser against a real server: list, create, move, read, write, delete, and a transfer
//! in each direction.
//!
//! Ignored by default: it needs the SSH lab. `make lab-up` starts one, and `make lab-verify` runs
//! this alongside the connection tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sevenshell_core::Core;
use sevenshell_types::{AuthMethod, EntryKind, Session, SessionAuth, SftpOutcome, TerminalSettings, TransferState};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};

struct Lab {
    _directory: TempDir,
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
        id: "lab-files".to_owned(),
        name: "lab files".to_owned(),
        group_id: None,
        protocol: sevenshell_types::Protocol::Ssh,
        host,
        port,
        user,
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    };
    vault
        .save_session(session, Some(SessionSecret::Password(password)))
        .expect("save");

    let (core, _events) = Core::new(vault);
    Some(Lab {
        _directory: directory,
        core,
        session_id: "lab-files".to_owned(),
    })
}

impl Lab {
    /// Opens the browser, trusting whatever key it is asked about — what a person does once.
    async fn open_trusting_everything(&self) -> String {
        for _ in 0..4 {
            match self.core.open_files(&self.session_id).await.expect("open") {
                SftpOutcome::Opened { handle, .. } => return handle,
                SftpOutcome::HostKeyUnknown { key } => self.core.trust_host_key(&key).expect("trust"),
                other => panic!("expected the browser to open, got {other:?}"),
            }
        }
        panic!("the route never finished asking");
    }

    /// Waits for a transfer to stop running, and says how it stopped.
    async fn settle(&self, id: &str) -> TransferState {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let transfer = self
                .core
                .transfers()
                .await
                .into_iter()
                .find(|transfer| transfer.id == id)
                .expect("the transfer is remembered");

            if !matches!(transfer.state, TransferState::Running) {
                return transfer.state;
            }

            assert!(Instant::now() < deadline, "the transfer never finished");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn browsing_a_real_directory_and_changing_it() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;

    // The starting directory lists, and every entry names its own full path.
    let home = lab
        .core
        .list_files(&handle, ".")
        .await
        .expect("list the home directory");
    assert!(
        home.path.starts_with('/'),
        "the listing resolves to an absolute path: {}",
        home.path
    );
    for entry in &home.entries {
        assert!(entry.path.starts_with(home.path.trim_end_matches('/')) || home.path == "/");
    }

    let workspace = format!("{}/7shell-live-files", home.path.trim_end_matches('/'));

    // Whatever an earlier run left behind goes first, so this test can be run twice — and a run
    // that failed half way through does not poison the next one.
    clear_workspace(&lab, &handle, &workspace).await;

    lab.core
        .create_folder(&handle, &workspace)
        .await
        .expect("create a folder");

    let note = format!("{workspace}/note.txt");
    lab.core
        .write_text_file(&handle, &note, "hello from 7Shell\n")
        .await
        .expect("write a file");

    let listing = lab.core.list_files(&handle, &workspace).await.expect("list the folder");
    let entry = listing
        .entries
        .iter()
        .find(|entry| entry.name == "note.txt")
        .expect("the file is listed");
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(entry.size, 18, "the size came back from the server");

    assert_eq!(
        lab.core.read_text_file(&handle, &note).await.expect("read it back"),
        "hello from 7Shell\n"
    );

    let renamed = format!("{workspace}/renamed.txt");
    lab.core
        .rename_entry(&handle, &note, &renamed)
        .await
        .expect("rename it");
    assert!(
        lab.core
            .list_files(&handle, &workspace)
            .await
            .expect("list")
            .entries
            .iter()
            .any(|entry| entry.name == "renamed.txt"),
        "the new name is there"
    );

    // Down, then up, then compare: the bytes have to survive both directions.
    let scratch = tempdir().expect("temp dir");
    let downloaded = scratch.path().join("downloaded.txt");
    let download = lab
        .core
        .start_download(&handle, &renamed, downloaded.clone())
        .await
        .expect("start a download");
    assert_eq!(
        lab.settle(&download).await,
        TransferState::Done,
        "the download finished"
    );
    assert_eq!(
        std::fs::read_to_string(&downloaded).expect("read the local copy"),
        "hello from 7Shell\n"
    );

    let source = scratch.path().join("upload.txt");
    std::fs::write(&source, "the other direction\n").expect("write a local file");
    let upload = lab
        .core
        .start_upload(&handle, source, &workspace)
        .await
        .expect("start an upload");
    assert_eq!(lab.settle(&upload).await, TransferState::Done, "the upload finished");
    assert_eq!(
        lab.core
            .read_text_file(&handle, &format!("{workspace}/upload.txt"))
            .await
            .expect("read it back"),
        "the other direction\n"
    );

    // A folder takes its contents with it, and then it is gone.
    lab.core
        .delete_entry(&handle, &workspace, EntryKind::Directory)
        .await
        .expect("delete the folder");
    assert!(
        lab.core.list_files(&handle, &workspace).await.is_err(),
        "the folder is no longer there"
    );

    lab.core.close_files(&handle).await.expect("close the browser");
    assert!(
        lab.core.list_files(&handle, ".").await.is_err(),
        "a closed browser is closed"
    );
}

/// Removes the test's folder if it is there, and says nothing when it is not.
async fn clear_workspace(lab: &Lab, handle: &str, workspace: &str) {
    if lab.core.list_files(handle, workspace).await.is_ok() {
        lab.core
            .delete_entry(handle, workspace, EntryKind::Directory)
            .await
            .expect("remove what an earlier run left");
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_missing_file_says_what_is_missing() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;

    let error = lab
        .core
        .read_text_file(&handle, "/definitely-not-here-7shell")
        .await
        .expect_err("there is no such file");

    assert_eq!(error.code(), sevenshell_error::ErrorCode::SftpNotFound);
    assert_eq!(
        error.message_params().get("path").map(String::as_str),
        Some("/definitely-not-here-7shell")
    );
}

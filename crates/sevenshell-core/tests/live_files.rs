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

/// A folder named to be awkward, which is the point: it holds a space and an apostrophe, and it is
/// the name that gets interpolated into the `tar` command line.
const AWKWARD: &str = "it's a folder";

/// Creates a workspace with one awkwardly named folder holding two files, and returns both paths.
async fn packed_workspace(lab: &Lab, handle: &str) -> (String, String) {
    let home = lab.core.list_files(handle, ".").await.expect("list the home directory");
    let workspace = format!("{}/7shell-live-packed", home.path.trim_end_matches('/'));
    clear_workspace(lab, handle, &workspace).await;

    let inner = format!("{workspace}/{AWKWARD}");
    lab.core
        .create_folder(handle, &workspace)
        .await
        .expect("create the workspace");
    lab.core
        .create_folder(handle, &inner)
        .await
        .expect("create the folder being packed");
    lab.core
        .write_text_file(handle, &format!("{inner}/one.txt"), "first\n")
        .await
        .expect("write the first file");
    lab.core
        .write_text_file(handle, &format!("{inner}/two.txt"), "second\n")
        .await
        .expect("write the second file");

    (workspace, inner)
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_folder_comes_down_and_is_opened_where_it_was_asked_for() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, inner) = packed_workspace(&lab, &handle).await;

    let scratch = tempdir().expect("temp dir");
    let into = scratch.path().join("somewhere");
    std::fs::create_dir(&into).expect("a folder to land in");

    let id = lab
        .core
        .start_packed_download(&handle, std::slice::from_ref(&inner), into.clone())
        .await
        .expect("start the packed download");
    let state = lab.settle(&id).await;
    assert_eq!(state, TransferState::Done, "the packed download finished: {state:?}");

    // What arrived is the folder itself, named and filled as it was on the far side. The archive was
    // a vehicle and is gone from this side — which is the whole of what "both ends unpack" means.
    let landed = into.join(AWKWARD);
    assert_eq!(
        std::fs::read_to_string(landed.join("one.txt")).expect("read the first file"),
        "first\n"
    );
    assert_eq!(
        std::fs::read_to_string(landed.join("two.txt")).expect("read the second file"),
        "second\n"
    );
    assert!(
        !std::env::temp_dir().join(format!("7shell-{id}.tar.gz")).exists(),
        "the archive this machine downloaded through is not left behind"
    );

    // The size was unknown when the transfer was asked for — nothing had been packed yet — and by the
    // end it is the archive's own, which is what the progress bar counted against.
    let transfer = lab
        .core
        .transfers()
        .await
        .into_iter()
        .find(|transfer| transfer.id == id)
        .expect("the transfer is remembered");
    assert!(transfer.size > 0, "the row knows how much crossed");
    assert!(transfer.packed, "and that it crossed packed");

    // Neither is the archive that was made on the far side left lying in the home directory.
    let home = lab
        .core
        .list_files(&handle, ".")
        .await
        .expect("list the home directory");
    assert!(
        !home.entries.iter().any(|entry| entry.name.starts_with(".7shell-")),
        "the staging file was cleaned up: {:?}",
        home.entries.iter().map(|entry| &entry.name).collect::<Vec<_>>()
    );

    clear_workspace(&lab, &handle, &workspace).await;
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn files_chosen_here_are_packed_and_land_there() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    // Two ordinary local files, one of them named awkwardly: this is what somebody chooses in the
    // dialog, and packing them is this side's job rather than theirs.
    let scratch = tempdir().expect("temp dir");
    let first = scratch.path().join("notes.txt");
    let second = scratch.path().join("it's a report.txt");
    std::fs::write(&first, "chosen here\n").expect("write");
    std::fs::write(&second, "also chosen here\n").expect("write");

    let target = format!("{workspace}/landed");
    lab.core
        .create_folder(&handle, &target)
        .await
        .expect("create the folder to land in");

    let id = lab
        .core
        .start_packed_upload(&handle, vec![first, second], &target)
        .await
        .expect("start the packed upload");
    assert_eq!(lab.settle(&id).await, TransferState::Done, "the packed upload finished");

    // Each file is there under its own name, with its own contents: what crossed the wire was one
    // archive, and what the far side ended up with is the files themselves.
    assert_eq!(
        lab.core
            .read_text_file(&handle, &format!("{target}/notes.txt"))
            .await
            .expect("read the first file back"),
        "chosen here\n"
    );
    assert_eq!(
        lab.core
            .read_text_file(&handle, &format!("{target}/it's a report.txt"))
            .await
            .expect("read the second file back"),
        "also chosen here\n"
    );

    // The row says what happened, which is the half of this feature somebody has to be able to see:
    // a batch that was packed, and how much went into it.
    let transfer = lab
        .core
        .transfers()
        .await
        .into_iter()
        .find(|transfer| transfer.id == id)
        .expect("the transfer is remembered");
    assert_eq!(transfer.name, "upload_pack_2_files.tar.gz");
    assert!(transfer.packed, "the row says it travelled packed");
    assert_eq!(
        transfer.remote_path, target,
        "and it names the directory the files landed in, which is what the panel reads again"
    );
    assert!(transfer.size > 0, "the row knows how much crossed");

    // Neither archive is left behind: not the one made here, and not the one that was unpacked there.
    assert!(
        !std::env::temp_dir().join(format!("7shell-{id}.tar.gz")).exists(),
        "the archive this machine made is gone"
    );
    let home = lab
        .core
        .list_files(&handle, ".")
        .await
        .expect("list the home directory");
    assert!(
        !home.entries.iter().any(|entry| entry.name.starts_with(".7shell-")),
        "the staging file on the far side is gone: {:?}",
        home.entries.iter().map(|entry| &entry.name).collect::<Vec<_>>()
    );

    clear_workspace(&lab, &handle, &workspace).await;
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_folder_survives_the_whole_way_round() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, inner) = packed_workspace(&lab, &handle).await;

    // Down: the far side packs the folder, this side opens it into a folder somebody picked.
    let scratch = tempdir().expect("temp dir");
    let into = scratch.path().join("downloaded");
    std::fs::create_dir(&into).expect("a folder to land in");
    let down = lab
        .core
        .start_packed_download(&handle, std::slice::from_ref(&inner), into.clone())
        .await
        .expect("start the packed download");
    assert_eq!(
        lab.settle(&down).await,
        TransferState::Done,
        "the packed download finished"
    );

    // Up: the files that just arrived are chosen and packed again, into a different remote folder.
    let landed = into.join(AWKWARD);
    let target = format!("{workspace}/back-again");
    lab.core
        .create_folder(&handle, &target)
        .await
        .expect("create the folder to land in");
    let up = lab
        .core
        .start_packed_upload(&handle, vec![landed.join("one.txt"), landed.join("two.txt")], &target)
        .await
        .expect("start the packed upload");
    assert_eq!(lab.settle(&up).await, TransferState::Done, "the packed upload finished");

    // Round the whole circle, the bytes are the ones the far side started with — which is the only
    // thing a transfer promises, whichever way it went.
    assert_eq!(
        lab.core
            .read_text_file(&handle, &format!("{target}/one.txt"))
            .await
            .expect("read the first file back"),
        "first\n"
    );
    assert_eq!(
        lab.core
            .read_text_file(&handle, &format!("{target}/two.txt"))
            .await
            .expect("read the second file back"),
        "second\n"
    );

    clear_workspace(&lab, &handle, &workspace).await;
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn several_chosen_entries_come_down_as_one_archive() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, inner) = packed_workspace(&lab, &handle).await;

    // A file beside the awkwardly named folder, so what was chosen is a mixture: that is the shape a
    // selection usually has, and the shape one `tar` command has to take in its stride.
    let loose = format!("{workspace}/loose.txt");
    lab.core
        .write_text_file(&handle, &loose, "loose\n")
        .await
        .expect("write the loose file");

    let scratch = tempdir().expect("temp dir");
    let into = scratch.path().join("selection");
    std::fs::create_dir(&into).expect("a folder to land in");

    let id = lab
        .core
        .start_packed_download(&handle, &[loose.clone(), inner.clone()], into.clone())
        .await
        .expect("start the packed download");
    assert_eq!(
        lab.settle(&id).await,
        TransferState::Done,
        "the packed download finished"
    );

    // Each of them landed under its own name: the file where it was, the folder with its contents.
    assert_eq!(
        std::fs::read_to_string(into.join("loose.txt")).expect("read the loose file"),
        "loose\n"
    );
    assert_eq!(
        std::fs::read_to_string(into.join(AWKWARD).join("one.txt")).expect("read the first file"),
        "first\n"
    );
    assert_eq!(
        std::fs::read_to_string(into.join(AWKWARD).join("two.txt")).expect("read the second file"),
        "second\n"
    );

    // One row for the whole selection, named for what it was rather than for one of the things in it.
    let transfer = lab
        .core
        .transfers()
        .await
        .into_iter()
        .find(|transfer| transfer.id == id)
        .expect("the transfer is remembered");
    assert_eq!(transfer.name, "download_pack_2_items.tar.gz");
    assert!(transfer.packed);
    assert_eq!(transfer.remote_path, workspace, "it says where they came from");

    // And neither archive is left behind.
    assert!(
        !std::env::temp_dir().join(format!("7shell-{id}.tar.gz")).exists(),
        "the archive this machine downloaded through is gone"
    );
    let home = lab
        .core
        .list_files(&handle, ".")
        .await
        .expect("list the home directory");
    assert!(
        !home.entries.iter().any(|entry| entry.name.starts_with(".7shell-")),
        "the staging file on the far side is gone: {:?}",
        home.entries.iter().map(|entry| &entry.name).collect::<Vec<_>>()
    );

    clear_workspace(&lab, &handle, &workspace).await;
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn creating_a_name_that_is_taken_is_refused_rather_than_emptied() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    // The awkward folder this makes is not what this test is about; it is only the workspace's shape.
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    let created = format!("{workspace}/new.txt");
    lab.core
        .create_file(&handle, &created)
        .await
        .expect("create an empty file");
    let entry = lab
        .core
        .list_files(&handle, &workspace)
        .await
        .expect("list the workspace")
        .entries
        .into_iter()
        .find(|entry| entry.name == "new.txt")
        .expect("the new file is listed");
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(entry.size, 0, "an empty file, which is what was asked for");

    // What matters about the second attempt is not the error but what it did *not* do: SFTP's plain
    // create opens with truncate, so a refusal is the only thing standing between a name somebody
    // mistyped and a file that quietly lost its contents.
    lab.core
        .write_text_file(&handle, &created, "keep me\n")
        .await
        .expect("write something into it");
    let error = lab
        .core
        .create_file(&handle, &created)
        .await
        .expect_err("the name is taken");
    assert_eq!(error.code(), sevenshell_error::ErrorCode::SftpExists);
    assert_eq!(
        lab.core.read_text_file(&handle, &created).await.expect("read it back"),
        "keep me\n"
    );

    let folder = format!("{workspace}/taken");
    lab.core.create_folder(&handle, &folder).await.expect("create a folder");
    let error = lab
        .core
        .create_folder(&handle, &folder)
        .await
        .expect_err("the folder name is taken too");
    assert_eq!(error.code(), sevenshell_error::ErrorCode::SftpExists);

    clear_workspace(&lab, &handle, &workspace).await;
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn an_upload_is_told_which_of_its_names_are_already_there() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    // The awkward folder this makes is not what this test is about; it is only the workspace's shape.
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    // Something that is already there, put there through the same door a person would use.
    let occupied = format!("{workspace}/there.txt");
    lab.core
        .write_text_file(&handle, &occupied, "already here\n")
        .await
        .expect("write the file that is already there");

    let here = tempdir().expect("a local folder to send from");
    let taken = here.path().join("there.txt");
    let free = here.path().join("fresh.txt");
    std::fs::write(&taken, "mine\n").expect("write the local file");
    std::fs::write(&free, "mine\n").expect("write the local file");

    let names = lab
        .core
        .upload_names_taken(&handle, &workspace, &[taken, free])
        .await
        .expect("check the names");

    assert_eq!(
        names,
        vec!["there.txt".to_owned()],
        "only the name that is taken is reported"
    );

    // Looking is not sending: what was there is untouched, and the free name was not created by
    // being checked.
    assert_eq!(
        lab.core.read_text_file(&handle, &occupied).await.expect("read it back"),
        "already here\n"
    );
    let listing = lab
        .core
        .list_files(&handle, &workspace)
        .await
        .expect("list the workspace");
    assert!(
        listing.entries.iter().all(|entry| entry.name != "fresh.txt"),
        "checking a name does not create it"
    );

    clear_workspace(&lab, &handle, &workspace).await;
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

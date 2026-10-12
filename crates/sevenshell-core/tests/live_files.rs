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

            // Waiting counts as unfinished: a paused transfer has not stopped, it has been asked to
            // hold, and a caller waiting for this one to settle would be waiting for a person.
            if !matches!(transfer.state, TransferState::Running | TransferState::Paused) {
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
        entry.mode.map(|mode| mode & 0o777),
        Some(0o644),
        "the permissions came with the listing"
    );
    assert_eq!(
        entry.owner.as_deref(),
        Some("labuser"),
        "the owner arrives as the name the server keeps for the number it sent"
    );
    assert!(
        entry
            .group
            .as_deref()
            .is_some_and(|group| !group.chars().all(|character| character.is_ascii_digit())),
        "the group is named too, rather than left as the number: {:?}",
        entry.group
    );

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

/// Text long enough to cross several chunks, numbered so a run says where two halves were joined.
fn payload(lines: usize) -> String {
    (0..lines).map(|line| format!("line {line:06}\n")).collect()
}

/// One row of the transfer list, which is what the interface draws.
async fn row(lab: &Lab, id: &str) -> sevenshell_types::TransferInfo {
    lab.core
        .transfers()
        .await
        .into_iter()
        .find(|transfer| transfer.id == id)
        .expect("the transfer is remembered")
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_transfer_that_failed_can_be_tried_again_from_the_row_it_left() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    // A directory that is not there yet: the upload fails when it opens the far side, before any
    // byte has been written anywhere.
    let missing = format!("{workspace}/not-yet");
    let scratch = tempdir().expect("temp dir");
    let source = scratch.path().join("retry.txt");
    std::fs::write(&source, "sent on the second try\n").expect("write the local file");

    let id = lab
        .core
        .start_upload(&handle, source, &missing)
        .await
        .expect("start the upload");
    let state = lab.settle(&id).await;
    assert!(
        matches!(state, TransferState::Failed { .. }),
        "an upload into a directory that does not exist cannot have worked: {state:?}"
    );

    // What a person fixes before pressing retry, and then the retry itself.
    lab.core
        .create_folder(&handle, &missing)
        .await
        .expect("create the directory that was missing");
    lab.core.retry_transfer(&id).await.expect("retry it");
    assert_eq!(lab.settle(&id).await, TransferState::Done, "the retry finished");

    assert_eq!(
        lab.core
            .read_text_file(&handle, &format!("{missing}/retry.txt"))
            .await
            .expect("read it back"),
        "sent on the second try\n"
    );

    // One row with the id it had: a retry is the same transfer rather than a second one beside it.
    let rows: Vec<_> = lab
        .core
        .transfers()
        .await
        .into_iter()
        .filter(|transfer| transfer.id == id)
        .collect();
    assert_eq!(rows.len(), 1, "the retry left one row, not two");

    clear_workspace(&lab, &handle, &workspace).await;
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_transfer_can_be_held_and_let_go_again() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    // Big enough that a hold is worth having: at this size the transfer is hundreds of chunks, and
    // holding it lands long before it is done.
    let text = payload(120_000);
    let remote = format!("{workspace}/held.txt");
    lab.core
        .write_text_file(&handle, &remote, &text)
        .await
        .expect("write the file to download");

    let scratch = tempdir().expect("temp dir");
    let target = scratch.path().join("held.txt");
    let older = "the version that was here before\n";
    std::fs::write(&target, older).expect("write the older local file");

    // The hold has to land while the transfer is still moving. On a machine fast enough to finish
    // before the next call is made, the attempt is simply made again — which is cheaper and more
    // honest than a test that waits for a transfer to be slow enough.
    let mut held = None;
    for attempt in 0..3 {
        let id = lab
            .core
            .start_download(&handle, &remote, target.clone())
            .await
            .expect("start the download");
        lab.core.pause_transfer(&id).await.expect("hold it");

        if row(&lab, &id).await.state == TransferState::Paused {
            held = Some(id);
            break;
        }
        assert!(
            attempt < 2,
            "three attempts in a row finished before they could be held"
        );
    }
    let id = held.expect("the transfer was held");

    // Held means held: the file that was already there is untouched, and the row says so.
    assert_eq!(
        std::fs::read_to_string(&target).expect("read the local file"),
        older,
        "a held transfer had already written over the file it is replacing"
    );

    lab.core.resume_transfer(&id).await.expect("let it go");
    assert_eq!(
        lab.settle(&id).await,
        TransferState::Done,
        "it finished once it was let go"
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("read the local file"),
        text,
        "the file it finished with is the whole file"
    );

    clear_workspace(&lab, &handle, &workspace).await;
}

/// A transfer is not the tab's: closing the browser that started it takes away the browsing.
///
/// The row has to still be there afterwards, because with the file panel gone the list is the only
/// place left that can show or stop it — and a 2 GB upload that died because somebody tidied a panel
/// away would be a worse answer than a transfer outliving the tab.
#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_transfer_keeps_going_when_the_browser_that_started_it_is_closed() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    let text = payload(60_000);
    let remote = format!("{workspace}/survives.txt");
    lab.core
        .write_text_file(&handle, &remote, &text)
        .await
        .expect("write the file to download");

    let scratch = tempdir().expect("temp dir");
    let target = scratch.path().join("survives.txt");
    let id = lab
        .core
        .start_download(&handle, &remote, target.clone())
        .await
        .expect("start the download");

    lab.core.close_files(&handle).await.expect("close the browser");

    // The browser is really closed, so the assertion below cannot pass by the close having quietly
    // done nothing.
    assert!(
        lab.core.list_files(&handle, ".").await.is_err(),
        "a closed browser is closed"
    );
    let listed = lab
        .core
        .transfers()
        .await
        .into_iter()
        .find(|transfer| transfer.id == id);
    assert!(
        listed.is_some(),
        "the transfer went with the panel that started it, and nobody can see or stop it now"
    );

    assert_eq!(
        lab.settle(&id).await,
        TransferState::Done,
        "it finished with nobody watching"
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("read the local file"),
        text,
        "closing the browser did not cost the transfer any of its bytes"
    );

    clear_workspace(&lab, &handle, &workspace).await;
}

/// The list the panel draws knows whose bytes each row is, and where its local end is.
///
/// This is what the two entries about the file are decided by, and neither answer may be a path: the
/// row says only that there is one.
#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_row_says_which_session_it_is_for_and_whether_it_has_a_local_end() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    let remote = format!("{workspace}/placed.txt");
    lab.core
        .write_text_file(&handle, &remote, "a few bytes\n")
        .await
        .expect("write the file to download");

    let scratch = tempdir().expect("temp dir");
    let target = scratch.path().join("placed.txt");
    let id = lab
        .core
        .start_download(&handle, &remote, target.clone())
        .await
        .expect("start the download");
    assert_eq!(lab.settle(&id).await, TransferState::Done, "the download finished");

    let info = row(&lab, &id).await;
    assert_eq!(
        info.session_id, lab.session_id,
        "the row names the session it moved bytes for"
    );
    assert!(info.has_local_path, "a download knows where it put the file");

    // What the two entries that use it are answered with: the path and whether it may be opened.
    let end = lab
        .core
        .transfer_local_end(&id)
        .await
        .expect("this transfer has a local end");
    assert_eq!(end.path, target);
    assert!(end.done, "a finished transfer may be opened");

    // And a packed upload has none: the archive that carried the files was deleted once read.
    let source = scratch.path().join("packed-not-placed.txt");
    std::fs::write(&source, "one of several\n").expect("write a local file");
    let packed = lab
        .core
        .start_packed_upload(&handle, vec![source], &workspace)
        .await
        .expect("start the packed upload");
    assert_eq!(lab.settle(&packed).await, TransferState::Done, "the upload finished");
    assert!(
        lab.core.transfer_local_end(&packed).await.is_none(),
        "a packed upload has no single local end to point at"
    );

    clear_workspace(&lab, &handle, &workspace).await;
}

/// Stopping keeps the row; removing takes it away. Clearing takes away only what has stopped.
#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn stopping_keeps_the_row_and_clearing_leaves_what_is_still_moving() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    // One that has finished, and one that is being held: the two states a "clear" has to tell apart.
    let small = format!("{workspace}/settled.txt");
    lab.core
        .write_text_file(&handle, &small, "done already\n")
        .await
        .expect("write the small file");
    let scratch = tempdir().expect("temp dir");
    let settled = lab
        .core
        .start_download(&handle, &small, scratch.path().join("settled.txt"))
        .await
        .expect("start the small download");
    assert_eq!(
        lab.settle(&settled).await,
        TransferState::Done,
        "the small one finished"
    );

    // Big enough that holding it lands while it is still moving, with the same retry as the hold
    // test uses: a machine fast enough to finish first is not a reason for a different assertion.
    let big = format!("{workspace}/held.txt");
    lab.core
        .write_text_file(&handle, &big, &payload(120_000))
        .await
        .expect("write the big file");
    let mut held = None;
    for attempt in 0..3 {
        let id = lab
            .core
            .start_download(&handle, &big, scratch.path().join("held.txt"))
            .await
            .expect("start the big download");
        lab.core.pause_transfer(&id).await.expect("hold it");

        if row(&lab, &id).await.state == TransferState::Paused {
            held = Some(id);
            break;
        }
        assert!(
            attempt < 2,
            "three attempts in a row finished before they could be held"
        );
    }
    let id = held.expect("the transfer was held");

    lab.core
        .clear_settled_transfers()
        .await
        .expect("clear what has stopped");
    let rows = lab.core.transfers().await;
    assert!(
        !rows.iter().any(|transfer| transfer.id == settled),
        "the finished transfer was still on the list after a clear"
    );
    assert!(
        rows.iter().any(|transfer| transfer.id == id),
        "a clear took away a transfer that is still going to happen"
    );

    // Stopping it leaves the row behind, saying what happened — which is not something a person
    // could tell if the row had simply vanished.
    lab.core.stop_transfer(&id).await.expect("stop it");
    assert_eq!(
        row(&lab, &id).await.state,
        TransferState::Cancelled,
        "a stopped transfer says it was cancelled"
    );

    // And removing is what takes the row away.
    lab.core.remove_transfer(&id).await.expect("remove it");
    assert!(
        !lab.core.transfers().await.iter().any(|transfer| transfer.id == id),
        "a removed transfer is gone from the list"
    );

    clear_workspace(&lab, &handle, &workspace).await;
}

#[tokio::test]
#[ignore = "needs the SSH lab with its fixtures: see deploy/lab/README.md"]
async fn a_directory_with_fifty_thousand_entries_is_read_in_reasonable_time() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;

    // The directory is made by the lab's own start-up script: fifty thousand files, because the
    // panel's acceptance is about a listing nobody would scroll through by hand.
    let started = Instant::now();
    let listing = lab
        .core
        .list_files(&handle, "big-directory")
        .await
        .expect("the big directory lists");
    let elapsed = started.elapsed();

    assert_eq!(listing.entries.len(), 50_000, "the fixture is the fixture");
    assert!(
        listing.entries.iter().any(|entry| entry.name == "entry-050000.txt"),
        "and it is the last of them, not fifty thousand of something else"
    );

    // Generous, because this is news rather than a budget: the reading is one SFTP listing and one
    // serialization, and what would make it slow is doing something per entry that should not be
    // done per entry. Seconds here would be a finding; milliseconds are what it costs.
    println!("fifty thousand entries arrived in {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(10),
        "reading fifty thousand entries took {elapsed:?}"
    );
}

/// How high this process's memory has been, in bytes, or `None` where the platform will not say.
///
/// Hand-rolled rather than dragged in as a dependency: it is a few lines of platform code on each of
/// the two platforms the lab runs on, and a test that watches memory should not be the reason another
/// crate joins the build. The *peak* is what matters — a transfer that held the whole file would show
/// it while it happened and never again.
fn peak_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
        let kibibytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        // No `return`: on the platform this arm is compiled for, it is the whole of the function.
        Some(kibibytes * 1024)
    }

    #[cfg(windows)]
    {
        use std::ffi::c_void;

        #[repr(C)]
        struct Counters {
            size: u32,
            page_faults: u32,
            peak_working_set: usize,
            working_set: usize,
            quota_peak_paged: usize,
            quota_paged: usize,
            quota_peak_nonpaged: usize,
            quota_nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
        }

        // `GetProcessMemoryInfo` lives in psapi rather than in the default libraries, which is what
        // the link attribute is for.
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut c_void;
            fn GetProcessMemoryInfo(process: *mut c_void, counters: *mut Counters, size: u32) -> i32;
        }

        let mut counters = Counters {
            size: size_of::<Counters>() as u32,
            page_faults: 0,
            peak_working_set: 0,
            working_set: 0,
            quota_peak_paged: 0,
            quota_paged: 0,
            quota_peak_nonpaged: 0,
            quota_nonpaged: 0,
            pagefile: 0,
            peak_pagefile: 0,
        };

        // SAFETY: the handle is this process's own pseudo-handle — nothing to close, nothing to
        // borrow — and the struct is the one `GetProcessMemoryInfo` documents for this size.
        let asked = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.size) };
        (asked != 0).then_some(counters.peak_working_set as u64)
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    {
        None
    }
}

/// The bytes of a file, summed in a way that notices a reordering as well as a missing byte.
///
/// Not a cryptographic digest: what this is asked is whether two files hold the same bytes, and the
/// answer has to be cheap enough to run over a gigabyte without becoming the thing being measured.
/// The chunks are numbered so that content and its position both matter.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn content_id(path: &std::path::Path) -> u64 {
    use std::io::Read;

    let mut file = std::fs::File::open(path).expect("open the file");
    let mut hash = FNV_OFFSET;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).expect("read the file");
        if read == 0 {
            return hash;
        }
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab, and moves a gigabyte: see deploy/lab/README.md"]
async fn a_gigabyte_crosses_without_being_held_in_memory() {
    let Some(lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    let (workspace, _) = packed_workspace(&lab, &handle).await;

    // A gigabyte of numbered chunks, written a megabyte at a time so that *this* never holds it
    // either: a peak measured afterwards would remember an allocation made during the setup.
    let scratch = tempdir().expect("temp dir");
    let source = scratch.path().join("gigabyte.bin");
    {
        use std::io::Write;

        let mut file = std::fs::File::create(&source).expect("create the file");
        for chunk in 0..1024_u32 {
            // Each megabyte is filled with a byte of its own, which is what makes a file that came
            // back rearranged a different file: a megabyte of zeroes would not.
            let filled = vec![(chunk % 251) as u8; 1024 * 1024];
            file.write_all(&filled).expect("write a megabyte");
        }
        file.sync_all().expect("flush it");
    }
    let expected = content_id(&source);

    let before = peak_memory();
    let started = Instant::now();

    let up = lab
        .core
        .start_upload(&handle, source.clone(), &workspace)
        .await
        .expect("start the upload");
    assert_eq!(lab.settle(&up).await, TransferState::Done, "the upload finished");

    let landed = lab
        .core
        .list_files(&handle, &workspace)
        .await
        .expect("list the workspace")
        .entries
        .into_iter()
        .find(|entry| entry.name == "gigabyte.bin")
        .expect("the uploaded file is there");
    assert_eq!(landed.size, 1024 * 1024 * 1024, "and it is a gigabyte");

    let downloaded = scratch.path().join("downloaded.bin");
    let down = lab
        .core
        .start_download(&handle, &format!("{workspace}/gigabyte.bin"), downloaded.clone())
        .await
        .expect("start the download");
    assert_eq!(lab.settle(&down).await, TransferState::Done, "the download finished");

    let elapsed = started.elapsed();
    assert_eq!(
        content_id(&downloaded),
        expected,
        "the gigabyte that came back is not the gigabyte that went"
    );

    // The claim this test exists for. The limit is not tight — a channel, a channel's buffers and the
    // async runtime cost tens of megabytes — but it is far below the file, which is the whole point:
    // what crosses is moved in chunks, and the file itself is never in memory on either side.
    let Some(after) = peak_memory() else {
        eprintln!("this platform will not say how much memory the process has used; the transfer itself is checked");
        return;
    };
    let peak = after.saturating_sub(before.unwrap_or(0));
    let mebibytes = peak / (1024 * 1024);
    let held = after / (1024 * 1024);
    println!(
        "two gigabytes in both directions took {elapsed:?}; the transfer added {mebibytes} MiB to a peak of {held} MiB"
    );

    assert!(
        peak < 256 * 1024 * 1024,
        "moving a gigabyte in each direction used {mebibytes} MiB of memory, which means something held the file"
    );

    let _ = std::fs::remove_file(&downloaded);
    clear_workspace(&lab, &handle, &workspace).await;
}

//! A container's files behind the file browser's own commands, against real Docker.
//!
//! Ignored by default: it needs an engine and the right to create containers. Run it with
//! `SEVENSHELL_TEST_DOCKER=1 cargo test -p sevenshell-core --test live_container_files -- --ignored`.
//! A machine without an engine, or without the small image this test pulls, skips with a sentence
//! rather than failing: that is an environment, not a defect in this crate.
//!
//! The protocol-level coverage of the same container API lives in `sevenshell-container`'s
//! `live_container_files`; this one is about the seam above it — one handle, the browser's own
//! commands, and the transfer list.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sevenshell_core::Core;
use sevenshell_error::ErrorCode;
use sevenshell_types::{
    AuthMethod, ContainerCreateRequest, ContainerRuntime, ContainerSettings, ContainerTransport, EntryKind, Protocol,
    Session, SessionAuth, SftpOutcome, TerminalSettings, TransferState,
};
use sevenshell_vault::Vault;
use tempfile::{TempDir, tempdir};

/// The image the test uses. Small, and it carries a shell, `tar`, `base64` and `chmod`.
const IMAGE: &str = "busybox:latest";
const NAME_PREFIX: &str = "7shell-live-wired-";

fn engine_is_reachable() -> bool {
    if std::env::var("SEVENSHELL_TEST_DOCKER").as_deref() != Ok("1") {
        eprintln!("skipped: set SEVENSHELL_TEST_DOCKER=1 to run the live container file test");
        return false;
    }
    let reachable = docker(&["version"]).is_some_and(|output| output.status.success());
    if !reachable {
        eprintln!("skipped: the Docker engine is not reachable from this process");
    }
    reachable
}

fn image_is_available() -> bool {
    if docker(&["image", "inspect", IMAGE]).is_some_and(|output| output.status.success()) {
        return true;
    }
    let pulled = docker(&["pull", IMAGE]).is_some_and(|output| output.status.success());
    if !pulled {
        eprintln!("skipped: {IMAGE} is not on this engine and could not be pulled");
    }
    pulled
}

fn docker(arguments: &[&str]) -> Option<std::process::Output> {
    std::process::Command::new("docker").args(arguments).output().ok()
}

/// A container is removed even when an assertion fails halfway through.
struct ContainerGuard {
    name: String,
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.name]);
    }
}

fn session(id: &str, name: &str, protocol: Protocol) -> Session {
    Session {
        id: id.to_owned(),
        name: name.to_owned(),
        group_id: None,
        protocol,
        host: String::new(),
        port: 0,
        user: String::new(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        desktop: sevenshell_types::DesktopSettings::default(),
        telnet: sevenshell_types::TelnetSettings::default(),
        serial: sevenshell_types::SerialSettings::default(),
        local: sevenshell_types::LocalSettings::default(),
        wsl: sevenshell_types::WslSettings::default(),
        container: ContainerSettings::default(),
        kubernetes: sevenshell_types::KubernetesSettings::default(),
        webdav: sevenshell_types::WebDavSettings::default(),
        ftp: sevenshell_types::FtpSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    }
}

/// Waits until the transfer is no longer moving, and answers why it stopped.
async fn settled(core: &Core, id: &str) -> Result<(), String> {
    for _ in 0..200 {
        if let Some(info) = core.transfers().await.into_iter().find(|info| info.id == id) {
            match info.state {
                TransferState::Done => return Ok(()),
                TransferState::Failed { detail } => return Err(format!("{id} failed: {detail}")),
                TransferState::Cancelled => return Err(format!("{id} was cancelled")),
                TransferState::Running | TransferState::Paused => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!("the transfer {id} never finished"))
}

#[tokio::test]
#[ignore = "needs Docker Desktop and SEVENSHELL_TEST_DOCKER=1"]
async fn a_container_browser_answers_every_file_command_and_one_transfer_each_way() {
    if !engine_is_reachable() || !image_is_available() {
        return;
    }

    let directory = tempdir().expect("a temporary folder");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create the vault");

    let mut saved = session("docker-local", "docker local", Protocol::Container);
    saved.container = ContainerSettings {
        runtime: ContainerRuntime::Docker,
        transport: ContainerTransport::Local,
        ssh_session: String::new(),
        namespace: String::new(),
    };
    vault.save_session(saved, None).expect("save the container session");

    let (core, _events) = Core::new(Arc::clone(&vault));
    let connection = core
        .container_connect("docker-local")
        .await
        .expect("a local runtime needs no network to be named");

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let name = format!("{NAME_PREFIX}{}-{stamp}", std::process::id());
    let create = ContainerCreateRequest {
        image: IMAGE.to_owned(),
        name: name.clone(),
        ports: Vec::new(),
        volumes: Vec::new(),
        env: Vec::new(),
        restart_policy: "no".to_owned(),
        command: "sleep 600".to_owned(),
        interactive: false,
        tty: false,
    };
    core.container_create(&connection, &create)
        .await
        .expect("the container should be created");
    let guard = ContainerGuard { name: name.clone() };

    let outcome = browsing(&core, &connection, &name, &directory).await;

    core.container_disconnect(&connection).await;
    drop(guard);
    if let Err(message) = outcome {
        panic!("{message}");
    }
}

/// The browser's own commands, in the order a person would use them.
async fn browsing(core: &Core, connection: &str, name: &str, directory: &TempDir) -> Result<(), String> {
    // `docker run -d` answers when the container is started, and the first exec may still race it.
    let mut opened = None;
    let mut refusal = None;
    for _ in 0..40 {
        match core.open_container_files(connection, name, "/").await {
            Ok(SftpOutcome::Opened {
                handle,
                session_id,
                listing,
                packed,
            }) => {
                opened = Some((handle, session_id, listing, packed));
                break;
            }
            Ok(other) => return Err(format!("opening answered {other:?}")),
            Err(error) => {
                if error.code() == ErrorCode::ContainerCommandFailed {
                    refusal = Some(error);
                    tokio::time::sleep(Duration::from_millis(250)).await;
                } else {
                    return Err(format!("opening failed: {error:?}"));
                }
            }
        }
    }
    let Some((handle, session_id, root, packed)) = opened else {
        return Err(format!("the container never answered a listing: {refusal:?}"));
    };

    if root.path != "/" {
        return Err(format!("/ resolved to {} instead of itself", root.path));
    }
    if session_id != connection {
        return Err(format!(
            "the browser belongs to {session_id} rather than the connection {connection} it was opened on"
        ));
    }
    if !packed {
        return Err("a container browser has to offer packed transfers".to_owned());
    }
    if !root
        .entries
        .iter()
        .any(|entry| entry.name == "etc" && entry.kind == EntryKind::Directory)
    {
        return Err(format!("the root listing has no /etc directory: {root:?}"));
    }

    let tmp = core
        .list_files(&handle, "/tmp")
        .await
        .map_err(|error| format!("listing /tmp failed: {error:?}"))?;
    if tmp.path != "/tmp" {
        return Err(format!("/tmp resolved to {}", tmp.path));
    }

    core.create_folder(&handle, "/tmp/wired")
        .await
        .map_err(|error| format!("create_folder failed: {error:?}"))?;
    core.write_text_file(&handle, "/tmp/wired/hello.txt", "hello from the browser\n")
        .await
        .map_err(|error| format!("write_text_file failed: {error:?}"))?;
    let text = core
        .read_text_file(&handle, "/tmp/wired/hello.txt")
        .await
        .map_err(|error| format!("read_text_file failed: {error:?}"))?;
    if text != "hello from the browser\n" {
        return Err(format!("the editor read back {text:?}"));
    }

    core.create_file(&handle, "/tmp/wired/empty.txt")
        .await
        .map_err(|error| format!("create_file failed: {error:?}"))?;
    let listing = core
        .list_files(&handle, "/tmp/wired")
        .await
        .map_err(|error| format!("listing the new folder failed: {error:?}"))?;
    let empty = listing
        .entries
        .iter()
        .find(|entry| entry.name == "empty.txt")
        .ok_or_else(|| format!("the created file is not listed: {listing:?}"))?;
    if empty.size != 0 {
        return Err(format!("a created file is not empty: {empty:?}"));
    }

    core.set_permissions(&handle, "/tmp/wired/hello.txt", 0o600)
        .await
        .map_err(|error| format!("set_permissions failed: {error:?}"))?;
    let listing = core
        .list_files(&handle, "/tmp/wired")
        .await
        .map_err(|error| format!("listing after chmod failed: {error:?}"))?;
    let hello = listing
        .entries
        .iter()
        .find(|entry| entry.name == "hello.txt")
        .ok_or_else(|| format!("hello.txt is gone after chmod: {listing:?}"))?;
    if hello.mode != Some(0o600) {
        return Err(format!("the mode did not change: {hello:?}"));
    }

    core.rename_entry(&handle, "/tmp/wired/hello.txt", "/tmp/wired/renamed.txt")
        .await
        .map_err(|error| format!("rename failed: {error:?}"))?;
    let listing = core
        .list_files(&handle, "/tmp/wired")
        .await
        .map_err(|error| format!("listing after rename failed: {error:?}"))?;
    if listing.entries.iter().any(|entry| entry.name == "hello.txt") {
        return Err(format!("the old name survived the rename: {listing:?}"));
    }
    if !listing.entries.iter().any(|entry| entry.name == "renamed.txt") {
        return Err(format!("the new name is not there: {listing:?}"));
    }

    // A rename onto a name that is taken is a refusal, and the file that was there keeps its bytes:
    // `mv` would otherwise replace it, and the interface never asked anybody about that.
    core.write_text_file(&handle, "/tmp/wired/keep.txt", "keep me\n")
        .await
        .map_err(|error| format!("writing the file to keep failed: {error:?}"))?;
    let error = core
        .rename_entry(&handle, "/tmp/wired/renamed.txt", "/tmp/wired/keep.txt")
        .await
        .expect_err("a name that is taken is not a rename");
    if error.code() != ErrorCode::SftpExists {
        return Err(format!("the refusal carried {:?} instead of SFTP-EXISTS", error.code()));
    }
    let kept = core
        .read_text_file(&handle, "/tmp/wired/keep.txt")
        .await
        .map_err(|error| format!("reading the kept file failed: {error:?}"))?;
    if kept != "keep me\n" {
        return Err(format!("the refused rename replaced its target: {kept:?}"));
    }

    // Copy out of the folder, then let the same paste ask about the name it would replace.
    let copied = core
        .paste_entries(&handle, &["/tmp/wired/renamed.txt".to_owned()], "/tmp", false, false)
        .await
        .map_err(|error| format!("paste failed: {error:?}"))?;
    if !copied.conflicts.is_empty() || copied.results.len() != 1 || !copied.results[0].ok {
        return Err(format!("the first paste did not just copy: {copied:?}"));
    }
    let asked = core
        .paste_entries(&handle, &["/tmp/wired/renamed.txt".to_owned()], "/tmp", false, false)
        .await
        .map_err(|error| format!("the second paste failed: {error:?}"))?;
    if asked.conflicts != vec!["/tmp/renamed.txt".to_owned()] || !asked.results.is_empty() {
        return Err(format!("the taken name was not asked about: {asked:?}"));
    }
    let replaced = core
        .paste_entries(&handle, &["/tmp/wired/renamed.txt".to_owned()], "/tmp", false, true)
        .await
        .map_err(|error| format!("the overwriting paste failed: {error:?}"))?;
    if replaced.results.len() != 1 || !replaced.results[0].ok {
        return Err(format!("answering yes did not replace the file: {replaced:?}"));
    }

    // A move is the same channel with the container's own `mv`.
    core.delete_entry(&handle, "/tmp/wired/renamed.txt", EntryKind::File)
        .await
        .map_err(|error| format!("deleting the original failed: {error:?}"))?;
    let moved = core
        .paste_entries(&handle, &["/tmp/renamed.txt".to_owned()], "/tmp/wired", true, false)
        .await
        .map_err(|error| format!("the move failed: {error:?}"))?;
    if moved.results.len() != 1 || !moved.results[0].ok {
        return Err(format!("the move did not happen: {moved:?}"));
    }
    let listing = core
        .list_files(&handle, "/tmp/wired")
        .await
        .map_err(|error| format!("listing after the move failed: {error:?}"))?;
    if !listing.entries.iter().any(|entry| entry.name == "renamed.txt") {
        return Err(format!("the moved file is not in its new folder: {listing:?}"));
    }

    // An upload through the transfer list, then the same file read back by the editor.
    let local = directory.path().join("uploaded.txt");
    std::fs::write(&local, "uploaded from this machine\n").map_err(|error| error.to_string())?;
    let uploaded = core
        .start_upload(&handle, local.clone(), "/tmp/wired")
        .await
        .map_err(|error| format!("start_upload failed: {error:?}"))?;
    settled(core, &uploaded).await?;
    let text = core
        .read_text_file(&handle, "/tmp/wired/uploaded.txt")
        .await
        .map_err(|error| format!("reading the uploaded file failed: {error:?}"))?;
    if text != "uploaded from this machine\n" {
        return Err(format!("the uploaded file carries {text:?}"));
    }

    // A download through the transfer list, with the local end the row points at.
    let destination = directory.path().join("downloaded.txt");
    let downloaded = core
        .start_download(&handle, "/tmp/wired/renamed.txt", destination.clone())
        .await
        .map_err(|error| format!("start_download failed: {error:?}"))?;
    settled(core, &downloaded).await?;
    let landed = core
        .transfer_local_end(&downloaded)
        .await
        .ok_or_else(|| "the download row has no local end".to_owned())?;
    if !landed.done || landed.path != destination {
        return Err(format!("the finished download points at {:?}", landed.path));
    }
    let bytes = std::fs::read_to_string(&destination).map_err(|error| error.to_string())?;
    if bytes != "hello from the browser\n" {
        return Err(format!("the downloaded file carries {bytes:?}"));
    }

    // Deleting, then reading the folder again, is what the panel does after every such command.
    core.delete_entry(&handle, "/tmp/wired/uploaded.txt", EntryKind::File)
        .await
        .map_err(|error| format!("delete failed: {error:?}"))?;
    let listing = core
        .list_files(&handle, "/tmp/wired")
        .await
        .map_err(|error| format!("listing after delete failed: {error:?}"))?;
    if listing.entries.iter().any(|entry| entry.name == "uploaded.txt") {
        return Err(format!("the deleted file is still listed: {listing:?}"));
    }

    core.delete_entry(&handle, "/tmp/wired", EntryKind::Directory)
        .await
        .map_err(|error| format!("removing the folder failed: {error:?}"))?;

    core.close_files(&handle)
        .await
        .map_err(|error| format!("close_files failed: {error:?}"))?;
    let after = core
        .list_files(&handle, "/")
        .await
        .expect_err("a closed browser has no listing");
    if after.code() != ErrorCode::AppInternal {
        return Err(format!("a closed handle answered {:?}", after.code()));
    }

    Ok(())
}

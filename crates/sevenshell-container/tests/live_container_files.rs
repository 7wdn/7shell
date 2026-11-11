//! Real Docker Desktop, files inside a container.
//!
//! Ignored by default: it needs an engine and the right to create containers. Run it with
//! `SEVENSHELL_TEST_DOCKER=1 cargo test -p sevenshell-container --test live_container_files --
//! --ignored`. A machine without an engine, or without the small image this test pulls, skips with
//! a sentence rather than failing: that is an environment, not a defect in this crate.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sevenshell_container::{ContainerFiles, ContainerManager, Runner};
use sevenshell_types::{ContainerCreateRequest, ContainerRuntime, EntryKind};

/// The image the test uses. Small, and it carries a shell, `tar` and `chmod`.
const IMAGE: &str = "busybox:latest";
const NAME_PREFIX: &str = "7shell-live-files-";

fn engine_is_reachable() -> bool {
    if std::env::var("SEVENSHELL_TEST_DOCKER").as_deref() != Ok("1") {
        eprintln!("skipped: set SEVENSHELL_TEST_DOCKER=1 to run the live container file test");
        return false;
    }
    let reachable = std::process::Command::new("docker")
        .arg("version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !reachable {
        eprintln!("skipped: the Docker engine is not reachable from this process");
    }
    reachable
}

fn image_is_available() -> bool {
    let inspected = std::process::Command::new("docker")
        .args(["image", "inspect", IMAGE])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if inspected {
        return true;
    }
    let pulled = std::process::Command::new("docker")
        .args(["pull", IMAGE])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !pulled {
        eprintln!("skipped: {IMAGE} is not on this engine and could not be pulled");
    }
    pulled
}

/// A container is removed even when an assertion fails halfway through.
struct ContainerGuard {
    name: String,
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &self.name])
            .output();
    }
}

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("7shell-live-files-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&path).expect("a temporary folder");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
#[ignore = "needs Docker Desktop and SEVENSHELL_TEST_DOCKER=1"]
async fn a_real_container_is_listed_written_uploaded_and_downloaded() {
    if !engine_is_reachable() || !image_is_available() {
        return;
    }

    let manager = ContainerManager::new();
    let connection = manager.new_id();
    manager
        .connect(connection.clone(), ContainerRuntime::Docker, "", Runner::Local)
        .expect("the connection settings should be accepted");
    let provider = manager.provider(&connection).expect("the connection should be open");

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let name = format!("{NAME_PREFIX}{}-{stamp}", std::process::id());
    let request = ContainerCreateRequest {
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
    provider
        .create(&request)
        .await
        .expect("the container should be created");
    let _guard = ContainerGuard { name: name.clone() };

    let outcome: Result<(), String> = async {
        let files = ContainerFiles::new(ContainerRuntime::Docker, "", &name, Runner::Local)
            .map_err(|error| format!("the file handle was refused: {error:?}"))?;

        // `docker run -d` returns when the container is started; the first exec may still race it.
        let mut root = None;
        for _ in 0..40 {
            match files.list("/").await {
                Ok(listing) => {
                    root = Some(listing);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
        let root = root.ok_or_else(|| "the container never answered a listing".to_owned())?;
        if root.path != "/" {
            return Err(format!("/ resolved to {} instead of itself", root.path));
        }
        if !root
            .entries
            .iter()
            .any(|entry| entry.name == "etc" && entry.kind == EntryKind::Directory)
        {
            return Err(format!("the root listing has no /etc directory: {root:?}"));
        }

        files
            .create_dir("/tmp/files lab")
            .await
            .map_err(|error| format!("create_dir failed: {error:?}"))?;
        let hello = "/tmp/files lab/hello world.txt";
        files
            .write_text(hello, "first line\n")
            .await
            .map_err(|error| format!("write_text failed: {error:?}"))?;

        let listing = files
            .list("/tmp/files lab")
            .await
            .map_err(|error| format!("listing the new folder failed: {error:?}"))?;
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.name == "hello world.txt")
            .ok_or_else(|| format!("the written file is not in the listing: {listing:?}"))?;
        if entry.kind != EntryKind::File || entry.size != 11 || entry.mode != Some(0o644) {
            return Err(format!("the written file is not what was written: {entry:?}"));
        }
        let text = files
            .read_text(hello)
            .await
            .map_err(|error| format!("read_text failed: {error:?}"))?;
        if text != "first line\n" {
            return Err(format!("read_text answered {text:?}"));
        }

        // A name with a quote is the one that proves the script quoting against a real shell.
        let quoted = "/tmp/files lab/it's a file.txt";
        files
            .write_text(quoted, "quoted\n")
            .await
            .map_err(|error| format!("writing a quoted name failed: {error:?}"))?;
        let text = files
            .read_text(quoted)
            .await
            .map_err(|error| format!("reading a quoted name failed: {error:?}"))?;
        if text != "quoted\n" {
            return Err(format!("a quoted name answered {text:?}"));
        }

        files
            .set_mode(hello, 0o600)
            .await
            .map_err(|error| format!("set_mode failed: {error:?}"))?;
        let listing = files
            .list("/tmp/files lab")
            .await
            .map_err(|error| format!("listing after chmod failed: {error:?}"))?;
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.name == "hello world.txt")
            .expect("the file should still be listed");
        if entry.mode != Some(0o600) {
            return Err(format!("the mode did not change: {entry:?}"));
        }

        files
            .rename(hello, "/tmp/files lab/renamed.txt")
            .await
            .map_err(|error| format!("rename failed: {error:?}"))?;
        files
            .copy("/tmp/files lab/renamed.txt", "/tmp/files lab/copied.txt")
            .await
            .map_err(|error| format!("copy failed: {error:?}"))?;
        files
            .create_file("/tmp/files lab/empty.txt")
            .await
            .map_err(|error| format!("create_file failed: {error:?}"))?;
        let listing = files
            .list("/tmp/files lab")
            .await
            .map_err(|error| format!("listing after rename and copy failed: {error:?}"))?;
        for expected in ["renamed.txt", "copied.txt", "empty.txt", "it's a file.txt"] {
            if !listing.entries.iter().any(|entry| entry.name == expected) {
                return Err(format!(
                    "{expected} is missing after the metadata commands: {listing:?}"
                ));
            }
        }
        let empty = listing
            .entries
            .iter()
            .find(|entry| entry.name == "empty.txt")
            .expect("the empty file should be listed");
        if empty.size != 0 {
            return Err(format!("a created file is not empty: {empty:?}"));
        }

        files
            .remove("/tmp/files lab/copied.txt", false)
            .await
            .map_err(|error| format!("removing a file failed: {error:?}"))?;
        let listing = files
            .list("/tmp/files lab")
            .await
            .map_err(|error| format!("listing after removal failed: {error:?}"))?;
        if listing.entries.iter().any(|entry| entry.name == "copied.txt") {
            return Err(format!("the removed file is still listed: {listing:?}"));
        }

        // Upload a tree and read it back through a second listing.
        let local = TempTree::new();
        let site = local.path().join("site");
        std::fs::create_dir_all(site.join("assets")).map_err(|error| error.to_string())?;
        std::fs::write(site.join("index.html"), "<h1>hi</h1>\n").map_err(|error| error.to_string())?;
        std::fs::write(site.join("assets/app.css"), "body {}\n").map_err(|error| error.to_string())?;
        files
            .upload(&site, "/tmp/files lab")
            .await
            .map_err(|error| format!("upload failed: {error:?}"))?;
        let text = files
            .read_text("/tmp/files lab/site/assets/app.css")
            .await
            .map_err(|error| format!("reading an uploaded file failed: {error:?}"))?;
        if text != "body {}\n" {
            return Err(format!("an uploaded file answered {text:?}"));
        }

        let down = TempTree::new();
        let destination = down.path().join("site");
        files
            .download("/tmp/files lab/site", &destination)
            .await
            .map_err(|error| format!("download failed: {error:?}"))?;
        let index = std::fs::read_to_string(destination.join("index.html"))
            .map_err(|error| format!("the downloaded index.html is missing: {error}"))?;
        if index != "<h1>hi</h1>\n" {
            return Err(format!("the downloaded index.html carries {index:?}"));
        }
        let css = std::fs::read_to_string(destination.join("assets/app.css"))
            .map_err(|error| format!("the downloaded app.css is missing: {error}"))?;
        if css != "body {}\n" {
            return Err(format!("the downloaded app.css carries {css:?}"));
        }

        // A stopped container cannot run exec, so it cannot be browsed through this channel. That is
        // the documented limit of the seam; the assertion keeps it from silently changing.
        provider
            .action(&name, "stop")
            .await
            .map_err(|error| format!("stop failed: {error:?}"))?;
        if files.list("/").await.is_ok() {
            return Err("a stopped container pretended to answer a listing".to_owned());
        }
        provider
            .action(&name, "start")
            .await
            .map_err(|error| format!("start failed: {error:?}"))?;
        for _ in 0..40 {
            match files.list("/").await {
                Ok(_) => break,
                Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }

        files
            .remove("/tmp/files lab", true)
            .await
            .map_err(|error| format!("removing the folder failed: {error:?}"))?;
        if files.list("/tmp/files lab").await.is_ok() {
            return Err("the removed folder is still listable".to_owned());
        }
        Ok(())
    }
    .await;

    let _ = provider.action(&name, "remove").await;
    manager.remove(&connection);

    if let Err(message) = outcome {
        panic!("{message}");
    }
}

/// A tree past the 32 MiB ceiling this channel used to refuse, through a real engine.
///
/// The upload is packed while the container reads it and the download is written while the
/// container packs it, so what this proves is that neither direction is bounded by an amount of
/// memory: the tree is 40 MiB, which used to be a refusal before the first byte moved.
#[tokio::test]
#[ignore = "needs Docker Desktop and SEVENSHELL_TEST_DOCKER=1"]
async fn a_tree_past_the_old_transfer_ceiling_round_trips_through_a_container() {
    if !engine_is_reachable() || !image_is_available() {
        return;
    }

    let manager = ContainerManager::new();
    let connection = manager.new_id();
    manager
        .connect(connection.clone(), ContainerRuntime::Docker, "", Runner::Local)
        .expect("the connection settings should be accepted");
    let provider = manager.provider(&connection).expect("the connection should be open");

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let name = format!("{NAME_PREFIX}{}-{stamp}", std::process::id());
    let request = ContainerCreateRequest {
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
    provider
        .create(&request)
        .await
        .expect("the container should be created");
    let _guard = ContainerGuard { name: name.clone() };

    const BIG_BYTES: usize = 36 * 1024 * 1024;
    let payload: Vec<u8> = (0..BIG_BYTES).map(|index| (index % 251) as u8).collect();

    let outcome: Result<(), String> = async {
        let files = ContainerFiles::new(ContainerRuntime::Docker, "", &name, Runner::Local)
            .map_err(|error| format!("the file handle was refused: {error:?}"))?;

        // The first exec can race the container's start.
        for _ in 0..40 {
            if files.list("/").await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        let local = TempTree::new();
        let bulk = local.path().join("bulk");
        std::fs::create_dir_all(bulk.join("nested")).map_err(|error| error.to_string())?;
        std::fs::write(bulk.join("big.bin"), &payload).map_err(|error| error.to_string())?;
        std::fs::write(bulk.join("nested/notes.txt"), "a small file beside a large one\n")
            .map_err(|error| error.to_string())?;

        files
            .upload(&bulk, "/tmp")
            .await
            .map_err(|error| format!("uploading 40 MiB failed: {error:?}"))?;

        let listing = files
            .list("/tmp/bulk")
            .await
            .map_err(|error| format!("listing the uploaded tree failed: {error:?}"))?;
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.name == "big.bin")
            .ok_or_else(|| format!("big.bin is not in the listing: {listing:?}"))?;
        if entry.size != BIG_BYTES as u64 {
            return Err(format!("the container holds {} bytes, not {BIG_BYTES}", entry.size));
        }

        let down = TempTree::new();
        let destination = down.path().join("bulk");
        files
            .download("/tmp/bulk", &destination)
            .await
            .map_err(|error| format!("downloading 40 MiB failed: {error:?}"))?;
        let received = std::fs::read(destination.join("big.bin"))
            .map_err(|error| format!("the downloaded big.bin is missing: {error}"))?;
        if received != payload {
            return Err(format!(
                "the downloaded file is {} bytes and does not match the {} uploaded",
                received.len(),
                payload.len()
            ));
        }
        let notes = std::fs::read_to_string(destination.join("nested/notes.txt"))
            .map_err(|error| format!("the downloaded notes.txt is missing: {error}"))?;
        if notes != "a small file beside a large one\n" {
            return Err(format!("the downloaded notes.txt carries {notes:?}"));
        }

        files
            .remove("/tmp/bulk", true)
            .await
            .map_err(|error| format!("removing the uploaded tree failed: {error:?}"))?;
        Ok(())
    }
    .await;

    let _ = provider.action(&name, "remove").await;
    manager.remove(&connection);

    if let Err(message) = outcome {
        panic!("{message}");
    }
}

/// The lab's SSH connection, or a note that this test cannot run.
///
/// A session of its own rather than a `Core`: this crate owns the runner, and what is under test is
/// the channel, not the orchestration above it.
async fn lab_runner() -> Option<Runner> {
    use sevenshell_ssh::{Credential, OpenRequest, Purpose, Route, SshSession, Target, TerminalSize, probe_host_key};
    use std::sync::Arc;

    let (Some(host), Some(port), Some(user), Some(password)) = (
        std::env::var("SEVENSHELL_TEST_HOST").ok(),
        std::env::var("SEVENSHELL_TEST_PORT").ok(),
        std::env::var("SEVENSHELL_TEST_USER").ok(),
        std::env::var("SEVENSHELL_TEST_PASSWORD").ok(),
    ) else {
        eprintln!("skipped: the SSH lab is not configured (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return None;
    };
    let port = port.parse::<u16>().ok()?;
    let target = Target { host, port, user };
    let presented = probe_host_key(&target.host, target.port, Duration::from_secs(20))
        .await
        .expect("probe the lab host key");
    let request = OpenRequest {
        target,
        credential: Credential::Password(password),
        terminal_type: String::new(),
        keepalive_seconds: 0,
        forward_agent: false,
        timeout: Duration::from_secs(20),
        expected_host_key: presented,
        size: TerminalSize::default(),
        route: Route::Direct,
        purpose: Purpose::Carrier,
    };
    let (events, _unused) = tokio::sync::mpsc::channel(1);
    let session = SshSession::open(request, events).await.expect("connect to the lab");
    Some(Runner::Ssh(Arc::new(session)))
}

/// A file streamed into `docker exec -i` and back out over an SSH channel.
///
/// The lab's runtime is a `docker:dind` service the sshd container reaches, so this is the whole
/// remote path: quoted command line, exec channel, streaming input, streaming output.
#[tokio::test]
#[ignore = "needs the SSH lab and its dind runtime"]
async fn a_streamed_file_crosses_an_ssh_channel_to_a_real_engine() {
    let Some(runner) = lab_runner().await else {
        return;
    };
    let provider = sevenshell_container::Provider::new(ContainerRuntime::Docker, "", runner.clone())
        .expect("the connection settings should be accepted");

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let name = format!("{NAME_PREFIX}ssh-{}-{stamp}", std::process::id());
    let request = ContainerCreateRequest {
        image: "busybox:1.36".to_owned(),
        name: name.clone(),
        ports: Vec::new(),
        volumes: Vec::new(),
        env: Vec::new(),
        restart_policy: "no".to_owned(),
        command: "sleep 300".to_owned(),
        interactive: false,
        tty: false,
    };
    provider
        .create(&request)
        .await
        .expect("the container should be created");
    let _guard = ContainerGuard { name: name.clone() };

    let outcome: Result<(), String> = async {
        let files = ContainerFiles::new(ContainerRuntime::Docker, "", &name, runner)
            .map_err(|error| format!("the file handle was refused: {error:?}"))?;
        for _ in 0..40 {
            if files.list("/").await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        // Two megabytes: past a channel window, so the write and the read really overlap.
        let payload: Vec<u8> = (0..2 * 1024 * 1024).map(|index| (index % 241) as u8).collect();
        let local = TempTree::new();
        let source = local.path().join("payload.bin");
        std::fs::write(&source, &payload).map_err(|error| error.to_string())?;
        files
            .upload(&source, "/tmp")
            .await
            .map_err(|error| format!("upload over SSH failed: {error:?}"))?;

        let down = TempTree::new();
        let destination = down.path().join("payload.bin");
        files
            .download("/tmp/payload.bin", &destination)
            .await
            .map_err(|error| format!("download over SSH failed: {error:?}"))?;
        let received = std::fs::read(&destination).map_err(|error| error.to_string())?;
        if received != payload {
            return Err(format!(
                "the file came back as {} bytes and does not match the {} sent",
                received.len(),
                payload.len()
            ));
        }

        files
            .remove("/tmp/payload.bin", false)
            .await
            .map_err(|error| format!("removing the file failed: {error:?}"))?;
        Ok(())
    }
    .await;

    let _ = provider.action(&name, "remove").await;
    if let Err(message) = outcome {
        panic!("{message}");
    }
}

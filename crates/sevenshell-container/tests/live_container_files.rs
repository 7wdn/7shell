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

/// The image the test uses. Small, and it carries a shell, `tar`, `base64` and `chmod`.
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

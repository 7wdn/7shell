//! A pod's files behind the file browser's own commands, against a real cluster.
//!
//! Ignored by default: `deploy/lab/kubernetes` starts the one-node k3s the fixtures live on, and
//! `SEVENSHELL_TEST_KUBECONFIG` names its kubeconfig. `lab-shell` in `sevenshell-lab` is a busybox
//! that sleeps, so it has a shell and a `tar` but no files of its own to disturb.
//!
//! ```text
//! SEVENSHELL_TEST_KUBECONFIG=.scratch/k3s/kubeconfig-original.yaml \
//!   cargo test -p sevenshell-core --test live_pod_files -- --ignored --test-threads=1
//! ```
//!
//! What this covers that a fake server cannot: the kubelet actually closing (or not closing) a
//! non-terminal exec, `tar` inside the container deciding when its input has ended, and a real API
//! server authorising a certificate-based client on a second kind of channel.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, EntryKind, K8sExecRequest, KubeconfigSource, KubernetesSettings, Protocol, Session, SessionAuth,
    SftpOutcome, TerminalSettings, TransferState,
};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};

const NAMESPACE: &str = "sevenshell-lab";
const POD: &str = "lab-shell";
const CONTAINER: &str = "lab-shell";

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// A core holding one imported kubeconfig, the way the interface would have saved it.
struct Cluster {
    _directory: TempDir,
    core: Arc<Core>,
    session_id: String,
}

fn cluster() -> Option<Cluster> {
    let Some(path) = environment("SEVENSHELL_TEST_KUBECONFIG") else {
        eprintln!("the Kubernetes lab is not configured; set SEVENSHELL_TEST_KUBECONFIG");
        return None;
    };
    let kubeconfig = std::fs::read(&path).expect("read the kubeconfig");

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    vault
        .save_session(
            Session {
                id: "lab-cluster".to_owned(),
                name: "lab cluster".to_owned(),
                group_id: None,
                protocol: Protocol::Kubernetes,
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
                container: sevenshell_types::ContainerSettings::default(),
                kubernetes: KubernetesSettings {
                    source: KubeconfigSource::Imported,
                    context: None,
                    namespace: NAMESPACE.to_owned(),
                    via_ssh: None,
                },
                webdav: sevenshell_types::WebDavSettings::default(),
                ftp: sevenshell_types::FtpSettings::default(),
                jump: Vec::new(),
                forwards: Vec::new(),
                updated_at: 0,
            },
            Some(SessionSecret::Kubeconfig(
                String::from_utf8(kubeconfig).expect("kubeconfig is text"),
            )),
        )
        .expect("save session");

    let (core, _events) = Core::new(Arc::clone(&vault));
    Some(Cluster {
        _directory: directory,
        core,
        session_id: "lab-cluster".to_owned(),
    })
}

/// Waits until the transfer is no longer moving, and answers why it stopped.
async fn settled(core: &Core, id: &str) -> Result<(), String> {
    for _ in 0..300 {
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
#[ignore = "needs a real cluster; set SEVENSHELL_TEST_KUBECONFIG"]
async fn a_pod_browser_lists_reads_writes_and_carries_a_tree_both_ways() {
    let Some(lab) = cluster() else {
        return;
    };
    let core = Arc::clone(&lab.core);
    let connection = core.k8s_connect(&lab.session_id).await.expect("connect");

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let root = format!("/tmp/7shell-pod-{}-{stamp}", std::process::id());

    let outcome = browsing(&core, &connection, &root).await;

    core.k8s_disconnect(&connection);
    if let Err(message) = outcome {
        panic!("{message}");
    }
}

/// The browser's own commands, in the order a person would use them.
async fn browsing(core: &Core, connection: &str, root: &str) -> Result<(), String> {
    let opened = match core.open_pod_files(connection, NAMESPACE, POD, CONTAINER, "/").await {
        Ok(opened) => opened,
        Err(error) => return Err(format!("opening the pod browser failed: {error:?}")),
    };
    let SftpOutcome::Opened {
        handle,
        session_id,
        listing,
        packed,
    } = opened
    else {
        return Err(format!("opening answered {opened:?}"));
    };
    if session_id != connection {
        return Err(format!(
            "the browser belongs to {session_id} rather than the connection {connection} it was opened on"
        ));
    }
    if !packed {
        return Err("a pod browser has to offer packed transfers".to_owned());
    }
    if listing.path != "/" {
        return Err(format!("/ resolved to {} instead of itself", listing.path));
    }
    if !listing
        .entries
        .iter()
        .any(|entry| entry.name == "bin" && entry.kind == EntryKind::Directory)
    {
        return Err(format!("the root listing has no /bin directory: {listing:?}"));
    }

    let tmp = core
        .list_files(&handle, "/tmp")
        .await
        .map_err(|error| format!("listing /tmp failed: {error:?}"))?;
    if tmp.path != "/tmp" {
        return Err(format!("/tmp resolved to {}", tmp.path));
    }

    core.create_folder(&handle, root)
        .await
        .map_err(|error| format!("create_folder failed: {error:?}"))?;
    core.write_text_file(&handle, &format!("{root}/hello.txt"), "hello from the pod\n")
        .await
        .map_err(|error| format!("write_text_file failed: {error:?}"))?;
    let text = core
        .read_text_file(&handle, &format!("{root}/hello.txt"))
        .await
        .map_err(|error| format!("read_text_file failed: {error:?}"))?;
    if text != "hello from the pod\n" {
        return Err(format!("the editor read back {text:?}"));
    }

    core.create_file(&handle, &format!("{root}/empty.txt"))
        .await
        .map_err(|error| format!("create_file failed: {error:?}"))?;
    let listing = core
        .list_files(&handle, root)
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

    core.rename_entry(&handle, &format!("{root}/hello.txt"), &format!("{root}/renamed.txt"))
        .await
        .map_err(|error| format!("rename failed: {error:?}"))?;
    let listing = core
        .list_files(&handle, root)
        .await
        .map_err(|error| format!("listing after rename failed: {error:?}"))?;
    if listing.entries.iter().any(|entry| entry.name == "hello.txt") {
        return Err(format!("the old name survived the rename: {listing:?}"));
    }

    // A tree goes up as one archive and comes back as one: the same transport the container
    // browser uses, with the pod's `tar` on the far side of it.
    let local = tempdir().map_err(|error| error.to_string())?;
    let tree = local.path().join("site");
    std::fs::create_dir_all(tree.join("assets")).map_err(|error| error.to_string())?;
    std::fs::write(tree.join("index.html"), "<h1>pod</h1>\n").map_err(|error| error.to_string())?;
    std::fs::write(tree.join("assets/app.css"), "body {}\n").map_err(|error| error.to_string())?;

    let uploaded = core
        .start_upload(&handle, tree.clone(), root)
        .await
        .map_err(|error| format!("start_upload failed: {error:?}"))?;
    settled(core, &uploaded).await?;

    let listing = core
        .list_files(&handle, root)
        .await
        .map_err(|error| format!("listing after the upload failed: {error:?}"))?;
    if !listing.entries.iter().any(|entry| entry.name == "site") {
        return Err(format!("the uploaded tree is not listed: {listing:?}"));
    }

    let destination = local.path().join("downloaded");
    let downloaded = core
        .start_download(&handle, &format!("{root}/site"), destination.clone())
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
    let index = std::fs::read_to_string(destination.join("index.html")).map_err(|error| error.to_string())?;
    let css = std::fs::read_to_string(destination.join("assets/app.css")).map_err(|error| error.to_string())?;
    if index != "<h1>pod</h1>\n" || css != "body {}\n" {
        return Err(format!(
            "the downloaded tree is not the one that went up: {index:?} {css:?}"
        ));
    }

    // A command that fails inside the pod is a refusal with the pod's own words, not an empty
    // answer: `cat` of the folder is what a person sees when they open the wrong thing.
    let error = core
        .read_text_file(&handle, &format!("{root}/site"))
        .await
        .expect_err("a folder is not text");
    let detail = error.detail().unwrap_or_default().to_owned();
    if !detail.contains("not a regular file") {
        return Err(format!("reading a folder said {detail:?}"));
    }

    core.delete_entry(&handle, root, EntryKind::Directory)
        .await
        .map_err(|error| format!("removing the tree failed: {error:?}"))?;
    let tmp = core
        .list_files(&handle, "/tmp")
        .await
        .map_err(|error| format!("listing /tmp after the cleanup failed: {error:?}"))?;
    let name = root.rsplit('/').next().unwrap_or(root);
    if tmp.entries.iter().any(|entry| entry.name == name) {
        return Err(format!("the deleted tree is still listed: {tmp:?}"));
    }

    // The terminal and the file browser are different channels: one must still open after the
    // other has been used, or a pod exec is a pty in disguise.
    core.close_files(&handle)
        .await
        .map_err(|error| format!("close_files failed: {error:?}"))?;
    let terminal = core
        .k8s_exec_open(
            connection,
            &K8sExecRequest {
                namespace: NAMESPACE.to_owned(),
                pod: POD.to_owned(),
                container: CONTAINER.to_owned(),
                command: "echo after-files".to_owned(),
            },
            80,
            24,
        )
        .await
        .map_err(|error| format!("the terminal did not open after the browser: {error:?}"))?;
    core.close_terminal(&terminal)
        .await
        .map_err(|error| format!("closing the terminal failed: {error:?}"))?;

    Ok(())
}

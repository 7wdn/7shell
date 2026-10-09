//! Transfers against a real server: what they write while they are running, and what a second
//! attempt does with what the first one left behind.
//!
//! Ignored by default: it needs the SSH lab, which `make lab-up` starts.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 \
//!   SEVENSHELL_TEST_USER=labuser SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-sftp --test live_transfers -- --ignored -- --test-threads=1
//! ```
//!
//! These are this crate's own live tests because the rules live in it: whoever calls `download` or
//! `upload` — the core today, a command line later — gets them exactly as they are here, so the
//! place to prove them is where they are written. Everything below is deterministic: no test waits
//! for a transfer to be "slow enough", because a test that depends on timing is a test that passes
//! for the wrong reason on a fast machine and fails for no reason on a slow one.

use std::sync::Arc;
use std::time::Duration;

use sevenshell_sftp::{SftpSession, TransferProgress, local_part_of, part_of};
use sevenshell_ssh::{Credential, OpenRequest, Purpose, Route, SshSession, Target, TerminalSize, probe_host_key};
use tempfile::{TempDir, tempdir};

const OPEN_TIMEOUT: Duration = Duration::from_secs(20);

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// An SFTP channel to the lab, and a folder on it that this run owns.
struct Lab {
    /// Kept alive because the channel rides on it.
    _session: Arc<SshSession>,
    sftp: Arc<SftpSession>,
    root: String,
    _local: TempDir,
}

async fn lab(name: &str) -> Option<Lab> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return None;
    };
    let Ok(port) = port.parse::<u16>() else {
        eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
        return None;
    };

    let target = Target { host, port, user };
    let presented = probe_host_key(&target.host, target.port, OPEN_TIMEOUT)
        .await
        .expect("probe the host key");
    let request = OpenRequest {
        target,
        credential: Credential::Password(password),
        terminal_type: String::new(),
        keepalive_seconds: 0,
        forward_agent: false,
        timeout: OPEN_TIMEOUT,
        expected_host_key: presented,
        size: TerminalSize::default(),
        route: Route::Direct,
        purpose: Purpose::Carrier,
    };

    let (events, _unused) = tokio::sync::mpsc::channel(1);
    let session = Arc::new(SshSession::open(request, events).await.expect("connect to the lab"));
    let stream = session
        .open_subsystem_stream("sftp")
        .await
        .expect("open the sftp channel");
    let sftp = Arc::new(SftpSession::new(stream).await.expect("an sftp session"));

    // A folder of this run's own, under the home directory. Named per test, so two runs of the same
    // test file cannot tread on each other and a leftover from a failed run is at least identifiable.
    let home = sftp.resolve(".").await;
    let root = format!("{}/7shell-live-transfers-{name}", home.trim_end_matches('/'));
    if sftp.exists(&root).await.unwrap_or(false) {
        sftp.remove_dir_recursive(&root).await.expect("clear an earlier run");
    }
    sftp.create_dir(&root).await.expect("create the workspace");

    Some(Lab {
        _session: session,
        sftp,
        root,
        _local: tempdir().expect("a local scratch directory"),
    })
}

/// Text long enough to cross several chunks, and long enough that a resumed half is visible in it.
///
/// The lines are numbered, so a run's output says *where* two halves were joined rather than only
/// that they disagree.
fn payload(lines: usize) -> String {
    (0..lines).map(|line| format!("line {line:06}\n")).collect()
}

/// A marker of the same length, for a part file that is deliberately not the source's own prefix.
fn marker(bytes: usize) -> String {
    "X".repeat(bytes)
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_download_continues_into_the_part_file_it_finds() {
    let Some(lab) = lab("download-resume").await else {
        return;
    };
    let text = payload(4_000);
    let remote = format!("{}/payload.txt", lab.root);
    lab.sftp.write_text(&remote, &text).await.expect("write it");

    // A part file holding the beginning of *something* — deliberately not this file's own beginning,
    // because that is the only way to tell a transfer that continued from one that started again.
    let local = lab._local.path().join("payload.txt");
    let part = local_part_of(&local);
    let already = marker(text.len() / 3);
    std::fs::write(&part, &already).expect("write the part file");

    let progress = TransferProgress::new(0);
    lab.sftp
        .download(&remote, &local, &progress)
        .await
        .expect("the download finished");

    let arrived = std::fs::read_to_string(&local).expect("read what was downloaded");
    assert_eq!(arrived.len(), text.len(), "the finished file is the whole file");
    assert!(
        arrived.starts_with(&already),
        "the bytes that were already here were fetched again over the top of themselves"
    );
    assert_eq!(
        &arrived[already.len()..],
        &text[already.len()..],
        "the rest did not come from the server"
    );
    assert!(!part.exists(), "the part file became the file it was standing in for");
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn an_upload_continues_into_the_part_file_on_the_far_side() {
    let Some(lab) = lab("upload-resume").await else {
        return;
    };
    let text = payload(4_000);
    let local = lab._local.path().join("payload.txt");
    std::fs::write(&local, &text).expect("write the local file");

    // The far side already has the beginning of something, under the part name.
    let remote = format!("{}/payload.txt", lab.root);
    let part = part_of(&remote);
    let already = marker(text.len() / 3);
    lab.sftp.write_text(&part, &already).await.expect("write the part file");

    let progress = TransferProgress::new(0);
    lab.sftp
        .upload(&local, &remote, &progress)
        .await
        .expect("the upload finished");

    let landed = lab.sftp.read_text(&remote).await.expect("read it back");
    assert_eq!(landed.len(), text.len());
    assert!(
        landed.starts_with(&already),
        "what was already on the server was sent again over the top of itself"
    );
    assert_eq!(
        &landed[already.len()..],
        &text[already.len()..],
        "the rest of the file never arrived"
    );
    assert!(
        !lab.sftp.exists(&part).await.unwrap_or(false),
        "the part file on the far side is still there"
    );
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn the_name_that_was_asked_for_is_written_last_and_a_cancelled_one_leaves_nothing() {
    let Some(lab) = lab("written-last").await else {
        return;
    };
    let text = payload(400);
    let remote = format!("{}/payload.txt", lab.root);
    lab.sftp.write_text(&remote, &text).await.expect("write it");

    let local = lab._local.path().join("payload.txt");
    let part = local_part_of(&local);

    // Held before the first chunk, which is what makes this test about the order of things rather
    // than about how fast the machine is: the transfer has opened everything it needs and has not
    // been allowed to move a byte.
    let progress = TransferProgress::new(0);
    progress.pause();
    let running = {
        let sftp = Arc::clone(&lab.sftp);
        let remote = remote.clone();
        let local = local.clone();
        let progress = Arc::clone(&progress);
        tokio::spawn(async move { sftp.download(&remote, &local, &progress).await })
    };

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !local.exists(),
        "the name that was asked for exists while the transfer is still running"
    );
    assert!(part.exists(), "what a running transfer has is the part file beside it");

    progress.cancel();
    let outcome = running.await.expect("the task finished");
    assert!(outcome.is_err(), "a cancelled transfer is not a finished one");

    assert!(!part.exists(), "a cancelled transfer left its part file behind");
    assert!(
        !local.exists(),
        "a cancelled transfer created the name it was asked for"
    );
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_cancelled_transfer_leaves_the_file_that_was_already_there() {
    let Some(lab) = lab("cancelled-keeps").await else {
        return;
    };
    let text = payload(400);
    let remote = format!("{}/payload.txt", lab.root);
    lab.sftp.write_text(&remote, &text).await.expect("write it");

    // Both ends already hold a complete file — this is the download half first.
    let older = "the version that was here before\n";
    let local = lab._local.path().join("payload.txt");
    std::fs::write(&local, older).expect("write the older local file");

    let progress = TransferProgress::new(0);
    progress.cancel();
    let refused = lab.sftp.download(&remote, &local, &progress).await;
    assert!(refused.is_err(), "a cancelled transfer is not a finished one");

    assert_eq!(
        std::fs::read_to_string(&local).expect("read the local file"),
        older,
        "a cancelled download wrote over the file that was already there"
    );
    assert!(!local_part_of(&local).exists(), "and it left its part file behind");

    // And the upload half, where the file that must survive is on the far side.
    let occupied = format!("{}/occupied.txt", lab.root);
    lab.sftp
        .write_text(&occupied, older)
        .await
        .expect("write the remote file");
    let source = lab._local.path().join("occupied.txt");
    std::fs::write(&source, &text).expect("write the local file");

    let progress = TransferProgress::new(0);
    progress.cancel();
    let refused = lab.sftp.upload(&source, &occupied, &progress).await;
    assert!(refused.is_err(), "a cancelled transfer is not a finished one");

    assert_eq!(
        lab.sftp.read_text(&occupied).await.expect("read it back"),
        older,
        "a cancelled upload wrote over the file that was already there"
    );
    assert!(
        !lab.sftp.exists(&part_of(&occupied)).await.unwrap_or(false),
        "and it left its part file on the far side"
    );
}

//! SCP against a scripted peer.
//!
//! The protocol is a conversation over one stream, so the other side of it can be written down:
//! each test plays the far side by hand — sending exactly the records `scp -f` would, or answering
//! exactly the way `scp -t` does — and then asks whether the side under test did what the protocol
//! says it owes. Nothing here needs a server, which is the point: these are the cases that are
//! impossible to arrange on a real one, like a name that climbs out of the destination.

use std::path::Path;

use sevenshell_error::ErrorCode;
use sevenshell_ssh::scp::{self, TransferOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

const OPEN: TransferOptions = TransferOptions {
    recursive: false,
    preserve_times: false,
};

const TREE: TransferOptions = TransferOptions {
    recursive: true,
    preserve_times: false,
};

/// One byte from the far side, which has to be the acknowledgement the protocol owes.
async fn expect_acknowledgement(peer: &mut DuplexStream) {
    let mut byte = [0u8; 1];
    peer.read_exact(&mut byte)
        .await
        .expect("an acknowledgement should arrive");
    assert_eq!(byte[0], 0, "the side under test should have acknowledged");
}

/// Sends the single byte that means "go on".
async fn acknowledge(peer: &mut DuplexStream) {
    peer.write_all(&[0])
        .await
        .expect("an acknowledgement should be written");
}

/// A record line, without its newline.
async fn line(peer: &mut DuplexStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        peer.read_exact(&mut byte).await.expect("a record should arrive");
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).expect("a record is text")
}

/// A whole `C` record, its payload and its closing byte, the way a source sends one.
async fn send_file_record(peer: &mut DuplexStream, mode: u32, name: &str, body: &[u8]) {
    peer.write_all(format!("C{mode:04o} {} {name}\n", body.len()).as_bytes())
        .await
        .expect("a file record should be written");
    expect_acknowledgement(peer).await;
    peer.write_all(body).await.expect("the bytes should be written");
    peer.write_all(&[0])
        .await
        .expect("the end of the file should be written");
    expect_acknowledgement(peer).await;
}

/// What a scripted sink saw the source send.
#[derive(Debug, PartialEq, Eq)]
enum Sent {
    Time { seconds: u64 },
    File { mode: u32, name: String, body: Vec<u8> },
    Directory { mode: u32, name: String },
    End,
}

/// Plays `scp -t`: acknowledges the transfer, then every record, and says what arrived.
async fn play_sink(peer: &mut DuplexStream) -> Vec<Sent> {
    acknowledge(peer).await;
    let mut seen = Vec::new();
    loop {
        let mut first = [0u8; 1];
        match peer.read(&mut first).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => panic!("the sink could not read: {error}"),
        }
        match first[0] {
            b'C' => {
                let record = line(peer).await;
                let mut fields = record.splitn(3, ' ');
                let mode = u32::from_str_radix(fields.next().expect("a mode"), 8).expect("an octal mode");
                let size: usize = fields.next().expect("a size").parse().expect("a size in decimal");
                let name = fields.next().expect("a name").to_owned();
                acknowledge(peer).await;
                let mut body = vec![0u8; size];
                peer.read_exact(&mut body).await.expect("the file's bytes");
                let mut end = [0u8; 1];
                peer.read_exact(&mut end).await.expect("the end of a file");
                assert_eq!(end[0], 0, "a file ends with a zero byte of its own");
                acknowledge(peer).await;
                seen.push(Sent::File { mode, name, body });
            }
            b'D' => {
                let record = line(peer).await;
                let mut fields = record.splitn(3, ' ');
                let mode = u32::from_str_radix(fields.next().expect("a mode"), 8).expect("an octal mode");
                fields.next();
                let name = fields.next().expect("a name").to_owned();
                acknowledge(peer).await;
                seen.push(Sent::Directory { mode, name });
            }
            b'E' => {
                assert!(line(peer).await.is_empty(), "the end of a directory carries nothing");
                acknowledge(peer).await;
                seen.push(Sent::End);
            }
            b'T' => {
                let record = line(peer).await;
                let seconds = record
                    .split(' ')
                    .next()
                    .expect("a time")
                    .parse()
                    .expect("a time in seconds");
                acknowledge(peer).await;
                seen.push(Sent::Time { seconds });
            }
            other => panic!("the source sent 0x{other:02x}, which this sink does not know"),
        }
    }
    seen
}

// -- Downloads ------------------------------------------------------------

/// A transfer is awaited inside a command handler, which hands the future to the runtime; that
/// only compiles if the whole conversation is `Send`. Nothing here is polled — holding the future
/// is the assertion.
#[test]
fn a_transfer_future_can_be_handed_to_the_runtime() {
    fn require_send<T: Send>(value: T) -> T {
        value
    }

    let (ours, _theirs) = tokio::io::duplex(64);
    drop(require_send(scp::receive(
        ours,
        "/srv/file",
        Path::new("somewhere"),
        &OPEN,
        None,
    )));

    let (ours, _theirs) = tokio::io::duplex(64);
    drop(require_send(scp::send(ours, Path::new("a-file"), "/srv", &OPEN, None)));
}

#[tokio::test]
async fn a_download_writes_the_file_the_far_side_sends() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let (ours, mut theirs) = tokio::io::duplex(4096);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        send_file_record(&mut theirs, 0o644, "hello.txt", b"hello").await;
    });

    let report = scp::receive(ours, "/srv/hello.txt", room.path(), &OPEN, None)
        .await
        .expect("the download should finish");
    source.await.expect("the scripted source should finish");

    assert_eq!(
        std::fs::read(room.path().join("hello.txt")).expect("the file should be there"),
        b"hello"
    );
    assert_eq!(report.bytes, 5);
    assert_eq!(report.files(), 1);
    assert_eq!(report.directories(), 0);
    assert_eq!(report.entries[0].path, "hello.txt");
}

#[tokio::test]
async fn a_recursive_download_rebuilds_the_tree_below_the_destination() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let (ours, mut theirs) = tokio::io::duplex(4096);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        theirs.write_all(b"D0755 0 tree\n").await.expect("a directory record");
        expect_acknowledgement(&mut theirs).await;
        send_file_record(&mut theirs, 0o644, "one.txt", b"one").await;
        theirs.write_all(b"D0700 0 nested\n").await.expect("a directory record");
        expect_acknowledgement(&mut theirs).await;
        send_file_record(&mut theirs, 0o600, "two.txt", b"two").await;
        theirs.write_all(b"E\n").await.expect("the end of a directory");
        expect_acknowledgement(&mut theirs).await;
        theirs.write_all(b"E\n").await.expect("the end of a directory");
        expect_acknowledgement(&mut theirs).await;
    });

    let report = scp::receive(ours, "/srv/tree", room.path(), &TREE, None)
        .await
        .expect("the download should finish");
    source.await.expect("the scripted source should finish");

    assert_eq!(
        std::fs::read(room.path().join("tree/one.txt")).expect("the first file"),
        b"one"
    );
    assert_eq!(
        std::fs::read(room.path().join("tree/nested/two.txt")).expect("the second file"),
        b"two"
    );
    assert_eq!(report.directories(), 2);
    assert_eq!(report.files(), 2);
    assert_eq!(report.entries[2].path, "tree/nested");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let nested = std::fs::metadata(room.path().join("tree/nested")).expect("the directory");
        assert_eq!(
            nested.permissions().mode() & 0o777,
            0o700,
            "the mode arrives with the directory"
        );
    }
}

#[tokio::test]
async fn a_name_that_climbs_out_of_the_destination_is_refused() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let destination = room.path().join("downloads");
    let (ours, mut theirs) = tokio::io::duplex(4096);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        theirs.write_all(b"C0644 4 ../evil\n").await.expect("a file record");
    });

    let error = scp::receive(ours, "/srv/evil", &destination, &OPEN, None)
        .await
        .expect_err("the name should have been refused");
    source.await.expect("the scripted source should finish");

    assert_eq!(error.code(), ErrorCode::ScpTransferFailed);
    assert!(
        !room.path().join("evil").exists(),
        "nothing may be written outside the destination"
    );
}

#[tokio::test]
async fn a_name_that_is_a_path_is_refused() {
    for name in ["a/b", "/etc/passwd", "..", "."] {
        let room = tempfile::tempdir().expect("a temporary directory");
        let (ours, mut theirs) = tokio::io::duplex(4096);
        let sent = name.to_owned();

        let source = tokio::spawn(async move {
            expect_acknowledgement(&mut theirs).await;
            theirs
                .write_all(format!("C0644 3 {sent}\n").as_bytes())
                .await
                .expect("a file record");
        });

        let error = scp::receive(ours, "/srv/file", room.path(), &OPEN, None)
            .await
            .expect_err("a name that is a path should have been refused");
        source.await.expect("the scripted source should finish");

        assert_eq!(
            error.code(),
            ErrorCode::ScpTransferFailed,
            "`{name}` should have been refused"
        );
        assert!(!room.path().join("file").exists());
    }
}

#[tokio::test]
async fn the_far_sides_own_error_becomes_the_error_the_caller_reads() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let (ours, mut theirs) = tokio::io::duplex(4096);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        theirs.write_all(&[2]).await.expect("a fatal error byte");
        theirs
            .write_all(b"scp: /srv/absent: No such file or directory\n")
            .await
            .expect("the far side's own words");
    });

    let error = scp::receive(ours, "/srv/absent", room.path(), &OPEN, None)
        .await
        .expect_err("a fatal record should end the transfer");
    source.await.expect("the scripted source should finish");

    assert_eq!(error.code(), ErrorCode::ScpTransferFailed);
    assert!(
        error.detail().unwrap_or_default().contains("No such file or directory"),
        "the far side's words should survive: {:?}",
        error.detail()
    );
}

#[tokio::test]
async fn a_download_that_stops_before_its_bytes_arrive_says_so() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let (ours, mut theirs) = tokio::io::duplex(4096);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        // The header promises four bytes; the connection goes away after two of them.
        theirs.write_all(b"C0644 4 cut.txt\n").await.expect("a file record");
        expect_acknowledgement(&mut theirs).await;
        theirs.write_all(b"cu").await.expect("part of the file");
    });

    let error = scp::receive(ours, "/srv/cut.txt", room.path(), &OPEN, None)
        .await
        .expect_err("a cut transfer should be an error");
    source.await.expect("the scripted source should finish");

    assert_eq!(error.code(), ErrorCode::ScpTransferFailed);
    assert!(
        error.detail().unwrap_or_default().contains("connection ended"),
        "the reason should be the connection, not something else: {:?}",
        error.detail()
    );
    assert!(
        !room.path().join("cut.txt").exists(),
        "a file that did not finish may not wear the name it was asked for"
    );
}

#[tokio::test]
async fn a_directory_is_refused_when_recursion_was_not_asked_for() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let (ours, mut theirs) = tokio::io::duplex(4096);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        theirs.write_all(b"D0755 0 tree\n").await.expect("a directory record");
    });

    let error = scp::receive(ours, "/srv/tree", room.path(), &OPEN, None)
        .await
        .expect_err("a directory should not be taken without recursion");
    source.await.expect("the scripted source should finish");

    assert_eq!(error.code(), ErrorCode::ScpTransferFailed);
    assert!(!room.path().join("tree").exists());
}

#[tokio::test]
async fn a_time_record_is_acknowledged_and_the_file_behind_it_still_arrives() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let (ours, mut theirs) = tokio::io::duplex(4096);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        theirs
            .write_all(b"T1700000000 0 1700000000 0\n")
            .await
            .expect("a time record");
        expect_acknowledgement(&mut theirs).await;
        send_file_record(&mut theirs, 0o644, "timed.txt", b"now").await;
    });

    let report = scp::receive(ours, "/srv/timed.txt", room.path(), &OPEN, None)
        .await
        .expect("the download should finish");
    source.await.expect("the scripted source should finish");

    assert_eq!(report.bytes, 3);
    assert_eq!(std::fs::read(room.path().join("timed.txt")).expect("the file"), b"now");
}

#[tokio::test]
async fn the_destination_is_created_when_it_is_not_there_yet() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let destination = room.path().join("not/yet/there");
    let (ours, mut theirs) = tokio::io::duplex(4096);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        send_file_record(&mut theirs, 0o644, "made.txt", b"x").await;
    });

    scp::receive(ours, "/srv/made.txt", &destination, &OPEN, None)
        .await
        .expect("the download should finish");
    source.await.expect("the scripted source should finish");

    assert_eq!(std::fs::read(destination.join("made.txt")).expect("the file"), b"x");
}

#[tokio::test]
async fn the_progress_callback_ends_on_the_number_of_bytes_that_arrived() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let (ours, mut theirs) = tokio::io::duplex(64 * 1024);

    let source = tokio::spawn(async move {
        expect_acknowledgement(&mut theirs).await;
        let body = vec![7u8; 100 * 1024];
        send_file_record(&mut theirs, 0o644, "big.bin", &body).await;
    });

    let mut seen = Vec::new();
    let mut report_progress = |bytes: u64| seen.push(bytes);
    let report = scp::receive(ours, "/srv/big.bin", room.path(), &OPEN, Some(&mut report_progress))
        .await
        .expect("the download should finish");
    source.await.expect("the scripted source should finish");

    assert_eq!(report.bytes, 100 * 1024);
    assert_eq!(seen.last().copied(), Some(100 * 1024));
    assert!(
        seen.windows(2).all(|pair| pair[0] <= pair[1]),
        "progress only ever grows"
    );
}

// -- Uploads --------------------------------------------------------------

#[tokio::test]
async fn an_upload_sends_the_file_under_its_own_name() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let file = room.path().join("report.txt");
    std::fs::write(&file, b"contents").expect("a file to send");

    let (ours, mut theirs) = tokio::io::duplex(4096);
    let sink = tokio::spawn(async move { play_sink(&mut theirs).await });

    let report = scp::send(ours, &file, "/srv", &OPEN, None)
        .await
        .expect("the upload should finish");
    let seen = sink.await.expect("the scripted sink should finish");

    assert_eq!(report.bytes, 8);
    assert_eq!(report.files(), 1);
    match &seen[..] {
        [Sent::File { name, body, .. }] => {
            assert_eq!(name, "report.txt");
            assert_eq!(body, b"contents");
        }
        other => panic!("the sink should have seen one file, not {other:?}"),
    }
}

#[tokio::test]
async fn an_upload_walks_a_directory_and_closes_it_after_its_files() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let tree = room.path().join("tree");
    std::fs::create_dir_all(tree.join("nested")).expect("a directory to send");
    std::fs::write(tree.join("a.txt"), b"a").expect("a file");
    std::fs::write(tree.join("nested/b.txt"), b"b").expect("a file");

    let (ours, mut theirs) = tokio::io::duplex(4096);
    let sink = tokio::spawn(async move { play_sink(&mut theirs).await });

    let report = scp::send(ours, &tree, "/srv", &TREE, None)
        .await
        .expect("the upload should finish");
    let seen = sink.await.expect("the scripted sink should finish");

    let shape: Vec<&str> = seen
        .iter()
        .map(|entry| match entry {
            Sent::Directory { name, .. } => name.as_str(),
            Sent::File { name, .. } => name.as_str(),
            Sent::End => "end",
            Sent::Time { .. } => "time",
        })
        .collect();
    assert_eq!(shape, ["tree", "a.txt", "nested", "b.txt", "end", "end"]);
    assert_eq!(report.directories(), 2);
    assert_eq!(report.files(), 2);
    assert_eq!(report.bytes, 2);
}

#[tokio::test]
async fn an_upload_keeps_the_mode_the_file_has() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let file = room.path().join("private.txt");
    std::fs::write(&file, b"x").expect("a file to send");

    #[cfg(unix)]
    let expected = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("a mode");
        0o600
    };
    // Windows has no bits to carry, so what is sent is what a file of that shape usually is.
    #[cfg(not(unix))]
    let expected = 0o644;

    let (ours, mut theirs) = tokio::io::duplex(4096);
    let sink = tokio::spawn(async move { play_sink(&mut theirs).await });

    scp::send(ours, &file, "/srv", &OPEN, None)
        .await
        .expect("the upload should finish");
    let seen = sink.await.expect("the scripted sink should finish");

    match &seen[..] {
        [Sent::File { mode, .. }] => assert_eq!(*mode, expected),
        other => panic!("the sink should have seen one file, not {other:?}"),
    }
}

#[tokio::test]
async fn times_are_sent_only_when_they_were_asked_for() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let file = room.path().join("timed.txt");
    std::fs::write(&file, b"x").expect("a file to send");

    let (ours, mut theirs) = tokio::io::duplex(4096);
    let sink = tokio::spawn(async move { play_sink(&mut theirs).await });
    let options = TransferOptions {
        preserve_times: true,
        ..OPEN
    };
    scp::send(ours, &file, "/srv", &options, None)
        .await
        .expect("the upload should finish");
    let seen = sink.await.expect("the scripted sink should finish");

    match &seen[..] {
        [Sent::Time { seconds }, Sent::File { .. }] => {
            assert!(*seconds > 1_600_000_000, "a real modification time, not a zero");
        }
        other => panic!("the sink should have seen a time record first, not {other:?}"),
    }
}

#[tokio::test]
async fn a_refusal_from_the_far_side_is_reported_with_its_words() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let file = room.path().join("file.txt");
    std::fs::write(&file, b"x").expect("a file to send");

    let (ours, mut theirs) = tokio::io::duplex(4096);
    let sink = tokio::spawn(async move {
        acknowledge(&mut theirs).await;
        let record = line(&mut theirs).await;
        assert!(
            record.starts_with('C'),
            "the file record should arrive before the refusal"
        );
        theirs.write_all(&[2]).await.expect("a fatal error byte");
        theirs
            .write_all(b"scp: /srv: Permission denied\n")
            .await
            .expect("the far side's own words");
    });

    let error = scp::send(ours, &file, "/srv", &OPEN, None)
        .await
        .expect_err("a refusal should end the upload");
    sink.await.expect("the scripted sink should finish");

    assert_eq!(error.code(), ErrorCode::ScpTransferFailed);
    assert!(
        error.detail().unwrap_or_default().contains("Permission denied"),
        "the far side's words should survive: {:?}",
        error.detail()
    );
}

#[tokio::test]
async fn an_upload_does_not_wait_forever_when_the_far_side_stops_answering() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let file = room.path().join("file.txt");
    std::fs::write(&file, b"x").expect("a file to send");

    let (ours, mut theirs) = tokio::io::duplex(4096);
    let sink = tokio::spawn(async move {
        acknowledge(&mut theirs).await;
        // The file record arrives and is never answered.
        let record = line(&mut theirs).await;
        assert!(record.starts_with('C'));
    });

    let error = scp::send(ours, &file, "/srv", &OPEN, None)
        .await
        .expect_err("a sink that goes away is an error, not a wait");
    sink.await.expect("the scripted sink should finish");

    assert_eq!(error.code(), ErrorCode::ScpTransferFailed);
    assert!(error.detail().unwrap_or_default().contains("connection ended"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_inside_a_directory_is_skipped_and_counted() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let tree = room.path().join("tree");
    std::fs::create_dir(&tree).expect("a directory to send");
    std::fs::write(tree.join("real.txt"), b"r").expect("a file");
    std::os::unix::fs::symlink("real.txt", tree.join("link.txt")).expect("a symbolic link");

    let (ours, mut theirs) = tokio::io::duplex(4096);
    let sink = tokio::spawn(async move { play_sink(&mut theirs).await });

    let report = scp::send(ours, &tree, "/srv", &TREE, None)
        .await
        .expect("the upload should finish");
    let seen = sink.await.expect("the scripted sink should finish");

    assert_eq!(report.skipped, 1);
    let files: Vec<String> = seen
        .into_iter()
        .filter_map(|entry| match entry {
            Sent::File { name, .. } => Some(name),
            _ => None,
        })
        .collect();
    assert_eq!(files, ["real.txt"]);
}

#[cfg(unix)]
#[tokio::test]
async fn a_name_the_protocol_cannot_carry_is_refused_rather_than_rewritten() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let file = room.path().join("two\nlines.txt");
    std::fs::write(&file, b"x").expect("a file to send");

    let (ours, mut theirs) = tokio::io::duplex(4096);
    let sink = tokio::spawn(async move { play_sink(&mut theirs).await });

    let error = scp::send(ours, &file, "/srv", &OPEN, None)
        .await
        .expect_err("a newline cannot be part of a record");
    let _ = sink.await;

    assert_eq!(error.code(), ErrorCode::ScpTransferFailed);
}

#[tokio::test]
async fn a_silent_far_side_is_not_an_empty_transfer() {
    let room = tempfile::tempdir().expect("a temporary directory");
    let (ours, mut theirs) = tokio::io::duplex(4096);
    let source = tokio::spawn(async move {
        // The far side accepts the channel and says nothing at all.
        expect_acknowledgement(&mut theirs).await;
    });

    let error = scp::receive(ours, "/srv/empty", room.path(), &OPEN, None)
        .await
        .expect_err("nothing arrived, so this is not a finished download");
    source.await.expect("the scripted source should finish");

    // What a missing `scp` looks like from here: the channel closes and no record arrives.
    assert_eq!(error.code(), ErrorCode::ScpTransferFailed);
    assert!(error.detail().unwrap_or_default().contains("not have scp"));
}

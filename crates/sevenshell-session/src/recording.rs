//! Recording a terminal: what the server sent, and when it arrived.
//!
//! The format is [asciicast v2](https://docs.asciinema.org/manual/asciicast/v2/), which is a line
//! of JSON describing the session followed by one line per event. Two reasons for that rather than
//! something of our own: a recording is most useful when the tools people already have can play it,
//! and a format that is somebody else's specification is one we cannot quietly drift away from.
//!
//! What is written is the output — what the server sent — and not the keystrokes: a recording is
//! for reading back what happened on the machine, and the things typed into it are in the history
//! the shell keeps. It is also a plain file, which is worth saying out loud in the interface: a
//! recording holds everything that was ever printed in that terminal.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use sevenshell_error::ShellError;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

/// The first line of a recording: the shape of the terminal when it started.
///
/// `version` is the format's, not the application's, and the two are easy to confuse.
#[derive(Serialize)]
struct Header<'a> {
    version: u8,
    width: u32,
    height: u32,
    timestamp: u64,
    title: &'a str,
}

/// One event: when it happened relative to the start, that it was output, and what it was.
#[derive(Serialize)]
struct Event<'a>(f64, &'a str, &'a str);

/// Where a recording goes, and the size the terminal was when it began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSpec {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub title: String,
}

/// An open recording.
///
/// Every write is best effort and every failure is swallowed: a disk that filled up while somebody
/// was working on a server is a reason to stop recording, not a reason to close their terminal.
pub struct Recording {
    file: BufWriter<File>,
    started: Instant,
}

impl Recording {
    /// Opens the file and writes the header.
    ///
    /// A recording that cannot be opened at all is an error the caller sees, because it is known
    /// before the session starts and there is still time to say something. After that, nothing is.
    pub async fn create(spec: &RecordingSpec) -> Result<Self, ShellError> {
        if let Some(parent) = spec.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| io_failure(parent, &error))?;
        }

        let file = File::create(&spec.path)
            .await
            .map_err(|error| io_failure(&spec.path, &error))?;
        let mut recording = Self {
            file: BufWriter::new(file),
            started: Instant::now(),
        };

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or_default();

        let header = Header {
            version: 2,
            width: spec.width,
            height: spec.height,
            timestamp,
            title: &spec.title,
        };
        recording.write_line(&header).await;
        Ok(recording)
    }

    /// Records a chunk of output, as it arrived.
    ///
    /// The bytes are decoded as UTF-8 lossily, which is what the terminal itself did to draw them:
    /// a recording of a file that was not text shows the same replacement characters the person saw.
    pub async fn output(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let event = Event(self.started.elapsed().as_secs_f64(), "o", &text);
        self.write_line(&event).await;
    }

    /// Flushes what is buffered. Called when the session ends, and harmless to skip.
    pub async fn finish(mut self) {
        let _ = self.file.flush().await;
    }

    async fn write_line<T: Serialize>(&mut self, value: &T) {
        let Ok(mut line) = serde_json::to_vec(value) else {
            return;
        };
        line.push(b'\n');
        let _ = self.file.write_all(&line).await;
    }
}

/// The file could not be created, said in the terms of the thing that failed.
fn io_failure(path: &Path, error: &std::io::Error) -> ShellError {
    ShellError::store_io(format!("{} could not be written: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn record(chunks: &[&[u8]]) -> String {
        let directory = tempfile::tempdir().expect("a temp dir");
        let spec = RecordingSpec {
            path: directory.path().join("session.cast"),
            width: 120,
            height: 40,
            title: "lab".to_owned(),
        };

        let mut recording = Recording::create(&spec).await.expect("the file opens");
        for chunk in chunks {
            recording.output(chunk).await;
        }
        recording.finish().await;

        tokio::fs::read_to_string(&spec.path)
            .await
            .expect("the recording is readable")
    }

    #[tokio::test]
    async fn a_recording_is_a_header_and_one_line_per_chunk() {
        let text = record(&[b"hello\n", b"world\n"]).await;
        let mut lines = text.lines();

        let header: serde_json::Value =
            serde_json::from_str(lines.next().expect("a header")).expect("the header is JSON");
        assert_eq!(header["version"], 2, "asciicast v2");
        assert_eq!(header["width"], 120);
        assert_eq!(header["height"], 40);
        assert_eq!(header["title"], "lab");
        assert!(header["timestamp"].as_u64().unwrap_or_default() > 0);

        let first: serde_json::Value =
            serde_json::from_str(lines.next().expect("an event")).expect("the event is JSON");
        assert_eq!(first[1], "o", "output");
        assert_eq!(first[2], "hello\n");
        assert!(
            first[0].as_f64().unwrap_or(-1.0) >= 0.0,
            "the time is measured from the start"
        );

        assert!(lines.next().is_some(), "the second chunk is its own line");
        assert!(lines.next().is_none(), "and there is nothing else");
    }

    #[tokio::test]
    async fn output_that_is_not_text_is_recorded_as_what_was_shown() {
        // A terminal shows replacement characters for bytes that are not UTF-8, and a recording
        // that refused them would be a recording that stopped mid-session.
        let text = record(&[&[0xff, 0xfe, b'a']]).await;
        let event = text.lines().nth(1).expect("an event");

        // The replacement characters themselves rather than an escape: the file is UTF-8, and what
        // somebody reading the recording sees should be what the terminal drew.
        assert!(
            event.contains('\u{fffd}'),
            "the replacement characters are in the line: {event}"
        );
        assert!(event.contains('a'), "and the byte that was text is kept");
    }
}

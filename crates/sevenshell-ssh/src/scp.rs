//! SCP, spoken from this side.
//!
//! The file browser talks SFTP, but a server can be old enough to offer a shell and no `sftp`
//! subsystem, and then the only thing that moves a file is the protocol `scp` has spoken since the
//! beginning: one byte stream carrying `C`, `D`, `E` and `T` records, with single-byte
//! acknowledgements in between.
//!
//! This module is the protocol and nothing else. It runs over any [`AsyncRead`] + [`AsyncWrite`], so
//! the tests drive it against a scripted peer through an in-memory pipe, and the session side only
//! has to start `scp -f` or `scp -t` on a channel and hand the stream over.
//!
//! Three things matter more here than the record format:
//!
//! * **A name from the far side is one path component.** A `C` record is a line of text, so
//!   `../../.ssh/authorized_keys` is a name the far side may choose. Every name is checked before
//!   it touches the local file system, and the resolved path is checked again to still be inside
//!   the directory the caller named. A name the far side sends is never allowed to decide *where*
//!   something lands, only *what it is called*.
//! * **Acknowledgements are the flow control.** The source sends one record and waits; the sink
//!   acknowledges one record and then reads. A side that acknowledges without having written the
//!   bytes would hand the far side permission to move on to a file that is not there, so every
//!   acknowledgement in this file comes after the work it stands for. The same is true of reading:
//!   the next record is not read until the previous one is finished.
//! * **An error ends the transfer.** The protocol has warning and fatal records (`0x01`, `0x02`) and
//!   a source may go on after a warning. This side stops at either: the caller gets the far side's
//!   own words and a partial transfer, never a complete-looking one with files missing from it.
//!
//! What the protocol cannot carry is left out rather than approximated: there is no record for a
//! symbolic link, a device or a socket, so a recursive upload counts those and skips them, and a
//! name that is not UTF-8 is refused rather than rewritten into a different name.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use sevenshell_error::ShellError;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};

use crate::SshSession;
use crate::exec::whole_seconds;

/// The one byte that means "go on".
const ACK: u8 = 0;
/// The byte that introduces a non-fatal error from the far side.
const WARNING: u8 = 1;
/// The byte that introduces a fatal error from the far side.
const FATAL: u8 = 2;

/// How much of a file is moved before the progress callback is told again.
///
/// A transfer is a sequence of acknowledgements, so the chunk is also the unit of patience: the
/// smaller it is the more often a cancellation is noticed, the larger it is the fewer round trips
/// this side makes for its own bookkeeping.
const CHUNK_BYTES: usize = 64 * 1024;

/// The longest record this will read before deciding the far side is not speaking SCP.
///
/// A name is one path component on any file system this runs against, so anything near this is
/// already suspicious; the ceiling exists so a peer that never sends a newline cannot make this side
/// read forever.
const MAX_RECORD_BYTES: usize = 8 * 1024;

/// What a download appends to the name it is writing while it is still writing it.
///
/// The same suffix the SFTP transfers use, on purpose: a person who finds a leftover should not have
/// to learn that this one came from a different protocol. The name that was asked for is only
/// created once every byte of it has arrived, so a broken transfer cannot leave a short file wearing
/// the right name.
const PART_SUFFIX: &str = ".7shell-part";

/// Bytes moved, told to whoever asked for a bar.
///
/// Cumulative for the whole transfer, so a caller only has to store the number it is given. There is
/// no richer event because the protocol has none: it never says how much is coming for a directory,
/// and a file's size is known only once its record has arrived.
pub type ByteProgress<'a> = &'a mut (dyn FnMut(u64) + Send);

/// What a transfer is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferOptions {
    /// Whether a directory is descended into. It also decides whether the far side is asked for
    /// `-r`, so a transfer that was not asked for recursion is not one the far side can turn into
    /// one: a `D` record arriving anyway is refused.
    pub recursive: bool,
    /// Whether modification times travel. Only an upload can honour it: putting a received file's
    /// time back needs something this crate does not carry, so a download consumes `T` records and
    /// leaves the file's time as the write made it. An upload that asks for this also starts the
    /// far side with `-p`, because a sink is only expecting `T` when it was told to preserve times.
    pub preserve_times: bool,
}

impl Default for TransferOptions {
    /// A single file, named by its own name, with the times of this machine not carried.
    fn default() -> Self {
        Self {
            recursive: false,
            preserve_times: false,
        }
    }
}

/// One thing that crossed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferredEntry {
    /// Where it went, relative to the directory the transfer was given, joined with `/` — the far
    /// side's separator and the only one the protocol has.
    pub path: String,
    /// Payload bytes. A directory is zero: the protocol sends a name, not a size.
    pub bytes: u64,
    pub directory: bool,
}

/// What a transfer did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferReport {
    /// Everything that crossed, in the order the protocol carried it.
    pub entries: Vec<TransferredEntry>,
    /// Payload bytes over all entries: the number a progress bar ends on.
    pub bytes: u64,
    /// Things that were there and could not cross — a symbolic link or a special file inside a
    /// recursive upload. Counted rather than refused, because failing an upload of a whole tree
    /// because one link in it cannot be sent is not the answer a person wants; the caller can say
    /// so afterwards.
    pub skipped: u64,
}

impl TransferReport {
    pub fn files(&self) -> u64 {
        self.entries.iter().filter(|entry| !entry.directory).count() as u64
    }

    pub fn directories(&self) -> u64 {
        self.entries.iter().filter(|entry| entry.directory).count() as u64
    }
}

/// The command that makes the far side send `remote_path` to this side.
///
/// The path is one shell word and nothing else, so a name that contains a quote, a space or a
/// semicolon is a name and not an instruction. `--` ends the options even before that, so a path
/// that begins with `-` is a path.
pub fn download_command(remote_path: &str, options: &TransferOptions) -> Result<String, ShellError> {
    check_remote_path(remote_path)?;
    let recursive = if options.recursive { " -r" } else { "" };
    Ok(format!("scp{recursive} -f -- {}", quote(remote_path)))
}

/// The command that lets this side put files into `remote_directory`.
///
/// `-r` travels to the sink as well as to the source, and it is not decoration: a sink that was not
/// started with it treats a `D` record as a protocol violation and ends the transfer, so a
/// recursive upload without it fails on the first directory.
///
/// `-p` is only asked for when times are to be preserved: a sink that was not told to preserve them
/// may refuse a `T` record, and the whole point of the flag is to agree about it up front.
pub fn upload_command(remote_directory: &str, options: &TransferOptions) -> Result<String, ShellError> {
    check_remote_path(remote_directory)?;
    let recursive = if options.recursive { " -r" } else { "" };
    let times = if options.preserve_times { " -p" } else { "" };
    Ok(format!("scp{recursive}{times} -t -- {}", quote(remote_directory)))
}

/// Receives what the far side sends, writing it inside `destination`.
///
/// This is the sink half of the protocol: it acknowledges, it creates, it writes, and it says
/// nothing the source did not ask for. The stream must already be attached to a source running
/// `scp -f`.
///
/// A destination that does not exist is created, because it is the caller's own choice and not a
/// name from the far side; a destination that exists and is not a directory is refused.
pub async fn receive<IO>(
    io: IO,
    remote_path: &str,
    destination: &Path,
    options: &TransferOptions,
    progress: Option<ByteProgress<'_>>,
) -> Result<TransferReport, ShellError>
where
    IO: AsyncRead + AsyncWrite + Send,
{
    let mut wire = Wire::new(io);
    let mut tally = Tally {
        report: TransferReport::default(),
        progress,
    };

    // The sink speaks first. A source waits for this byte before it sends anything; one that does
    // not wait reads it as the acknowledgement of its first record instead, and the conversation
    // stays in step either way, because this side owes exactly one acknowledgement per record it
    // accepts and this is the one it owes before the first.
    wire.acknowledge(remote_path, "the opening acknowledgement").await?;

    prepare_destination(destination, remote_path).await?;

    let root = destination.to_path_buf();
    // The directory being written into, with the name it has inside the transfer. The destination
    // itself is the bottom of the stack and is never popped, so a record can never close it.
    let mut open: Vec<OpenDirectory> = vec![OpenDirectory {
        path: root.clone(),
        relative: String::new(),
        mode: None,
    }];
    let mut buffer = vec![0u8; CHUNK_BYTES];

    loop {
        let start = match wire.octet().await {
            Ok(start) => start,
            Err(failure) => return Err(failure.error(remote_path, "the transfer could not be read")),
        };
        // A record only begins at a boundary, so the end of the stream here is the end of the
        // transfer and not a cut one.
        let Some(start) = start else { break };

        match start {
            b'T' => {
                let line = wire.record(remote_path, "a time record").await?;
                check_time_record(&line, remote_path)?;
                wire.acknowledge(remote_path, "a time record").await?;
            }
            b'C' => {
                let line = wire.record(remote_path, "a file record").await?;
                let record = FileRecord::parse(&line).map_err(|reason| malformed(remote_path, reason))?;
                let (parent, relative) = current_directory(&open, remote_path)?;
                let name = part_name(&record.name, remote_path)?;
                let target = parent.join(name);
                inside(&root, &target, remote_path)?;
                let relative = beneath(&relative, name);
                receive_file(
                    &mut wire,
                    remote_path,
                    &target,
                    &relative,
                    &record,
                    &mut tally,
                    &mut buffer,
                )
                .await?;
            }
            b'D' => {
                if !options.recursive {
                    return Err(ShellError::scp_transfer_failed(
                        remote_path,
                        "the far side sent a directory, but this transfer was not asked to take one",
                    ));
                }
                let line = wire.record(remote_path, "a directory record").await?;
                let record = DirectoryRecord::parse(&line).map_err(|reason| malformed(remote_path, reason))?;
                let (parent, relative) = current_directory(&open, remote_path)?;
                let name = part_name(&record.name, remote_path)?;
                let target = parent.join(name);
                inside(&root, &target, remote_path)?;
                let relative = beneath(&relative, name);
                open_directory(remote_path, &target, &relative).await?;
                open.push(OpenDirectory {
                    path: target,
                    relative: relative.clone(),
                    mode: Some(record.mode),
                });
                wire.acknowledge(remote_path, "a directory record").await?;
                tally.landed(relative, 0, true);
            }
            b'E' => {
                let line = wire.record(remote_path, "the end of a directory").await?;
                if !line.is_empty() {
                    return Err(malformed(
                        remote_path,
                        "the end of a directory arrived with something attached",
                    ));
                }
                if open.len() < 2 {
                    return Err(malformed(
                        remote_path,
                        "the far side closed a directory this side never opened",
                    ));
                }
                wire.acknowledge(remote_path, "the end of a directory").await?;
                if let Some(frame) = open.pop()
                    && let Some(mode) = frame.mode
                {
                    // Last, not first: a directory that is about to be written into has to stay
                    // writable while its contents arrive.
                    apply_mode(&frame.path, mode).await.map_err(|error| {
                        local_failure(
                            remote_path,
                            &frame.relative,
                            "the directory's permissions could not be set",
                            &error,
                        )
                    })?;
                }
            }
            WARNING | FATAL => {
                let message = wire.record(remote_path, "an error record").await?;
                return Err(refusal(remote_path, start == FATAL, &message));
            }
            ACK => {
                return Err(malformed(
                    remote_path,
                    "an acknowledgement arrived where a record belongs, so the two sides are out of step",
                ));
            }
            other => {
                return Err(malformed(
                    remote_path,
                    &format!("0x{other:02x} is not a record this protocol has"),
                ));
            }
        }
    }

    if open.len() > 1 {
        return Err(ShellError::scp_transfer_failed(
            remote_path,
            "the connection ended inside a directory the far side never closed",
        ));
    }

    // A source always says something: one record, or one error. A silent stream is a command that
    // did not run — a server without `scp`, most often — and calling that an empty transfer would
    // hand the caller a success it cannot tell from a real one.
    if tally.report.entries.is_empty() {
        return Err(ShellError::scp_transfer_failed(
            remote_path,
            "the connection ended before the far side sent anything, so it may not have scp",
        ));
    }

    Ok(tally.finish())
}

/// Sends `source` to the far side, which must already be running `scp -t` for `remote_directory`.
///
/// One named source per transfer, which is what the protocol's own command line does; a caller with
/// several files starts several transfers, or hands over a directory.
pub async fn send<IO>(
    io: IO,
    source: &Path,
    remote_directory: &str,
    options: &TransferOptions,
    progress: Option<ByteProgress<'_>>,
) -> Result<TransferReport, ShellError>
where
    IO: AsyncRead + AsyncWrite + Send,
{
    let mut wire = Wire::new(io);
    let mut tally = Tally {
        report: TransferReport::default(),
        progress,
    };

    // The sink acknowledges each of its targets before the source says anything, so a command that
    // is not `scp -t` — a shell that printed "command not found" instead — fails here rather than
    // halfway through a file that the far side will never finish.
    match wire.reply().await {
        Ok(Reply::Acknowledged) => {}
        Ok(Reply::Refused { fatal, message }) => return Err(refusal(remote_directory, fatal, &message)),
        Ok(Reply::Ended) => {
            return Err(ShellError::scp_transfer_failed(
                remote_directory,
                "the connection closed before the far side accepted the transfer; it may not have scp",
            ));
        }
        Err(failure) => return Err(failure.error(remote_directory, "the far side's opening acknowledgement")),
    }

    // Following a link the caller named is what `scp` does and what a person who picked that name
    // means. Inside a directory it is different: a link's content is not what the far side will
    // rebuild, and following one can walk in a circle, so links below the top are skipped.
    let metadata = fs::metadata(source).await.map_err(|error| {
        local_failure(
            remote_directory,
            "the source",
            "the file to send could not be read",
            &error,
        )
    })?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(ShellError::scp_transfer_failed(
            remote_directory,
            "the source is neither a file nor a directory, which this protocol cannot carry",
        ));
    }

    let name = wire_name(source, remote_directory)?;
    let mut work = vec![Step::Entry {
        path: source.to_path_buf(),
        relative: name,
        metadata,
    }];

    while let Some(step) = work.pop() {
        match step {
            Step::Close => {
                wire.write(b"E\n")
                    .await
                    .map_err(|failure| failure.error(remote_directory, "the end of a directory"))?;
                expect_acknowledgement(&mut wire, remote_directory, "accept the end of a directory").await?;
            }
            Step::Entry {
                path,
                relative,
                metadata,
            } => {
                let name = wire_name(&path, remote_directory)?;
                if metadata.is_dir() {
                    send_directory_start(&mut wire, remote_directory, &name, &metadata, options.preserve_times).await?;
                    tally.landed(relative.clone(), 0, true);
                    // The close is queued before the contents, so it comes out of the stack after
                    // them and after everything they push in turn.
                    work.push(Step::Close);
                    let mut children = children_of(&path, remote_directory).await?;
                    children.sort_by(|left, right| left.0.cmp(&right.0));
                    for (child_path, child_metadata) in children.into_iter().rev() {
                        let Some(child_name) = child_path.file_name().map(|name| name.to_string_lossy().into_owned())
                        else {
                            continue;
                        };
                        work.push(Step::Entry {
                            path: child_path,
                            relative: beneath(&relative, &child_name),
                            metadata: child_metadata,
                        });
                    }
                } else if metadata.is_file() {
                    let bytes = send_file(
                        &mut wire,
                        remote_directory,
                        &path,
                        &metadata,
                        &name,
                        options.preserve_times,
                        &mut tally,
                    )
                    .await?;
                    tally.landed(relative, bytes, false);
                } else {
                    tally.skipped();
                }
            }
        }
    }

    Ok(tally.finish())
}

impl SshSession {
    /// Downloads one far-side path into a local directory, over SCP.
    ///
    /// The whole conversation is bounded by `timeout`, including opening the channel: a server that
    /// accepts the connection and then says nothing is the case the timeout is for. A transfer that
    /// runs out of time is abandoned rather than closed politely — whatever is on the far side is
    /// not going to answer, and closing the channel is what ends it.
    pub async fn scp_download(
        &self,
        remote_path: &str,
        into: &Path,
        options: &TransferOptions,
        timeout: Duration,
        progress: Option<ByteProgress<'_>>,
    ) -> Result<TransferReport, ShellError> {
        let command = download_command(remote_path, options)?;
        let transfer = async {
            let stream = self.open_exec_stream(&command).await?;
            receive(stream, remote_path, into, options, progress).await
        };
        match tokio::time::timeout(timeout, transfer).await {
            Ok(result) => result,
            Err(_) => Err(ShellError::ssh_timeout(&self.host, self.port, whole_seconds(timeout))),
        }
    }

    /// Sends one local path into a far-side directory, over SCP.
    pub async fn scp_upload(
        &self,
        source: &Path,
        remote_directory: &str,
        options: &TransferOptions,
        timeout: Duration,
        progress: Option<ByteProgress<'_>>,
    ) -> Result<TransferReport, ShellError> {
        let command = upload_command(remote_directory, options)?;
        let transfer = async {
            let stream = self.open_exec_stream(&command).await?;
            send(stream, source, remote_directory, options, progress).await
        };
        match tokio::time::timeout(timeout, transfer).await {
            Ok(result) => result,
            Err(_) => Err(ShellError::ssh_timeout(&self.host, self.port, whole_seconds(timeout))),
        }
    }
}

/// A directory that is open, and the name it has inside the transfer.
struct OpenDirectory {
    path: PathBuf,
    relative: String,
    /// The mode to give it once it is full. `None` is the destination itself, which is the
    /// caller's directory and not one this transfer may re-permission.
    mode: Option<u32>,
}

/// One thing still to send, in the order a depth-first walk needs them.
enum Step {
    Entry {
        path: PathBuf,
        relative: String,
        metadata: std::fs::Metadata,
    },
    /// The end of the directory whose entries have all been sent.
    Close,
}

/// Bytes and entries, kept together so the helpers do not each need a report and a callback.
struct Tally<'a> {
    report: TransferReport,
    progress: Option<ByteProgress<'a>>,
}

impl Tally<'_> {
    fn moved(&mut self, bytes: u64) {
        self.report.bytes += bytes;
        if let Some(callback) = &mut self.progress {
            (**callback)(self.report.bytes);
        }
    }

    fn landed(&mut self, path: String, bytes: u64, directory: bool) {
        self.report.entries.push(TransferredEntry { path, bytes, directory });
    }

    fn skipped(&mut self) {
        self.report.skipped += 1;
    }

    fn finish(self) -> TransferReport {
        self.report
    }
}

/// A `C` record: one file.
struct FileRecord {
    mode: u32,
    size: u64,
    name: String,
}

impl FileRecord {
    /// Parses what follows the `C`: `<octal mode> <size> <name>`.
    ///
    /// The name is everything after the second space and may contain spaces of its own; the
    /// protocol has no escape for them, which is why the name is always last.
    fn parse(rest: &str) -> Result<Self, &'static str> {
        let mut fields = rest.splitn(3, ' ');
        let mode = fields.next().ok_or("a file record without a mode")?;
        let size = fields.next().ok_or("a file record without a size")?;
        let name = fields.next().ok_or("a file record without a name")?;
        let mode = u32::from_str_radix(mode, 8).map_err(|_| "a file record whose mode is not octal")? & 0o7777;
        let size = size
            .parse::<u64>()
            .map_err(|_| "a file record whose size is not a number")?;
        if name.is_empty() {
            return Err("a file record whose name is empty");
        }
        Ok(Self {
            mode,
            size,
            name: name.to_owned(),
        })
    }
}

/// A `D` record: one directory.
struct DirectoryRecord {
    mode: u32,
    name: String,
}

impl DirectoryRecord {
    /// Parses what follows the `D`: `<octal mode> <size> <name>`, where the size is always zero and
    /// is not used for anything.
    fn parse(rest: &str) -> Result<Self, &'static str> {
        let mut fields = rest.splitn(3, ' ');
        let mode = fields.next().ok_or("a directory record without a mode")?;
        fields.next().ok_or("a directory record without a size")?;
        let name = fields.next().ok_or("a directory record without a name")?;
        let mode = u32::from_str_radix(mode, 8).map_err(|_| "a directory record whose mode is not octal")? & 0o7777;
        if name.is_empty() {
            return Err("a directory record whose name is empty");
        }
        Ok(Self {
            mode,
            name: name.to_owned(),
        })
    }
}

/// Writes one received file and acknowledges it.
async fn receive_file<IO>(
    wire: &mut Wire<IO>,
    remote: &str,
    target: &Path,
    relative: &str,
    record: &FileRecord,
    tally: &mut Tally<'_>,
    buffer: &mut [u8],
) -> Result<(), ShellError>
where
    IO: AsyncRead + AsyncWrite,
{
    // Something of a different shape already there is refused rather than replaced: a directory
    // standing where the file goes, or a link pointing somewhere else, is exactly the case in which
    // writing through the name would put bytes outside the destination.
    match fs::symlink_metadata(target).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(ShellError::scp_transfer_failed(
                remote,
                format!("`{relative}` is a symbolic link here, so it was not written through"),
            ));
        }
        Ok(metadata) if metadata.is_dir() => {
            return Err(ShellError::scp_transfer_failed(
                remote,
                format!("a directory is already in the way of `{relative}`"),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(local_failure(remote, relative, "it could not be checked", &error)),
    }

    let part = part_path(target);
    let mut file = fs::File::create(&part)
        .await
        .map_err(|error| local_failure(remote, relative, "it could not be created", &error))?;

    // Only now is the source allowed to send the bytes.
    wire.acknowledge(remote, "a file record").await?;

    let outcome = fill_file(wire, remote, &mut file, record, tally, buffer).await;
    // Windows refuses to rename over a name that is still open, and the handle is not needed for
    // anything after this.
    drop(file);
    if let Err(error) = outcome {
        let _ = fs::remove_file(&part).await;
        return Err(error);
    }

    if let Err(error) = apply_mode(&part, record.mode).await {
        let _ = fs::remove_file(&part).await;
        return Err(local_failure(
            remote,
            relative,
            "its permissions could not be set",
            &error,
        ));
    }
    if let Err(error) = fs::rename(&part, target).await {
        let _ = fs::remove_file(&part).await;
        return Err(local_failure(remote, relative, "it could not be put in place", &error));
    }

    // The acknowledgement says the bytes are on disk, not merely that they were read.
    wire.acknowledge(remote, "the end of a file").await?;
    tally.landed(relative.to_owned(), record.size, false);
    Ok(())
}

/// Reads a file's payload, its closing byte, and the source's trailing zero.
///
/// The closing acknowledgement is not sent here: it belongs after the file is in place, which is the
/// caller's next step. Nothing between here and there reads from the wire, so the source is waiting
/// exactly as it would be.
async fn fill_file<IO>(
    wire: &mut Wire<IO>,
    remote: &str,
    file: &mut fs::File,
    record: &FileRecord,
    tally: &mut Tally<'_>,
    buffer: &mut [u8],
) -> Result<(), ShellError>
where
    IO: AsyncRead + AsyncWrite,
{
    let mut remaining = record.size;
    while remaining > 0 {
        let take = remaining.min(buffer.len() as u64) as usize;
        wire.payload(&mut buffer[..take], remote, "the file's bytes").await?;
        file.write_all(&buffer[..take])
            .await
            .map_err(|error| local_failure(remote, &record.name, "it could not be written", &error))?;
        remaining -= take as u64;
        tally.moved(take as u64);
    }
    file.flush()
        .await
        .map_err(|error| local_failure(remote, &record.name, "it could not be written", &error))?;

    // A source ends every file with a zero byte of its own, then waits. It is not an
    // acknowledgement — nothing has been acknowledged yet at that point.
    match wire.reply().await {
        Ok(Reply::Acknowledged) => Ok(()),
        Ok(Reply::Refused { fatal, message }) => Err(refusal(remote, fatal, &message)),
        Ok(Reply::Ended) => Err(ShellError::scp_transfer_failed(
            remote,
            format!(
                "the connection ended before the last byte of `{}` was confirmed",
                record.name
            ),
        )),
        Err(failure) => Err(failure.error(remote, "the end of a file")),
    }
}

/// Creates a received directory, reusing one that is already there, as `scp` itself does.
async fn open_directory(remote: &str, target: &Path, relative: &str) -> Result<(), ShellError> {
    match fs::symlink_metadata(target).await {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(ShellError::scp_transfer_failed(
            remote,
            format!("`{relative}` is a symbolic link here, so it was not written through"),
        )),
        Ok(metadata) if !metadata.is_dir() => Err(ShellError::scp_transfer_failed(
            remote,
            format!("a file is already in the way of `{relative}`"),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => fs::create_dir(target)
            .await
            .map_err(|error| local_failure(remote, relative, "it could not be created", &error)),
        Err(error) => Err(local_failure(remote, relative, "it could not be checked", &error)),
    }
}

/// Sends one file record, its payload, its closing byte and waits for the sink to agree.
async fn send_file<IO>(
    wire: &mut Wire<IO>,
    remote: &str,
    source: &Path,
    metadata: &std::fs::Metadata,
    name: &str,
    preserve_times: bool,
    tally: &mut Tally<'_>,
) -> Result<u64, ShellError>
where
    IO: AsyncRead + AsyncWrite,
{
    if preserve_times {
        send_time_record(wire, remote, metadata).await?;
    }

    let size = metadata.len();
    let mode = mode_of(metadata);
    wire.write(format!("C{mode:04o} {size} {name}\n").as_bytes())
        .await
        .map_err(|failure| failure.error(remote, "a file record"))?;
    expect_acknowledgement(wire, remote, "take a file record").await?;

    let mut file = fs::File::open(source)
        .await
        .map_err(|error| local_failure(remote, name, "it could not be opened to send", &error))?;
    let mut remaining = size;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    while remaining > 0 {
        let take = remaining.min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..take])
            .await
            .map_err(|error| local_failure(remote, name, "it could not be read", &error))?;
        if read == 0 {
            return Err(ShellError::scp_transfer_failed(
                remote,
                format!("`{name}` became shorter while it was being sent"),
            ));
        }
        wire.write(&buffer[..read])
            .await
            .map_err(|failure| failure.error(remote, "a file's bytes"))?;
        remaining -= read as u64;
        tally.moved(read as u64);
    }

    // The source's own end-of-file byte, distinct from the sink's acknowledgements.
    wire.write(&[ACK])
        .await
        .map_err(|failure| failure.error(remote, "the end of a file"))?;
    expect_acknowledgement(wire, remote, "take the end of a file").await?;
    Ok(size)
}

/// Sends the `D` record that opens a directory.
async fn send_directory_start<IO>(
    wire: &mut Wire<IO>,
    remote: &str,
    name: &str,
    metadata: &std::fs::Metadata,
    preserve_times: bool,
) -> Result<(), ShellError>
where
    IO: AsyncRead + AsyncWrite,
{
    if preserve_times {
        send_time_record(wire, remote, metadata).await?;
    }
    let mode = mode_of(metadata);
    wire.write(format!("D{mode:04o} 0 {name}\n").as_bytes())
        .await
        .map_err(|failure| failure.error(remote, "a directory record"))?;
    expect_acknowledgement(wire, remote, "take a directory record").await
}

/// Sends `T<mtime> 0 <atime> 0`, in whole seconds.
///
/// The two zeros are microseconds, and this side has nothing to say about them. The atime it sends
/// is the mtime: a modification time is what it has without another dependency, and saying the same
/// thing twice is more honest than a made-up second number.
async fn send_time_record<IO>(wire: &mut Wire<IO>, remote: &str, metadata: &std::fs::Metadata) -> Result<(), ShellError>
where
    IO: AsyncRead + AsyncWrite,
{
    let seconds = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    wire.write(format!("T{seconds} 0 {seconds} 0\n").as_bytes())
        .await
        .map_err(|failure| failure.error(remote, "a time record"))?;
    expect_acknowledgement(wire, remote, "take a time record").await
}

/// Reads a directory's entries, without following the links in it.
async fn children_of(directory: &Path, remote: &str) -> Result<Vec<(PathBuf, std::fs::Metadata)>, ShellError> {
    let mut listing = fs::read_dir(directory)
        .await
        .map_err(|error| local_failure(remote, "a directory being sent", "it could not be listed", &error))?;
    let mut children = Vec::new();
    while let Some(entry) = listing
        .next_entry()
        .await
        .map_err(|error| local_failure(remote, "a directory being sent", "it could not be listed", &error))?
    {
        // `DirEntry::metadata` does not follow a link, which is what decides whether the entry is
        // skipped below.
        let metadata = entry
            .metadata()
            .await
            .map_err(|error| local_failure(remote, "an entry being sent", "it could not be read", &error))?;
        children.push((entry.path(), metadata));
    }
    Ok(children)
}

/// The byte-level shape of the conversation: records on the way in, records and acks on the way out.
struct Wire<IO> {
    reader: BufReader<ReadHalf<IO>>,
    writer: WriteHalf<IO>,
}

impl<IO> Wire<IO>
where
    IO: AsyncRead + AsyncWrite,
{
    fn new(io: IO) -> Self {
        let (reader, writer) = tokio::io::split(io);
        Self {
            reader: BufReader::with_capacity(CHUNK_BYTES, reader),
            writer,
        }
    }

    /// One byte, or `None` when the stream has ended.
    async fn octet(&mut self) -> Result<Option<u8>, Failure> {
        let mut byte = [0u8; 1];
        match self.reader.read(&mut byte).await {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(byte[0])),
            Err(error) => Err(Failure::Io(error)),
        }
    }

    /// A record, up to and without its newline.
    async fn line(&mut self) -> Result<Option<String>, Failure> {
        let mut line = Vec::new();
        loop {
            match self.octet().await? {
                None if line.is_empty() => return Ok(None),
                None => return Err(Failure::Cut),
                Some(b'\n') => {
                    return String::from_utf8(line).map(Some).map_err(|_| Failure::NotText);
                }
                // No record contains one: SCP's own words are text, and a name is a path component
                // rather than a blob. Reading on would turn a desync into a very long name.
                Some(0) => return Err(Failure::NotARecord(0)),
                Some(byte) => {
                    if line.len() >= MAX_RECORD_BYTES {
                        return Err(Failure::TooLong);
                    }
                    line.push(byte);
                }
            }
        }
    }

    /// A record that must be there, because the byte that introduced it already was.
    async fn record(&mut self, remote: &str, what: &str) -> Result<String, ShellError> {
        match self.line().await {
            Ok(Some(line)) => Ok(line),
            Ok(None) => Err(ShellError::scp_transfer_failed(
                remote,
                format!("the connection ended in the middle of {what}"),
            )),
            Err(failure) => Err(failure.error(remote, what)),
        }
    }

    /// Exactly as many payload bytes as the far side announced.
    async fn payload(&mut self, buffer: &mut [u8], remote: &str, what: &str) -> Result<(), ShellError> {
        match self.reader.read_exact(buffer).await {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => Err(ShellError::scp_transfer_failed(
                remote,
                format!("the connection ended in the middle of {what}"),
            )),
            Err(error) => Err(Failure::Io(error).error(remote, what)),
        }
    }

    /// The far side's answer to whatever was just said.
    async fn reply(&mut self) -> Result<Reply, Failure> {
        match self.octet().await? {
            Some(ACK) => Ok(Reply::Acknowledged),
            Some(level @ (WARNING | FATAL)) => {
                // The words are one line; an empty message is still an answer.
                let message = self.line().await?.unwrap_or_default();
                Ok(Reply::Refused {
                    fatal: level == FATAL,
                    message,
                })
            }
            Some(other) => Err(Failure::NotARecord(other)),
            None => Ok(Reply::Ended),
        }
    }

    async fn acknowledge(&mut self, remote: &str, what: &str) -> Result<(), ShellError> {
        match self.write(&[ACK]).await {
            Ok(()) => Ok(()),
            Err(failure) => Err(failure.error(remote, what)),
        }
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        self.writer.write_all(bytes).await.map_err(Failure::Io)?;
        // Nothing is buffered on this side, but a channel that accepts a write and holds it until
        // something else happens is the same as an acknowledgement that never arrives.
        self.writer.flush().await.map_err(Failure::Io)
    }
}

/// What the far side said to a record.
enum Reply {
    /// The one byte that means "go on".
    Acknowledged,
    /// The far side's own refusal, with the byte that says whether it is fatal.
    Refused { fatal: bool, message: String },
    /// The stream ended instead.
    Ended,
}

/// Waits for the sink's acknowledgement of the last record.
async fn expect_acknowledgement<IO>(wire: &mut Wire<IO>, remote: &str, what: &str) -> Result<(), ShellError>
where
    IO: AsyncRead + AsyncWrite,
{
    match wire.reply().await {
        Ok(Reply::Acknowledged) => Ok(()),
        Ok(Reply::Refused { fatal, message }) => Err(refusal(remote, fatal, &message)),
        Ok(Reply::Ended) => Err(ShellError::scp_transfer_failed(
            remote,
            format!("the connection ended before the far side could {what}"),
        )),
        Err(failure) => Err(failure.error(remote, &format!("waiting for the far side to {what}"))),
    }
}

/// Reads that could not complete, in terms the caller can read.
enum Failure {
    /// The stream ended in the middle of something it had already begun.
    Cut,
    /// A record that never ended.
    TooLong,
    /// A byte this protocol does not put there.
    NotARecord(u8),
    /// A record that is not text, which means a name this side would have to rewrite to keep.
    NotText,
    /// The stream itself failed.
    Io(std::io::Error),
}

impl Failure {
    fn error(self, remote: &str, what: &str) -> ShellError {
        ShellError::scp_transfer_failed(remote, format!("{what} failed: {self}"))
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cut => formatter.write_str("the connection ended in the middle of a record"),
            Self::TooLong => {
                formatter.write_str("a record arrived without an end, so the far side may not be running scp")
            }
            Self::NotARecord(byte) => {
                write!(formatter, "0x{byte:02x} is not a record or an acknowledgement")
            }
            Self::NotText => {
                formatter.write_str("a record was not text, and a name that is not text would have to be renamed")
            }
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

/// The far side's own complaint, made into an envelope.
fn refusal(remote: &str, fatal: bool, message: &str) -> ShellError {
    let level = if fatal { "fatal" } else { "warning" };
    let message = message.trim();
    if message.is_empty() {
        ShellError::scp_transfer_failed(remote, format!("the far side sent a {level} error with no message"))
    } else {
        ShellError::scp_transfer_failed(remote, format!("the far side sent a {level} error: {message}"))
    }
}

/// A record this side cannot make sense of.
fn malformed(remote: &str, reason: &str) -> ShellError {
    ShellError::scp_transfer_failed(
        remote,
        format!("the far side sent a record this side cannot read: {reason}"),
    )
}

/// A failure on this machine, named by the far side's name for the thing rather than by a local
/// path: the interface never sees where anything is kept here.
fn local_failure(remote: &str, name: &str, what: &str, error: &std::io::Error) -> ShellError {
    ShellError::scp_transfer_failed(remote, format!("`{name}`: {what}: {error}"))
}

/// Checks a path this side is about to hand to a shell on the far side.
fn check_remote_path(path: &str) -> Result<(), ShellError> {
    if path.is_empty() {
        return Err(ShellError::app_internal(
            "an SCP transfer was asked for without a remote path",
        ));
    }
    if path.contains('\0') {
        return Err(ShellError::app_internal(
            "an SCP transfer was asked for a path containing a NUL",
        ));
    }
    Ok(())
}

/// One shell word, whatever the path contains.
///
/// Everything inside single quotes is literal to a POSIX shell, including `$`, backticks and
/// backslashes; the only character that cannot appear is the quote itself, which is closed,
/// escaped and reopened. This is the whole of the injection defence: a remote name may be anything,
/// and after this it is still one argument.
fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
}

/// A name from the far side, held to being exactly one path component.
///
/// Not cosmetic: a `C` record is a line a peer chooses, so `../../.ssh/authorized_keys` is a name it
/// may send, and joining that to the destination would write outside the directory a person picked.
/// Refusing is the only answer that keeps the destination a boundary.
fn part_name<'a>(name: &'a str, remote: &str) -> Result<&'a str, ShellError> {
    let refusal = |reason: String| ShellError::scp_transfer_failed(remote, reason);

    if name.is_empty() {
        return Err(refusal("the far side sent an empty name".to_owned()));
    }
    if name == "." || name == ".." {
        return Err(refusal(format!("the far side sent `{name}`, which is not a name")));
    }
    if name.contains('\0') {
        return Err(refusal(format!("a name from the far side contains a NUL: `{name}`")));
    }
    // Both separators on every platform: this side may be Windows, where a backslash is one too,
    // and a name that is a path is not a name wherever it is being written.
    if name.contains('/') || name.contains('\\') || (cfg!(windows) && name.contains(':')) {
        return Err(refusal(format!("`{name}` is a path rather than a name")));
    }
    Ok(name)
}

/// Confirms a joined path is still the caller's directory's business.
///
/// [`part_name`] already makes this true; the check stays because it is the property that matters,
/// and a future change to how names are cleaned up should fail here rather than at the file system.
fn inside(root: &Path, candidate: &Path, remote: &str) -> Result<(), ShellError> {
    if candidate.starts_with(root) {
        Ok(())
    } else {
        Err(ShellError::scp_transfer_failed(
            remote,
            "a name from the far side resolved outside the destination directory",
        ))
    }
}

/// The directory a record belongs in, and its name inside the transfer.
fn current_directory(open: &[OpenDirectory], remote: &str) -> Result<(PathBuf, String), ShellError> {
    match open.last() {
        Some(frame) => Ok((frame.path.clone(), frame.relative.clone())),
        // The stack starts with the destination and only ever pops below two, so this is a guard
        // against a panic rather than a state the loop can reach.
        None => Err(ShellError::scp_transfer_failed(
            remote,
            "the transfer lost the directory it was writing into",
        )),
    }
}

/// A name inside a transfer: the enclosing one, a slash, and this one.
fn beneath(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// Where a file is written while it is still arriving.
fn part_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(PART_SUFFIX);
    PathBuf::from(name)
}

/// Creates the destination if it is not there, and refuses one that is not a directory.
async fn prepare_destination(destination: &Path, remote: &str) -> Result<(), ShellError> {
    match fs::metadata(destination).await {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(ShellError::scp_transfer_failed(
            remote,
            "the destination of this download is not a directory",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => fs::create_dir_all(destination).await.map_err(|error| {
            ShellError::scp_transfer_failed(remote, format!("the destination could not be created: {error}"))
        }),
        Err(error) => Err(ShellError::scp_transfer_failed(
            remote,
            format!("the destination could not be checked: {error}"),
        )),
    }
}

/// A `T` record is four numbers; nothing else about it is used.
fn check_time_record(rest: &str, remote: &str) -> Result<(), ShellError> {
    let fields: Vec<&str> = rest.split(' ').collect();
    if fields.len() != 4 || fields.iter().any(|field| field.parse::<u64>().is_err()) {
        return Err(malformed(remote, "a time record is four whole numbers"));
    }
    Ok(())
}

/// The name a local path is sent under, held to what a record can carry.
fn wire_name(path: &Path, remote: &str) -> Result<String, ShellError> {
    let refusal = |reason: String| ShellError::scp_transfer_failed(remote, reason);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| refusal("a name to send is not text, and rewriting it would rename the file".to_owned()))?;
    if name.is_empty() {
        return Err(refusal("a name to send is empty".to_owned()));
    }
    if name.contains(['\n', '\r', '\0']) {
        return Err(refusal(format!(
            "`{name}` contains a character the protocol uses to end a record"
        )));
    }
    Ok(name.to_owned())
}

/// The permission bits to send for a local file or directory.
#[cfg(unix)]
fn mode_of(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

/// Windows has no permission bits to send, so it says what a file of that shape usually is.
#[cfg(not(unix))]
fn mode_of(metadata: &std::fs::Metadata) -> u32 {
    if metadata.is_dir() { 0o755 } else { 0o644 }
}

/// Gives a received file or directory the mode the far side sent.
#[cfg(unix)]
async fn apply_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await
}

/// Nothing to apply: Windows has no equivalent of the bits a `C` record carries.
#[cfg(not(unix))]
async fn apply_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_path_is_one_shell_word_whatever_is_in_it() {
        assert_eq!(quote("/var/log/nginx"), "'/var/log/nginx'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote("x; rm -rf /"), "'x; rm -rf /'");
        assert_eq!(quote("$(reboot)"), "'$(reboot)'");
        assert_eq!(quote("a b`c`d"), "'a b`c`d'");
        assert_eq!(quote("项目/日志"), "'项目/日志'");
    }

    #[test]
    fn a_download_asks_for_the_path_after_the_end_of_the_options() {
        let options = TransferOptions {
            recursive: true,
            ..TransferOptions::default()
        };
        assert_eq!(
            download_command("/srv/a b", &options).expect("a path"),
            "scp -r -f -- '/srv/a b'"
        );
        assert_eq!(
            download_command("-rf", &TransferOptions::default()).expect("a path"),
            "scp -f -- '-rf'"
        );
    }

    #[test]
    fn an_upload_only_asks_the_far_side_to_preserve_times_when_it_will_send_them() {
        assert_eq!(
            upload_command("/srv", &TransferOptions::default()).expect("a path"),
            "scp -t -- '/srv'"
        );
        let options = TransferOptions {
            preserve_times: true,
            ..TransferOptions::default()
        };
        assert_eq!(upload_command("/srv", &options).expect("a path"), "scp -p -t -- '/srv'");
    }

    #[test]
    fn a_recursive_upload_tells_the_sink_it_will_send_directories() {
        // A sink without `-r` treats a `D` record as a protocol violation, so this flag is what
        // makes a directory upload possible at all.
        let options = TransferOptions {
            recursive: true,
            ..TransferOptions::default()
        };
        assert_eq!(upload_command("/srv", &options).expect("a path"), "scp -r -t -- '/srv'");
    }

    #[test]
    fn a_name_from_the_far_side_is_never_a_path() {
        for refused in ["..", ".", "", "a/b", "../x", "/etc/passwd", "a\\b"] {
            assert!(
                part_name(refused, "/remote").is_err(),
                "`{refused}` should not have been accepted as a name"
            );
        }
        assert_eq!(part_name("a b", "/remote").expect("a name"), "a b");
    }

    #[test]
    fn a_file_record_keeps_spaces_in_the_name() {
        let record = FileRecord::parse("0644 12 two words.txt").expect("a file record");
        assert_eq!(record.mode, 0o644);
        assert_eq!(record.size, 12);
        assert_eq!(record.name, "two words.txt");
    }

    #[test]
    fn a_record_whose_mode_is_not_octal_is_refused() {
        assert!(FileRecord::parse("0899 1 x").is_err());
        assert!(DirectoryRecord::parse("0644 0").is_err());
        assert!(check_time_record("1700000000 0 1700000000 0", "/remote").is_ok());
        assert!(check_time_record("1700000000 0 1700000000", "/remote").is_err());
    }

    #[test]
    fn a_name_inside_a_directory_is_joined_with_the_far_sides_separator() {
        assert_eq!(beneath("", "top"), "top");
        assert_eq!(beneath("top", "file"), "top/file");
    }
}

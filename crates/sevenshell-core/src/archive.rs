//! Packed transfers: one archive across the wire instead of a directory's worth of files.
//!
//! The archive that crosses the wire is always made by the *far side's* own `tar`, and it is always a
//! *file* — it never travels through a command channel. That is the whole design, and it is worth
//! stating because the obvious alternative is worse in every direction: piping `tar czf -` through an
//! exec channel would need a streaming channel that does not exist yet, would meet the four-megabyte
//! output limit the first time somebody packed a real directory, and would leave the progress bar with
//! nothing to count. As a file, the archive goes down the same SFTP path every other transfer uses,
//! which already reports progress, honours cancellation and maps a refusal to an envelope.
//!
//! What this module holds is everything else those two directions need:
//!
//! - **The command line**, and the one thing that must not be got wrong about it: a path
//!   interpolated into a shell command is a path somebody else chose. The directory being packed can
//!   be called `x; rm -rf /`, and [`shell_quote`] is what stands between that name and the shell that
//!   would obey it.
//! - **The packing and unpacking done on this side.** An upload starts from ordinary files a person
//!   chose, not from an archive they made by hand, so this machine has to make one; a download ends
//!   with the archive opened here, because what somebody asked for is the folder, not a tarball to
//!   unpack themselves. Both are why `tar` and `flate2` are dependencies of this crate.
//!
//! An archive made here is a label as much as a file: it is unpacked on the other side and deleted at
//! both ends, so the name it is given in the transfer list is read by a person, not by a program.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sevenshell_error::ShellError;
use sevenshell_ssh::{ExecOutcome, SshSession};

/// How long the far side's `tar` is given.
///
/// Generous on purpose: the machine that packs is the machine being read from, and a large directory
/// on a slow disk is a long command rather than a hung one. This is the budget for a `tar` that has
/// stopped answering, not a hurry for one that is working.
pub const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(600);

/// How much of `tar`'s own complaint is kept when it fails.
///
/// Enough for the reason and the file it is about — the first line or two — and not enough to paste a
/// wall of text into an error envelope.
const MAX_REPORT: usize = 400;

/// Quotes a value for the shell that will run `tar`.
///
/// Single quotes, with the one escape a single-quoted string has: a quote inside ends the string, is
/// written as an escaped quote, and the string starts again. Everything else — spaces, `$`, `;`,
/// backticks, newlines — is literal inside single quotes, which is the point.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The command that packs one entry, as seen from the directory that holds it.
///
/// `-C` into the parent and the entry's own name afterwards, so what the archive stores is the
/// directory itself: unpacking it somewhere gives back `logs/` rather than the contents of `logs/`
/// strewn into wherever somebody happened to be standing.
pub fn pack_command(archive: &str, parent: &str, name: &str) -> String {
    format!(
        "tar czf {} -C {} {}",
        shell_quote(archive),
        shell_quote(parent),
        shell_quote(name)
    )
}

/// The command that unpacks an archive into a directory.
pub fn unpack_command(archive: &str, directory: &str) -> String {
    format!("tar xzf {} -C {}", shell_quote(archive), shell_quote(directory))
}

/// The name a directory's archive is offered under, which is what the save dialog suggests.
pub fn archive_name(directory: &str) -> String {
    format!("{directory}.tar.gz")
}

/// What a packed upload's archive is called in the transfer list, and it is a label rather than a
/// file: the archive is made here, sent, unpacked there, and deleted at both ends.
///
/// A batch of files has no name of its own the way a directory does, so the name has to say what
/// happened instead — that this was packed, and how much went into it. Somebody who chose six files
/// and then wonders whether the switch did anything is answered by the row itself.
pub fn uploaded_name(files: usize) -> String {
    let noun = if files == 1 { "file" } else { "files" };
    format!("upload_pack_{files}_{noun}.tar.gz")
}

/// Where an archive waits on this machine while a packed transfer is using it.
///
/// The system's temporary directory, named after the transfer: two packed transfers at once must not
/// write over each other's archive, and neither of them should leave one behind.
pub fn local_staging_path(token: &str) -> PathBuf {
    std::env::temp_dir().join(format!("7shell-{token}.tar.gz"))
}

/// Packs local files into one archive, and answers how large it turned out.
///
/// The entries are stored under their own file names rather than their paths: the archive is unpacked
/// into a directory somebody is looking at, and `Documents/report.pdf` appearing inside it would be a
/// tree nobody asked for.
///
/// Links are stored rather than followed, which is what the far side's `tar` does as well: a link to
/// a directory is a link, not a second copy of everything behind it.
pub fn pack_local(archive: &Path, sources: &[PathBuf]) -> Result<u64, ShellError> {
    let mut seen: Vec<String> = Vec::new();
    for source in sources {
        let name = file_name_of(source)?;
        if seen.contains(&name) {
            return Err(ShellError::sftp_archive_failed(
                &name,
                "two of the chosen files have this name, and one would land on top of the other",
            ));
        }
        seen.push(name);
    }

    let file = std::fs::File::create(archive)
        .map_err(|error| local_failure(archive, "the archive could not be written", error))?;
    let mut builder = tar::Builder::new(GzEncoder::new(file, Compression::default()));
    builder.follow_symlinks(false);

    for (source, name) in sources.iter().zip(seen.iter()) {
        builder
            .append_path_with_name(source, name)
            .map_err(|error| local_failure(archive, "a file could not be added to the archive", error))?;
    }

    let encoder = builder
        .into_inner()
        .map_err(|error| local_failure(archive, "the archive could not be finished", error))?;
    let mut encoder = encoder;
    encoder
        .flush()
        .map_err(|error| local_failure(archive, "the archive could not be written", error))?;
    let file = encoder
        .finish()
        .map_err(|error| local_failure(archive, "the archive could not be written", error))?;
    file.sync_all()
        .map_err(|error| local_failure(archive, "the archive could not be written", error))?;

    std::fs::metadata(archive)
        .map(|metadata| metadata.len())
        .map_err(|error| local_failure(archive, "the archive could not be read back", error))
}

/// Unpacks an archive into a directory.
///
/// Nothing is allowed to land outside that directory: the `tar` crate refuses an entry that is
/// absolute or climbs out with `..`, which is the check that stops an archive from writing anywhere
/// it likes. The archive here was made by this build a moment ago, so it is not the threat — but
/// "an archive we made ourselves" is exactly the assumption that stops being true the day somebody
/// can hand us one, and the check costs nothing.
pub fn unpack_local(archive: &Path, into: &Path) -> Result<(), ShellError> {
    let file = std::fs::File::open(archive)
        .map_err(|error| local_failure(archive, "the archive could not be opened", error))?;
    let mut reader = tar::Archive::new(GzDecoder::new(file));

    reader
        .unpack(into)
        .map_err(|error| local_failure(archive, "the archive could not be unpacked", error))
}

/// The name of a local file, which is the most of a local path an envelope is ever allowed to hold.
fn file_name_of(path: &Path) -> Result<String, ShellError> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| ShellError::sftp_archive_failed("(a path with no name)", "it cannot be packed"))
}

/// A failure about a local file, named and not located.
///
/// The path parameter of the envelope is the file's *name*: an error is shown in the window and
/// quoted in reports, and where somebody keeps their files is not the interface's business.
fn local_failure(path: &Path, what: &str, error: std::io::Error) -> ShellError {
    let name = path.file_name().map_or_else(
        || "(an archive with no name)".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    ShellError::sftp_archive_failed(&name, format!("{what}: {error}"))
}

/// Where an archive waits on the far side while it is being moved.
///
/// In the remote home, and dot-prefixed. Not in `/tmp`: the home is the one directory an SFTP session
/// is known to be able to write to, and a transfer must not fail because this host mounts `/tmp`
/// read-only or is one of the machines that has none.
pub fn staging_path(home: &str, token: &str) -> String {
    let separator = if home.ends_with('/') { "" } else { "/" };
    format!("{home}{separator}.7shell-{token}.tar.gz")
}

/// Runs one packing or unpacking step, and refuses to pretend it worked.
///
/// The exit status is the answer, not the fact that the command ran: a `tar` that could not find what
/// it was pointed at still exits 1 with its complaint on stderr, and a transfer that reported success
/// for that would be a lie with an empty file at the end of it.
pub async fn run(command: &str, connection: &SshSession, path: &str) -> Result<(), ShellError> {
    let outcome = connection.exec(command, ARCHIVE_TIMEOUT).await?;
    if outcome.succeeded() {
        return Ok(());
    }

    Err(ShellError::sftp_archive_failed(
        path,
        format!("`tar` exited with {}: {}", status_of(&outcome), report_of(&outcome)),
    ))
}

/// What `tar` said, on one line and cut short. Its complaints are multi-line, and an envelope is not
/// a place to paste a wall of text.
fn report_of(outcome: &ExecOutcome) -> String {
    let spoken = if outcome.stderr.trim().is_empty() {
        outcome.stdout.as_str()
    } else {
        outcome.stderr.as_str()
    };
    let flattened = spoken.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.is_empty() {
        return "it said nothing".to_owned();
    }
    match flattened.char_indices().nth(MAX_REPORT) {
        Some((cut, _)) => format!("{}…", &flattened[..cut]),
        None => flattened,
    }
}

/// The exit status, or the fact that there was not one — a command cut off by the output limit, or
/// one whose channel closed early, has no status to report.
fn status_of(outcome: &ExecOutcome) -> String {
    outcome
        .exit_code
        .map_or_else(|| "no status".to_owned(), |code| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_is_quoted_without_being_changed() {
        assert_eq!(shell_quote("/var/log/nginx"), "'/var/log/nginx'");
    }

    #[test]
    fn a_quote_in_a_name_cannot_end_the_command() {
        // The classic escape: `'` closes the string, `\'` is a literal quote, and the string opens
        // again. Without it, a directory called `it's` would hand the rest of the name to the shell.
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn a_name_that_holds_a_command_stays_a_name() {
        // The reason this function exists: everything inside single quotes is literal, so this is
        // one awkward directory rather than two commands.
        assert_eq!(shell_quote("x; rm -rf /"), "'x; rm -rf /'");
        assert_eq!(shell_quote("$(reboot)"), "'$(reboot)'");
        assert_eq!(shell_quote("a b`c`d"), "'a b`c`d'");
    }

    #[test]
    fn a_name_in_another_alphabet_survives_the_quoting() {
        assert_eq!(shell_quote("项目/日志"), "'项目/日志'");
    }

    #[test]
    fn packing_names_the_parent_and_the_entry() {
        assert_eq!(
            pack_command("/home/u/.7shell-x1.tar.gz", "/var/log", "nginx"),
            "tar czf '/home/u/.7shell-x1.tar.gz' -C '/var/log' 'nginx'"
        );
    }

    #[test]
    fn a_name_that_needs_quoting_is_quoted_in_the_command_too() {
        assert_eq!(
            pack_command("/tmp/a.tar.gz", "/home/u", "it's here"),
            r"tar czf '/tmp/a.tar.gz' -C '/home/u' 'it'\''s here'"
        );
    }

    #[test]
    fn unpacking_leaves_the_archive_where_it_is() {
        assert_eq!(
            unpack_command("/home/u/.7shell-x2.tar.gz", "/srv/site"),
            "tar xzf '/home/u/.7shell-x2.tar.gz' -C '/srv/site'"
        );
    }

    #[test]
    fn an_archive_is_named_after_the_directory_it_came_from() {
        assert_eq!(archive_name("logs"), "logs.tar.gz");
        assert_eq!(archive_name("项目"), "项目.tar.gz");
    }

    #[test]
    fn the_staging_file_is_hidden_and_named_after_the_transfer() {
        assert_eq!(staging_path("/home/user", "x7"), "/home/user/.7shell-x7.tar.gz");
        // A home that already ends in a slash does not get a second one.
        assert_eq!(staging_path("/", "x7"), "/.7shell-x7.tar.gz");
    }

    #[test]
    fn a_failed_step_carries_what_tar_said_and_not_the_whole_of_it() {
        let outcome = ExecOutcome {
            stdout: String::new(),
            stderr: format!("tar: thing: not found\n{}\n", "x".repeat(MAX_REPORT * 2)),
            exit_code: Some(2),
            duration: Duration::from_millis(1),
        };

        let error = report_of(&outcome);
        assert!(error.contains("tar: thing: not found"));
        assert!(error.ends_with('…'));
        assert!(error.len() < MAX_REPORT + 8);
        assert_eq!(status_of(&outcome), "2");
    }

    #[test]
    fn a_step_that_said_nothing_still_says_so() {
        let outcome = ExecOutcome {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            duration: Duration::from_millis(1),
        };

        assert_eq!(report_of(&outcome), "it said nothing");
        assert_eq!(status_of(&outcome), "no status");
    }

    /// A directory with two files in it, one of them named awkwardly — the name goes into the archive
    /// and comes back out of it, and a name with a quote in it is where a hand-rolled format breaks.
    fn sources() -> (tempfile::TempDir, Vec<PathBuf>) {
        let directory = tempfile::tempdir().expect("temp dir");
        let first = directory.path().join("notes.txt");
        let second = directory.path().join("it's a report.txt");
        std::fs::write(&first, "the first file\n").expect("write");
        std::fs::write(&second, "the second file\n").expect("write");
        (directory, vec![first, second])
    }

    #[test]
    fn a_packed_set_of_files_opens_again_on_the_other_side() {
        let (_source, files) = sources();
        let unpacked = tempfile::tempdir().expect("temp dir");
        let archive = local_staging_path("test-round-trip");

        let size = pack_local(&archive, &files).expect("pack");

        // The size is the archive's own, which is what the progress bar counts against.
        assert_eq!(size, std::fs::metadata(&archive).expect("stat").len());
        assert!(size > 20, "a gzip archive with two files in it is not this small");

        unpack_local(&archive, unpacked.path()).expect("unpack");

        assert_eq!(
            std::fs::read_to_string(unpacked.path().join("notes.txt")).expect("read"),
            "the first file\n"
        );
        assert_eq!(
            std::fs::read_to_string(unpacked.path().join("it's a report.txt")).expect("read"),
            "the second file\n"
        );
        std::fs::remove_file(&archive).ok();
    }

    #[test]
    fn the_files_are_stored_by_name_and_not_by_where_they_were() {
        let (_source, files) = sources();
        let unpacked = tempfile::tempdir().expect("temp dir");
        let archive = local_staging_path("test-by-name");

        pack_local(&archive, &files).expect("pack");
        unpack_local(&archive, unpacked.path()).expect("unpack");

        // Nothing under the archive's destination but the two names: no `Users/…/Temp/…` tree, which
        // is what storing the paths would have made.
        let mut entries: Vec<String> = std::fs::read_dir(unpacked.path())
            .expect("list")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        assert_eq!(entries, vec!["it's a report.txt".to_owned(), "notes.txt".to_owned()]);
        std::fs::remove_file(&archive).ok();
    }

    #[test]
    fn two_files_with_one_name_are_refused_before_anything_is_written() {
        let (source, _) = sources();
        let other = source.path().join("nested");
        std::fs::create_dir(&other).expect("mkdir");
        let twin = other.join("notes.txt");
        std::fs::write(&twin, "a different file with the same name\n").expect("write");

        let archive = local_staging_path("test-twins");
        let error = pack_local(&archive, &[source.path().join("notes.txt"), twin]).expect_err("the names collide");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SftpArchiveFailed);
        assert_eq!(
            error.message_params().get("path").map(String::as_str),
            Some("notes.txt"),
            "the refusal names the file that is in the way"
        );
        assert!(!archive.exists(), "nothing was written");
        // And what it does not name is where the file lives.
        assert!(
            !error
                .to_string()
                .contains(&source.path().to_string_lossy().into_owned()),
            "an envelope never carries a local path"
        );
    }

    /// Writes a tar by hand, because the builder refuses to make one that climbs out of its directory
    /// — and the point of this case is an archive that did not come from here. A download's archive
    /// comes back from the host, so "we made it ourselves" is not a property this side can rely on.
    fn write_escaping_tar(path: &Path) {
        fn put(target: &mut [u8], value: &[u8]) {
            target[..value.len()].copy_from_slice(value);
        }

        let mut header = [0u8; 512];
        put(&mut header[0..100], b"../escaped.txt");
        put(&mut header[100..108], b"0000644\0");
        put(&mut header[108..116], b"0000000\0");
        put(&mut header[116..124], b"0000000\0");
        put(&mut header[124..136], b"00000000006\0");
        put(&mut header[136..148], b"00000000000\0");
        // The checksum is computed over the header with its own field read as spaces.
        put(&mut header[148..156], b"        ");
        header[156] = b'0';
        put(&mut header[257..263], b"ustar\0");
        put(&mut header[263..265], b"00");

        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        put(&mut header[148..156], format!("{sum:06o}\0 ").as_bytes());

        let mut bytes = header.to_vec();
        bytes.extend_from_slice(b"gotcha");
        // One block of data, padded, then the two empty blocks that end an archive.
        bytes.resize(2 * 512, 0);
        bytes.resize(4 * 512, 0);
        std::fs::write(path, &bytes).expect("write the archive");
    }

    #[test]
    fn an_archive_that_climbs_out_of_its_directory_is_refused() {
        let destination = tempfile::tempdir().expect("temp dir");
        let escape = destination.path().join("escaped.txt");
        let archive = local_staging_path("test-escape");
        write_escaping_tar(&archive);

        let inside = destination.path().join("landing");
        unpack_local(&archive, &inside).expect_err("an archive may not write above where it lands");

        assert!(
            !escape.exists(),
            "nothing was written outside the directory it was given"
        );
        std::fs::remove_file(&archive).ok();
    }

    #[test]
    fn a_packed_upload_says_in_its_name_what_went_into_it() {
        assert_eq!(uploaded_name(1), "upload_pack_1_file.tar.gz");
        assert_eq!(uploaded_name(4), "upload_pack_4_files.tar.gz");
    }

    #[test]
    fn the_archive_waits_somewhere_of_its_own() {
        let path = local_staging_path("x7");
        assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
        assert_eq!(
            path.file_name().map(|name| name.to_string_lossy().into_owned()),
            Some("7shell-x7.tar.gz".to_owned())
        );
    }
}

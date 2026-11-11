//! The file that lets a bridge in another process find this one.
//!
//! The desktop is the only process with the vault and the live sessions, and `sshell-mcp` is started
//! by an MCP client that knows nothing about either. The file below is the whole handover: the port,
//! the token for this run, and enough of a timestamp to tell a live file from a leftover one.
//!
//! It is written atomically — a bridge that read a half-written file would report a broken desktop —
//! and it is owner-only on the platforms that have a mode bit. On Windows the file inherits the
//! access control list of `%APPDATA%\com.sevenshell.app`, which the operating system creates for this
//! user alone (the user, SYSTEM, and Administrators): there is no portable mode bit to set, and
//! replacing the inherited list with a hand-built one would be a security descriptor maintained by
//! hand in a program that has no other business with one.
//!
//! The token in it is a secret, so the file is removed on the way out and whenever the agent
//! interface is turned off. A leftover file names a token this run no longer accepts.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use sevenshell_api::{ENDPOINT_FILE_NAME, EndpointFile};
use sevenshell_error::ShellError;

/// Where the endpoint file lives, whether or not it exists yet.
pub fn path() -> Result<PathBuf, ShellError> {
    let directory = sevenshell_platform::config_dir()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this machine"))?;
    Ok(path_in(&directory))
}

/// The file's place inside one configuration directory.
pub fn path_in(directory: &Path) -> PathBuf {
    directory.join(ENDPOINT_FILE_NAME)
}

/// Writes one endpoint file to an explicit path.
///
/// The write is a temporary file renamed into place: rename is atomic on both filesystems this
/// application runs on, so a reader sees the previous file or this one and never a mixture.
pub fn write_to(path: &Path, endpoint: &EndpointFile) -> Result<(), ShellError> {
    let json = serde_json::to_vec(endpoint)
        .map_err(|error| ShellError::store_io(format!("the endpoint file could not be written: {error}")))?;
    let directory = path
        .parent()
        .ok_or_else(|| ShellError::store_io(format!("{} has no directory", path.display())))?;

    std::fs::create_dir_all(directory)
        .map_err(|error| ShellError::store_io(format!("cannot create {}: {error}", directory.display())))?;

    // The name is unique per attempt rather than fixed: two starts racing (a person clicking the
    // switch while startup is still finishing) must not write the same temporary file.
    let temporary = directory.join(format!(".{ENDPOINT_FILE_NAME}.{}.tmp", uuid::Uuid::new_v4().simple()));

    match write_temporary(&temporary, &json).and_then(|()| {
        std::fs::rename(&temporary, path)
            .map_err(|error| ShellError::store_io(format!("cannot replace {}: {error}", path.display())))
    }) {
        Ok(()) => Ok(()),
        Err(error) => {
            // A failed write must not leave the temporary file behind: it holds the token.
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn write_temporary(path: &Path, contents: &[u8]) -> Result<(), ShellError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }

    let mut file = options
        .open(path)
        .map_err(|error| ShellError::store_io(format!("cannot create {}: {error}", path.display())))?;
    file.write_all(contents)
        .map_err(|error| ShellError::store_io(format!("cannot write {}: {error}", path.display())))?;
    // Synced before the rename: a machine that loses power between the two would otherwise leave a
    // renamed file with none of its contents.
    file.sync_all()
        .map_err(|error| ShellError::store_io(format!("cannot flush {}: {error}", path.display())))?;
    Ok(())
}

/// Removes the endpoint file. An absent file is the normal case after a previous removal.
pub fn remove(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("{} could not be removed: {error}", path.display()),
    }
}

/// Reads an endpoint file back. The bridge has its own reader; this one is for tests.
pub fn read(path: &Path) -> Result<EndpointFile, ShellError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| ShellError::store_io(format!("cannot read {}: {error}", path.display())))?;
    serde_json::from_str(&raw)
        .map_err(|error| ShellError::store_io(format!("{} is not an endpoint file: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn an_endpoint() -> EndpointFile {
        EndpointFile::new(4711, "a-token-for-this-run", 4242, 1_700_000_000)
    }

    #[test]
    fn a_written_endpoint_reads_back_and_leaves_no_temporary_file() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());

        write_to(&path, &an_endpoint()).expect("the file is written");

        assert_eq!(read(&path).expect("readable"), an_endpoint());
        let leftovers: Vec<_> = std::fs::read_dir(directory.path())
            .expect("listable")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "a temporary file would hold the token");
    }

    #[cfg(unix)]
    #[test]
    fn the_endpoint_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());

        write_to(&path, &an_endpoint()).expect("the file is written");

        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "a token readable by another user is a token that has leaked"
        );
    }

    #[test]
    fn writing_again_replaces_the_previous_file_whole() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());

        write_to(&path, &an_endpoint()).expect("the first write");
        let rotated = EndpointFile::new(4712, "the-next-run", 4243, 1_700_000_100);
        write_to(&path, &rotated).expect("the second write");

        assert_eq!(read(&path).expect("readable"), rotated);
    }

    #[test]
    fn removing_a_file_that_is_not_there_is_not_an_error() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = path_in(directory.path());

        remove(&path);
        remove(&path);

        assert!(!path.exists());
    }
}

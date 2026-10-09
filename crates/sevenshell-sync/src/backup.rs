//! The backup file on disk: one JSON file, written in a way that cannot lose the one before it.
//!
//! A single file rather than a container, because a snapshot is a single thing. A container would
//! earn its keep the moment something binary had to travel with it — the background image is the
//! only candidate, and it is left behind on purpose — and until then it would be a second format to
//! open, with a manifest inside it saying what the file already says about itself.

use std::path::Path;

use sevenshell_error::ShellError;

use crate::snapshot::MAX_SNAPSHOT_BYTES;

/// Writes a snapshot where somebody asked for it.
///
/// The file is written beside itself and renamed, so an interrupted write leaves whatever was there
/// before rather than half of what was meant to replace it.
pub fn write(path: &Path, text: &str) -> Result<(), ShellError> {
    let bytes = text.as_bytes();
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err(ShellError::sync_io(format!(
            "the backup is {} bytes, and this build writes at most {MAX_SNAPSHOT_BYTES}",
            bytes.len()
        )));
    }

    let temporary = path.with_extension("json.writing");
    let write = || -> std::io::Result<()> {
        use std::io::Write;

        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    };

    if let Err(error) = write() {
        let _ = std::fs::remove_file(&temporary);
        return Err(ShellError::sync_io(format!("the backup could not be written: {error}")));
    }

    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        ShellError::sync_io(format!("the backup could not be replaced: {error}"))
    })
}

/// Reads a backup's text.
///
/// The size is checked from the file's own metadata, before a byte of it is read: a mistyped path
/// that lands on a disk image should be refused, not loaded.
pub fn read(path: &Path) -> Result<String, ShellError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| ShellError::sync_io(format!("the backup could not be read: {error}")))?;
    if metadata.len() > MAX_SNAPSHOT_BYTES {
        return Err(ShellError::sync_snapshot_invalid(
            "that file is far too large to be a backup",
        ));
    }

    std::fs::read_to_string(path).map_err(|error| ShellError::sync_io(format!("the backup could not be read: {error}")))
}

//! The picture behind the window, as a file this installation owns.
//!
//! Chosen once and copied here, rather than read from wherever it was picked out of: a wallpaper
//! somebody later renames, moves or deletes must not take the window's background with it, and the
//! interface is never handed a path it could read anything else through — it is handed a name to show,
//! and this module is the only thing that knows which file that name stands for.
//!
//! The copy keeps the *shape* of its base name rather than the name itself: `background-image.png`,
//! because what the file is called is the person's business and what it is is this module's. A file
//! of a kind this build cannot draw is refused before anything is copied, and the copy that was there
//! before only goes once the new one is in place.

use std::path::{Path, PathBuf};

use sevenshell_error::ShellError;

/// The name every stored picture is given, whatever it was called when it was chosen.
pub const STEM: &str = "background-image";

/// The kinds this build can draw, and therefore the only ones it will copy.
pub const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// The most this build will copy, in bytes.
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;

/// The stored picture, if there is one.
///
/// Found by looking for the name rather than by being told a path: the settings file holds what the
/// picture is *called*, and which file that is, is this module's to know.
pub fn stored_file(directory: &Path) -> Option<PathBuf> {
    EXTENSIONS
        .iter()
        .map(|extension| directory.join(format!("{STEM}.{extension}")))
        .find(|candidate| candidate.is_file())
}

/// The extension of a chosen file, lowercased, or the refusal to copy it.
fn extension_of(source: &Path) -> Result<String, ShellError> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    if EXTENSIONS.contains(&extension.as_str()) {
        Ok(extension)
    } else {
        // The extension as it was written, so the message can quote what was actually refused: an
        // empty string for a file with no extension at all is still worth saying.
        Err(ShellError::store_background_unsupported(
            source.extension().and_then(|value| value.to_str()).unwrap_or(""),
        ))
    }
}

/// Copies a chosen picture in, replacing whatever was there.
///
/// The order is the other way round from what it looks like it should be, and deliberately: the copies
/// that are no longer wanted go *first*. Two copies at once would make "which file is the picture"
/// a question with two answers, since the settings file names a picture rather than a file; deleting
/// first means a failure leaves the old picture where it was and nothing changed, and the caller is
/// told so rather than being left with a picture that is quietly not the one they chose.
pub fn store(directory: &Path, source: &Path) -> Result<(), ShellError> {
    let extension = extension_of(source)?;

    let metadata = std::fs::metadata(source)
        .map_err(|error| ShellError::store_io(format!("cannot read {}: {error}", source.display())))?;
    if metadata.len() > MAX_BYTES {
        return Err(ShellError::store_background_too_large(
            metadata.len() / (1024 * 1024),
            MAX_BYTES / (1024 * 1024),
        ));
    }

    std::fs::create_dir_all(directory)
        .map_err(|error| ShellError::store_io(format!("cannot create {}: {error}", directory.display())))?;

    remove_stale(directory, Some(&extension))?;

    let target = directory.join(format!("{STEM}.{extension}"));
    std::fs::copy(source, &target)
        .map_err(|error| ShellError::store_io(format!("cannot copy the picture into {}: {error}", target.display())))?;

    Ok(())
}

/// Removes the copies of every extension but the one being kept.
///
/// A copy that will not go is an error rather than a warning: it is a second picture, and a second
/// picture is one the window may draw instead of the one somebody chose.
fn remove_stale(directory: &Path, keep: Option<&str>) -> Result<(), ShellError> {
    for extension in EXTENSIONS {
        if Some(extension) == keep {
            continue;
        }
        let candidate = directory.join(format!("{STEM}.{extension}"));
        if candidate.is_file() {
            std::fs::remove_file(&candidate).map_err(|error| {
                ShellError::store_io(format!("the picture being replaced cannot be removed: {error}"))
            })?;
        }
    }
    Ok(())
}

/// Takes the stored picture away. Nothing stored is nothing to do, and not an error: the setting says
/// there is no picture, and this agrees with it.
pub fn clear(directory: &Path) -> Result<(), ShellError> {
    let Some(path) = stored_file(directory) else {
        return Ok(());
    };
    std::fs::remove_file(&path)
        .map_err(|error| ShellError::store_io(format!("cannot remove {}: {error}", path.display())))
}

/// The stored picture's bytes.
///
/// An error when there is none, rather than an empty answer: the interface draws a picture or it
/// draws none, and an empty byte string is a picture that fails to decode somewhere further along.
pub fn read(directory: &Path) -> Result<Vec<u8>, ShellError> {
    let Some(path) = stored_file(directory) else {
        return Err(ShellError::store_io("no background picture is stored"));
    };
    std::fs::read(&path).map_err(|error| ShellError::store_io(format!("cannot read {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory with one file of the given name and size in it.
    fn scratch_with(name: &str, bytes: usize) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let source = directory.path().join(name);
        std::fs::write(&source, vec![7_u8; bytes]).expect("the source file");
        (directory, source)
    }

    #[test]
    fn a_chosen_picture_is_copied_in_under_this_builds_own_name() {
        let (directory, source) = scratch_with("holiday photo.JPG", 32);
        store(directory.path(), &source).expect("the picture is stored");

        let stored = stored_file(directory.path()).expect("something is stored");
        assert_eq!(
            stored.file_name().and_then(|name| name.to_str()),
            Some("background-image.jpg"),
            "the copy keeps the shape of the name and the extension, not the name"
        );
        assert!(source.exists(), "the file somebody chose is theirs and is left alone");
    }

    #[test]
    fn replacing_a_picture_leaves_exactly_one_copy() {
        let (directory, png) = scratch_with("one.png", 16);
        store(directory.path(), &png).expect("the first picture is stored");

        let jpeg = directory.path().join("two.jpeg");
        std::fs::write(&jpeg, vec![1_u8; 16]).expect("the second source file");
        store(directory.path(), &jpeg).expect("the second picture is stored");

        let copies: Vec<PathBuf> = EXTENSIONS
            .iter()
            .map(|extension| directory.path().join(format!("{STEM}.{extension}")))
            .filter(|candidate| candidate.is_file())
            .collect();
        assert_eq!(copies.len(), 1, "a picture that was replaced is a picture that is gone");
        assert_eq!(copies[0].extension().and_then(|value| value.to_str()), Some("jpeg"));
    }

    #[test]
    fn a_kind_this_build_cannot_draw_is_refused_before_anything_is_copied() {
        let (directory, source) = scratch_with("notes.txt", 16);
        let error = store(directory.path(), &source).expect_err("a text file is not a picture");

        assert_eq!(
            error.code().as_str(),
            "STORE-BACKGROUND-UNSUPPORTED",
            "the refusal says what it is about"
        );
        assert!(
            stored_file(directory.path()).is_none(),
            "and nothing was copied in its place"
        );
    }

    #[test]
    fn a_picture_past_the_limit_is_refused() {
        let (directory, source) = scratch_with("huge.png", (MAX_BYTES + 1) as usize);
        let error = store(directory.path(), &source).expect_err("a picture past the limit is refused");

        assert_eq!(error.code().as_str(), "STORE-BACKGROUND-TOO-LARGE");
        assert!(stored_file(directory.path()).is_none());
    }

    #[test]
    fn reading_and_clearing_agree_about_what_is_stored() {
        let (directory, source) = scratch_with("one.png", 16);

        assert!(
            read(directory.path()).is_err(),
            "nothing stored is an error rather than an empty picture"
        );
        clear(directory.path()).expect("clearing nothing is not a failure");

        store(directory.path(), &source).expect("the picture is stored");
        assert_eq!(read(directory.path()).expect("it reads back").len(), 16);

        clear(directory.path()).expect("it is cleared");
        assert!(stored_file(directory.path()).is_none(), "and it is gone");
    }
}

//! Files the operating system handed us, and the tokens that stand in for them.
//!
//! A dropped file and a pasted file are the same problem twice: the operating system knows the
//! paths, and the interface must not. Handing the interface a path would put every file the user
//! can read within reach of anything running in the webview — it could ask the backend to upload
//! `~/.ssh/id_rsa` to a server of its choosing and nothing would look unusual.
//!
//! So the paths stay here, in a registry keyed by an opaque token, and the interface is told the
//! token and the file names. Uploading means "that token, into this directory", and the shell is
//! the only thing that ever learns where the bytes came from.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sevenshell_error::ShellError;
use tokio::sync::Mutex;

/// What the interface is told a drop or a paste contained.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFiles {
    pub token: String,
    /// The files' own names, which is what a person recognises in a message.
    pub names: Vec<String>,
}

/// How many files one drop or paste may carry.
///
/// Not a limit of the protocol: a drop of a hundred files is a mistake or an accident, and the
/// transfers it would start are worse than a refusal that says so.
const MAX_FILES: usize = 64;

/// Paths the operating system handed us, waiting to be named by whoever receives them.
#[derive(Default)]
pub struct PendingDrops {
    entries: Mutex<HashMap<String, Vec<PathBuf>>>,
    next: AtomicU64,
}

impl PendingDrops {
    /// Remembers a set of local paths and returns the token that stands for them.
    ///
    /// Files only, and only ones that exist: a dropped shortcut, a folder or a path that has since
    /// been deleted is not something an upload can do anything with.
    pub fn remember(&self, paths: Vec<PathBuf>) -> Result<PendingFiles, ShellError> {
        let files: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_file()).collect();

        if files.is_empty() {
            return Err(ShellError::app_unsupported("dropping something that is not a file"));
        }
        if files.len() > MAX_FILES {
            return Err(ShellError::app_unsupported("dropping that many files at once")
                .with_detail(format!("the limit is {MAX_FILES} files")));
        }

        let names = files
            .iter()
            .map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();

        let token = format!("d{}", self.next.fetch_add(1, Ordering::Relaxed));

        // Blocking on a mutex in an event handler is fine: the critical section is one insert, and
        // there is no await that could hold it across a task switch.
        if let Ok(mut entries) = self.entries.try_lock() {
            entries.insert(token.clone(), files);
        } else {
            log::warn!("a drop arrived while the pending list was busy and was refused");
            return Err(ShellError::app_internal("the pending files were busy; drop them again"));
        }

        Ok(PendingFiles { token, names })
    }

    /// Takes the paths a token stands for. A token is good for one upload.
    pub async fn take(&self, token: &str) -> Result<Vec<PathBuf>, ShellError> {
        self.entries
            .lock()
            .await
            .remove(token)
            .ok_or_else(|| ShellError::app_unsupported("those files are no longer waiting; drop or paste them again"))
    }
}

/// The file list on the clipboard, if the platform can read one.
///
/// Windows keeps it in a documented clipboard format; the other platforms have several, none of
/// them reachable from here, and guessing would be worse than saying so.
#[cfg(windows)]
pub fn clipboard_files() -> Result<Vec<PathBuf>, ShellError> {
    use clipboard_win::{formats::FileList, get_clipboard};

    let entries: Vec<String> = get_clipboard(FileList).map_err(|error| {
        ShellError::app_unsupported("reading files from the clipboard").with_detail(error.to_string())
    })?;

    Ok(entries.into_iter().map(PathBuf::from).collect())
}

#[cfg(not(windows))]
pub fn clipboard_files() -> Result<Vec<PathBuf>, ShellError> {
    Err(ShellError::app_unsupported(
        "pasting files from the clipboard on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn a_token_stands_for_the_files_that_were_dropped() {
        let directory = tempdir().expect("temp dir");
        let file = directory.path().join("notes.txt");
        std::fs::write(&file, "hello").expect("write");

        let pending = PendingDrops::default();
        let dropped = pending.remember(vec![file.clone()]).expect("remember");

        assert_eq!(dropped.names, vec!["notes.txt".to_owned()]);
        assert_eq!(pending.take(&dropped.token).await.expect("take"), vec![file]);
    }

    #[tokio::test]
    async fn a_token_is_good_for_one_upload() {
        let directory = tempdir().expect("temp dir");
        let file = directory.path().join("notes.txt");
        std::fs::write(&file, "hello").expect("write");

        let pending = PendingDrops::default();
        let dropped = pending.remember(vec![file]).expect("remember");
        pending.take(&dropped.token).await.expect("the first take works");

        let error = pending.take(&dropped.token).await.expect_err("the second does not");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
    }

    #[tokio::test]
    async fn dropping_a_directory_is_refused_rather_than_uploaded_as_a_file() {
        let directory = tempdir().expect("temp dir");

        let error = PendingDrops::default()
            .remember(vec![directory.path().to_path_buf()])
            .expect_err("a directory is not a file");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
    }

    #[tokio::test]
    async fn a_drop_of_too_many_files_is_refused_with_the_reason() {
        let directory = tempdir().expect("temp dir");
        let paths: Vec<PathBuf> = (0..(MAX_FILES + 1))
            .map(|index| {
                let path = directory.path().join(format!("file-{index}.txt"));
                std::fs::write(&path, "x").expect("write");
                path
            })
            .collect();

        let error = PendingDrops::default().remember(paths).expect_err("too many");

        assert!(error.detail().unwrap_or_default().contains(&MAX_FILES.to_string()));
    }
}

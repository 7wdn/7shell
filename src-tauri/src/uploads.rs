//! Uploads that stopped to ask about the names they would replace.
//!
//! The same reason as the drop registry next door: the local paths stay here. What the interface is
//! told is a token and the names already on the far side, it answers with one word, and the files
//! are sent from a place the webview never saw.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sevenshell_error::ShellError;
use tokio::sync::Mutex;

/// What somebody chose and has not sent yet, held while the question is open.
#[derive(Debug, Clone)]
pub struct PendingUpload {
    pub handle: String,
    pub directory: String,
    /// The local files, in the order the names were checked.
    pub sources: Vec<PathBuf>,
    /// Whether they travel as one archive — which is also why "send only what is missing" is not a
    /// real answer for them: an archive is unpacked whole.
    pub packed: bool,
    /// The names already on the far side when the question was asked.
    pub taken: Vec<String>,
}

impl PendingUpload {
    /// The sources that are not already there, for the answer that skips the rest.
    pub fn missing(&self) -> Vec<PathBuf> {
        self.sources
            .iter()
            .filter(|source| match source.file_name() {
                Some(name) => !self.taken.contains(&name.to_string_lossy().into_owned()),
                // A path with no file name has nothing to compare with, and the transfer that
                // refuses it will say so better than a filter can.
                None => true,
            })
            .cloned()
            .collect()
    }
}

/// Uploads waiting for an answer, keyed by the token the interface was given.
#[derive(Default)]
pub struct PendingUploads {
    entries: Mutex<HashMap<String, PendingUpload>>,
    next: AtomicU64,
}

impl PendingUploads {
    /// Remembers an upload and returns the token that stands for it.
    pub async fn remember(&self, upload: PendingUpload) -> String {
        let token = format!("upload-{}", self.next.fetch_add(1, Ordering::Relaxed));
        self.entries.lock().await.insert(token.clone(), upload);
        token
    }

    /// Takes the upload a token stands for. A token is good for exactly one answer.
    pub async fn take(&self, token: &str) -> Result<PendingUpload, ShellError> {
        self.entries
            .lock()
            .await
            .remove(token)
            .ok_or_else(|| ShellError::app_internal("that upload is no longer waiting; send the files again"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(sources: &[&str], taken: &[&str]) -> PendingUpload {
        PendingUpload {
            handle: "t1".to_owned(),
            directory: "/srv".to_owned(),
            sources: sources.iter().map(PathBuf::from).collect(),
            packed: false,
            taken: taken.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    #[test]
    fn skipping_leaves_out_exactly_the_files_that_are_already_there() {
        let upload = pending(&["/tmp/a.txt", "/tmp/b.txt", "/tmp/c.txt"], &["b.txt"]);

        assert_eq!(
            upload.missing(),
            vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/c.txt")]
        );
    }

    #[test]
    fn skipping_everything_leaves_nothing_to_send() {
        let upload = pending(&["/tmp/a.txt"], &["a.txt"]);

        assert!(upload.missing().is_empty());
    }

    #[tokio::test]
    async fn a_token_is_good_for_one_answer() {
        let registry = PendingUploads::default();
        let token = registry.remember(pending(&["/tmp/a.txt"], &["a.txt"])).await;

        assert!(registry.take(&token).await.is_ok());
        let second = registry.take(&token).await;
        assert!(second.is_err(), "the same answer cannot be given twice");
    }
}

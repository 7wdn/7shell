//! Where downloads land, when somebody has said where that is.
//!
//! A file of its own rather than a field of the settings, and that is not tidiness: the whole
//! settings struct is handed to the interface (`settings_get`), and a local path in that payload
//! would be a path the interface could also *write back* — which is the one thing invariant 6 keeps
//! out of its hands. What crosses to the interface is the folder's own name; the path lives here, is
//! read by the side that writes the bytes, and is asked for by the same rule the transfer list uses:
//! the interface names a row, this side answers with where.
//!
//! Same three rules as the settings file: a missing file is not an error, a file from a newer version
//! is refused, and writes are atomic.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;

/// The file name inside the configuration directory.
pub const DOWNLOADS_FILE_NAME: &str = "downloads.json";

/// Bumped only when the stored shape changes in a way older builds cannot read.
pub const DOWNLOADS_SCHEMA_VERSION: u32 = 1;

/// How many folders are remembered.
///
/// Five, because the list is a way *back* to one of the last few places somebody put things: a list
/// long enough to need reading is a list that has stopped being recognisable at a glance.
pub const HISTORY_LIMIT: usize = 5;

/// Where downloads go, as this side keeps it.
///
/// The history always holds the current folder first, so the dropdown somebody opens is one list
/// rather than two states to reconcile — and `current` is never a folder the list does not name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadFolder {
    current: Option<PathBuf>,
    /// The folders that have been chosen, newest first.
    history: Vec<PathBuf>,
}

impl DownloadFolder {
    /// Where downloads go, or `None` for "ask every time" — which is what an installation nobody has
    /// configured does, and what somebody who wants the save dialog back gets.
    pub fn current(&self) -> Option<&Path> {
        self.current.as_deref()
    }

    /// The folders that have been chosen, newest first.
    pub fn history(&self) -> &[PathBuf] {
        &self.history
    }

    /// Makes `path` where downloads go, and remembers it.
    ///
    /// Choosing a folder that is already in the list moves it to the front rather than adding a
    /// second copy: the same folder twice is not two ways back to two places.
    pub fn choose(&mut self, path: PathBuf) {
        self.history.retain(|entry| entry != &path);
        self.history.insert(0, path.clone());
        self.history.truncate(HISTORY_LIMIT);
        self.current = Some(path);
    }

    /// Makes the folder at `index` of the history where downloads go, moving it to the front.
    ///
    /// Answers whether there was one: an index from a list that has changed under the pointer is
    /// stale rather than wrong, and the caller shows the list it has rather than an error.
    pub fn select(&mut self, index: usize) -> bool {
        let Some(path) = self.history.get(index).cloned() else {
            return false;
        };

        self.choose(path);
        true
    }

    /// Stops using a fixed folder, so downloads ask again. What was used stays in the list.
    pub fn ask(&mut self) {
        self.current = None;
    }
}

/// What is actually on disk.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredDownloadFolder {
    schema_version: u32,
    #[serde(default)]
    current: Option<String>,
    #[serde(default)]
    history: Vec<String>,
}

impl StoredDownloadFolder {
    /// Reads the file into the shape the rest of the application uses.
    ///
    /// A path that is not absolute is dropped rather than kept: it would resolve against wherever
    /// the process happens to be standing, which is not a place anybody chose. The same goes for a
    /// current folder that is not in the history — the list is what the interface offers, so a folder
    /// the list does not name is one nobody could get back to.
    fn into_folder(self) -> DownloadFolder {
        let history: Vec<PathBuf> = self
            .history
            .into_iter()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .collect();

        let current = self
            .current
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && history.contains(path));

        let mut folder = DownloadFolder { current, history };
        folder.history.truncate(HISTORY_LIMIT);
        folder
    }
}

/// Where the downloads file lives, whether or not it exists yet.
pub fn downloads_file() -> Option<PathBuf> {
    crate::paths::config_dir().map(|directory| directory.join(DOWNLOADS_FILE_NAME))
}

/// The downloads file of the running application, from the platform configuration directory.
pub fn load() -> Result<DownloadFolder, ShellError> {
    match downloads_file() {
        Some(path) => load_from(&path),
        None => Ok(DownloadFolder::default()),
    }
}

/// Writes the downloads file of the running application, creating the directory if needed.
pub fn save(folder: &DownloadFolder) -> Result<(), ShellError> {
    let path = downloads_file()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this system"))?;

    save_to(&path, folder)
}

pub fn load_from(path: &Path) -> Result<DownloadFolder, ShellError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(DownloadFolder::default()),
        Err(error) => return Err(ShellError::store_io(format!("cannot read {}: {error}", path.display()))),
    };

    let stored: StoredDownloadFolder = serde_json::from_str(&raw)
        .map_err(|error| ShellError::store_io(format!("{} is not valid downloads JSON: {error}", path.display())))?;

    if stored.schema_version > DOWNLOADS_SCHEMA_VERSION {
        return Err(
            ShellError::store_schema_newer(stored.schema_version, DOWNLOADS_SCHEMA_VERSION)
                .with_detail(format!("{} was written by a newer version of 7Shell", path.display())),
        );
    }

    Ok(stored.into_folder())
}

pub fn save_to(path: &Path, folder: &DownloadFolder) -> Result<(), ShellError> {
    let stored = StoredDownloadFolder {
        schema_version: DOWNLOADS_SCHEMA_VERSION,
        current: folder.current.as_ref().map(|path| path.to_string_lossy().into_owned()),
        history: folder
            .history
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    };

    let json = serde_json::to_string_pretty(&stored)
        .map_err(|error| ShellError::app_internal(format!("the download folder could not be serialized: {error}")))?;

    crate::settings::write_atomically(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::write_atomically;

    /// A folder that is absolute on whichever machine the tests run on.
    ///
    /// The rule under test is about absolute paths, and a literal like `/one` is absolute on one
    /// platform and merely rooted on another — a test that cannot make up its mind about that is
    /// testing the host it happens to run on.
    fn at(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    /// Oldest first, so the list reads the way somebody chose it.
    fn folder(names: &[&str]) -> DownloadFolder {
        let mut folder = DownloadFolder::default();
        for name in names {
            folder.choose(at(name));
        }
        folder
    }

    /// A path as it is written in the file, escaped the way JSON needs it.
    fn as_json(path: &Path) -> String {
        serde_json::to_string(&path.to_string_lossy()).expect("a path is JSON")
    }

    /// A file with exactly these contents, in a directory the caller keeps alive.
    fn written(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join(DOWNLOADS_FILE_NAME);
        write_atomically(&path, contents.as_bytes()).expect("write");
        (directory, path)
    }

    /// A file naming one current folder and a list of them.
    fn file_with(current: Option<&str>, history: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let list = history
            .iter()
            .map(|name| as_json(&at(name)))
            .collect::<Vec<_>>()
            .join(",");
        let current = current
            .map(|name| as_json(&at(name)))
            .unwrap_or_else(|| "null".to_owned());

        written(&format!(
            "{{\"schemaVersion\":1,\"current\":{current},\"history\":[{list}]}}"
        ))
    }

    #[test]
    fn nothing_chosen_means_every_download_asks() {
        let folder = DownloadFolder::default();

        assert_eq!(folder.current(), None);
        assert!(folder.history().is_empty());
    }

    #[test]
    fn chosen_folders_are_remembered_newest_first() {
        let folder = folder(&["one", "two"]);

        assert_eq!(folder.current(), Some(at("two").as_path()));
        assert_eq!(
            folder.history(),
            &[at("two"), at("one")],
            "the newest choice is first, and the current folder is the first one"
        );
    }

    #[test]
    fn choosing_a_folder_twice_moves_it_to_the_front_rather_than_adding_it_again() {
        let mut folder = folder(&["one", "two"]);
        folder.choose(at("one"));

        assert_eq!(folder.history(), &[at("one"), at("two")]);
    }

    #[test]
    fn the_list_stops_at_the_last_five() {
        let mut folder = DownloadFolder::default();
        for index in 0..(HISTORY_LIMIT + 3) {
            folder.choose(at(&format!("folder-{index}")));
        }

        assert_eq!(folder.history().len(), HISTORY_LIMIT);
        assert_eq!(
            folder.history().first(),
            Some(&at(&format!("folder-{}", HISTORY_LIMIT + 2)))
        );
    }

    #[test]
    fn picking_one_from_the_list_makes_it_the_current_folder() {
        let mut folder = folder(&["one", "two"]);

        assert!(folder.select(1));
        assert_eq!(folder.current(), Some(at("one").as_path()));
        assert_eq!(folder.history(), &[at("one"), at("two")]);

        // A list that changed under the pointer: an answer of "no" is the honest one, and the caller
        // draws the list it has.
        assert!(!folder.select(9));
        assert_eq!(folder.current(), Some(at("one").as_path()));
    }

    #[test]
    fn asking_again_keeps_the_list_of_what_was_used() {
        let mut folder = folder(&["one", "two"]);
        folder.ask();

        assert_eq!(folder.current(), None);
        assert_eq!(
            folder.history(),
            &[at("two"), at("one")],
            "the way back to a folder somebody used before is not thrown away with the current one"
        );
    }

    #[test]
    fn a_file_nobody_has_written_reads_as_asking() {
        let path = Path::new("/definitely/not/here/downloads.json");

        assert_eq!(
            load_from(path).expect("a missing file is not an error"),
            DownloadFolder::default()
        );
    }

    #[test]
    fn what_was_written_is_what_comes_back() {
        let (_directory, path) = file_with(Some("chosen"), &["chosen", "older"]);

        let folder = load_from(&path).expect("read it back");

        assert_eq!(folder.current(), Some(at("chosen").as_path()));
        assert_eq!(folder.history(), &[at("chosen"), at("older")]);
    }

    #[test]
    fn a_relative_path_is_dropped_rather_than_resolved_against_the_process() {
        // "downloads" is relative on every platform, which is the point: it would resolve against
        // wherever the process happens to be standing.
        let json = format!(
            "{{\"schemaVersion\":1,\"current\":\"downloads\",\"history\":[\"downloads\",{}]}}",
            as_json(&at("things"))
        );
        let (_directory, path) = written(&json);

        let folder = load_from(&path).expect("read it back");

        assert_eq!(
            folder.current(),
            None,
            "a folder that would resolve against the process is not one anybody chose"
        );
        assert_eq!(folder.history(), &[at("things")]);
    }

    #[test]
    fn a_current_folder_the_list_does_not_name_is_dropped() {
        let json = format!(
            "{{\"schemaVersion\":1,\"current\":{},\"history\":[{}]}}",
            as_json(&at("elsewhere")),
            as_json(&at("things"))
        );
        let (_directory, path) = written(&json);

        assert_eq!(
            load_from(&path).expect("read it back").current(),
            None,
            "a folder the interface cannot offer is one nobody could get back to"
        );
    }

    #[test]
    fn a_file_written_by_a_newer_version_is_refused() {
        let (_directory, path) = written(r#"{"schemaVersion":99,"current":null,"history":[]}"#);

        let error = load_from(&path).expect_err("a newer file is refused");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreSchemaNewer);
    }

    #[test]
    fn a_saved_folder_survives_the_round_trip() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join(DOWNLOADS_FILE_NAME);
        let folder = folder(&["one", "two"]);

        save_to(&path, &folder).expect("save it");
        let read = load_from(&path).expect("read it back");

        assert_eq!(read, folder, "the same folder, the same list, in the same order");
    }
}

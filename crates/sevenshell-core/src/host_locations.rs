//! Places on this machine an agent may name, held as opaque tokens.
//!
//! A token is minted after a native dialog the host opens on the person's behalf: the person picks
//! a file or a folder, the path stays in this registry, and everything the interface or an agent
//! sees is the token and a display name. The registry lives as long as the application: a token is
//! a permission granted in this session, not a file somebody handed over for ever.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sevenshell_error::ShellError;
use sevenshell_types::{HostLocationKind, HostLocationView};

use crate::Core;

pub(crate) struct HostLocationEntry {
    path: PathBuf,
    name: String,
    kind: HostLocationKind,
}

impl Core {
    /// Records a location the host's dialog just returned, and answers the token that stands for it.
    pub fn host_location_add(
        &self,
        path: impl AsRef<Path>,
        kind: HostLocationKind,
    ) -> Result<HostLocationView, ShellError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(ShellError::host_location_unknown("the chosen location does not exist"));
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ShellError::host_location_unknown("the chosen location has no name to show"))?;
        let token = uuid::Uuid::new_v4().to_string();
        self.host_locations_lock().insert(
            token.clone(),
            HostLocationEntry {
                path: path.to_path_buf(),
                name: name.clone(),
                kind,
            },
        );
        Ok(HostLocationView { token, name, kind })
    }

    /// Every location this session knows, by display name.
    pub fn host_locations(&self) -> Vec<HostLocationView> {
        let mut views: Vec<HostLocationView> = self
            .host_locations_lock()
            .iter()
            .map(|(token, entry)| HostLocationView {
                token: token.clone(),
                name: entry.name.clone(),
                kind: entry.kind,
            })
            .collect();
        views.sort_by_key(|view| view.name.to_lowercase());
        views
    }

    /// Forgets one token. The path is left alone: nothing here created it.
    pub fn host_location_forget(&self, token: &str) -> bool {
        self.host_locations_lock().remove(token).is_some()
    }

    /// The path behind a token, when it is the kind the operation needs.
    pub(crate) fn host_location_path(&self, token: &str, expected: HostLocationKind) -> Result<PathBuf, ShellError> {
        let locations = self.host_locations_lock();
        let entry = locations.get(token).ok_or_else(|| {
            ShellError::host_location_unknown("this host location is not known; add it in the app first")
        })?;
        if entry.kind != expected {
            return Err(ShellError::host_location_unknown(match expected {
                HostLocationKind::File => "this operation needs a file token, and that token names a folder",
                HostLocationKind::Folder => "this operation needs a folder token, and that token names a file",
            }));
        }
        Ok(entry.path.clone())
    }

    fn host_locations_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, HostLocationEntry>> {
        self.host_locations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

//! Places on this machine an agent may name, held as opaque tokens.
//!
//! A token is minted after a native dialog the host opens on the person's behalf: the person picks
//! a file or a folder, the path is written into the vault, and everything the interface or an
//! agent sees is the token and a display name. The vault is where the path lives — encrypted at
//! rest, read only by the core, and never printed, audited or sent anywhere.

use std::path::{Path, PathBuf};

use sevenshell_error::ShellError;
use sevenshell_types::{HostLocationKind, HostLocationView};

use crate::Core;

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
        self.vault
            .save_host_location(&token, &name, kind, &path.to_string_lossy())?;
        Ok(HostLocationView { token, name, kind })
    }

    /// Every location this vault holds, by display name. A locked vault is an error, not an empty list.
    pub fn host_locations(&self) -> Result<Vec<HostLocationView>, ShellError> {
        Ok(self
            .vault
            .host_locations()?
            .into_iter()
            .map(|record| HostLocationView {
                token: record.token,
                name: record.name,
                kind: record.kind,
            })
            .collect())
    }

    /// Forgets one token. The path is left alone: nothing here created it.
    pub fn host_location_forget(&self, token: &str) -> Result<bool, ShellError> {
        self.vault.remove_host_location(token)
    }

    /// The path behind a token, when it is the kind the operation needs.
    pub(crate) fn host_location_path(&self, token: &str, expected: HostLocationKind) -> Result<PathBuf, ShellError> {
        let record = self.vault.host_location(token)?.ok_or_else(|| {
            ShellError::host_location_unknown("this host location is not known; add it in the app first")
        })?;
        if record.kind != expected {
            return Err(ShellError::host_location_unknown(match expected {
                HostLocationKind::File => "this operation needs a file token, and that token names a folder",
                HostLocationKind::Folder => "this operation needs a folder token, and that token names a file",
            }));
        }
        Ok(PathBuf::from(record.path))
    }
}

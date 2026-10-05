//! The keys this device holds.
//!
//! One method, and it is here rather than beside another feature because it belongs to none of
//! them: the sessions that use a key, the editor that picks one, and the security page that lists
//! them all ask the same question, and none of them owns the answer.

use sevenshell_error::ShellError;
use sevenshell_types::KeyInfo;

use crate::Core;

impl Core {
    /// The imported keys, as the interface may know them: a name, an algorithm, a fingerprint, and
    /// whether a passphrase is held. Never the material.
    pub fn keys(&self) -> Result<Vec<KeyInfo>, ShellError> {
        self.vault.keys()
    }
}

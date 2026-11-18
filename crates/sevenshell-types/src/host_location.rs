//! A place on this machine an agent may name, without the interface ever holding its path.
//!
//! A token is minted by a native dialog in the host: the person picks a file or a folder, the host
//! keeps the path, and everything the interface or an agent sees is this view — a token, a display
//! name and whether it is a file or a folder. The tools that copy, clone or capture take the token
//! and the host resolves it; nothing on the wire, in a prompt or in an audit line is a path.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostLocationKind {
    File,
    Folder,
}

/// One host location as anything outside the host sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostLocationView {
    /// The opaque name tools pass back. Never a path.
    pub token: String,
    /// What the person sees, which is the file or folder's own name.
    pub name: String,
    pub kind: HostLocationKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_carries_a_token_and_a_name_and_no_path() {
        let view = HostLocationView {
            token: "t-1".to_owned(),
            name: "exports".to_owned(),
            kind: HostLocationKind::Folder,
        };
        let json = serde_json::to_value(&view).expect("a view serializes");
        assert_eq!(
            json,
            serde_json::json!({ "token": "t-1", "name": "exports", "kind": "folder" })
        );
    }
}

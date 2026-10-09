//! A secret on its way into a file, and back out of one.
//!
//! The vault holds two kinds of secret at rest: the login stored for a session, and the material of
//! an imported key. Both leave it the same way — inside a payload that a passphrase seals — and
//! both come back the same way, so the shape they take in between lives here rather than in
//! whichever feature happens to be writing a file this week. An export and a sync snapshot carry
//! the *same* types, which is the point: the discipline below is written once and cannot drift
//! between two copies of it.
//!
//! Three rules, and they are all one rule about a value that has left the vault still being a
//! secret:
//!
//! * **It is wiped when it is dropped.** Nothing here is a `String` handed back to the allocator to
//!   be picked up by the next allocation.
//! * **It can never be printed.** Neither type derives `Debug`; the manual implementations say what
//!   an entry is *about* and nothing else.
//! * **It cannot be taken out by value without giving up the struct.** The one way to move a value
//!   out consumes `self`, so there is never a second copy and never a struct left behind holding a
//!   wiped-but-still-allocated buffer.

use serde::{Deserialize, Serialize};
use sevenshell_types::KeyInfo;
use zeroize::Zeroize;

/// Which secret an entry holds.
///
/// Written out rather than derived from the session on the way in: the file should say what it
/// carries, so a reader — and a person looking at it — does not have to know the authentication
/// rules to see that a password is in there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    Password,
    KeyPassphrase,
}

/// One stored secret in a file.
///
/// No `Debug`: a derived one would print the value into any log line an unwary caller wrote. What
/// this is *about* is safe to say, and that is all the manual implementation says.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretEntry {
    /// The session the secret belongs to, by the id it had in the file it came from.
    pub session_id: String,
    pub kind: SecretKind,
    pub value: String,
}

impl SecretEntry {
    /// Moves the value out so it can be wrapped for the vault, leaving nothing behind to wipe.
    ///
    /// Consuming `self` keeps one owner for the secret: cloning would leave a second copy in memory
    /// for the allocator to hand to whoever asks next.
    pub fn into_value(mut self) -> String {
        std::mem::take(&mut self.value)
    }
}

impl Drop for SecretEntry {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

impl std::fmt::Debug for SecretEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretEntry({:?}, <redacted>)", self.kind)
    }
}

/// An imported key in a file: its handle, what the interface knows about it, and the material.
///
/// The same three rules as [`SecretEntry`], for the same reason. `info` is safe to print — it is
/// what the interface already shows — so it is the two values below it that the manual `Debug`
/// leaves out.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntry {
    pub key_ref: String,
    pub info: KeyInfo,
    pub contents: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

impl KeyEntry {
    /// Takes the material apart so it can be handed back to the vault, which wraps and wipes it.
    ///
    /// Consuming `self` keeps one owner for the secret: cloning would leave a second copy in memory
    /// for the allocator to hand to whoever asks next.
    pub fn into_parts(mut self) -> (String, Option<String>) {
        (std::mem::take(&mut self.contents), self.passphrase.take())
    }
}

impl Drop for KeyEntry {
    fn drop(&mut self) {
        self.contents.zeroize();
        if let Some(passphrase) = &mut self.passphrase {
            passphrase.zeroize();
        }
    }
}

impl std::fmt::Debug for KeyEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEntry")
            .field("key_ref", &self.key_ref)
            .field("display_name", &self.info.display_name)
            .field("contents", &"<redacted>")
            .field("passphrase", &self.passphrase.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

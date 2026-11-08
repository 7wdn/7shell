//! What a remote desktop session tells the interface.
//!
//! The same shape as the SSH trust decision: an unknown certificate is a question, a changed one
//! is a stop, and an opened session carries a handle — never the password or the raw connection.

use serde::{Deserialize, Serialize};

use crate::KnownHostEntry;

/// A certificate an RDP server presented, before anybody has decided to trust it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentedCertificate {
    pub host: String,
    pub port: u16,
    /// `SHA256:` followed by the base64 digest — the spelling SSH fingerprints already use.
    pub fingerprint: String,
    /// The certificate itself, base64 DER, so a saved trust record is self-contained.
    pub certificate: String,
}

/// What the interface is told when opening a remote desktop needs a decision, or has succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DesktopOutcome {
    /// This endpoint has no trusted certificate yet. Nothing has been authenticated.
    CertificateUnknown { certificate: PresentedCertificate },
    /// The presented certificate differs from the trusted one. A hard stop, not a prompt.
    CertificateMismatch {
        certificate: PresentedCertificate,
        trusted: KnownHostEntry,
    },
    /// The session is open and producing frames.
    Opened {
        handle: String,
        session_id: String,
        width: u16,
        height: u16,
    },
}

/// A desktop that is open right now, so the interface can rebuild its tabs after a reload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDesktop {
    pub handle: String,
    pub session_id: String,
    pub width: u16,
    pub height: u16,
}

/// Input from the interface.
///
/// Separate from the transport's own input types on purpose: this is the shape the page sends,
/// and it carries no RDP vocabulary the interface would have to keep up with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DesktopInput {
    PointerMove {
        x: u16,
        y: u16,
    },
    PointerButton {
        button: DesktopMouseButton,
        pressed: bool,
    },
    /// In RDP's units: 120 per notch, negative for up/left.
    Wheel {
        vertical: bool,
        units: i16,
    },
    /// A PC scancode; `0xE0xx` is the extended form.
    Key {
        scancode: u16,
        pressed: bool,
    },
    /// A composed string from an input method. Sent as Unicode key events, so a Chinese input
    /// method works even though no scancode describes the character it produced.
    Text {
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopMouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

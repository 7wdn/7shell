//! Talking to a server: check the key it presents, authenticate, carry a terminal.
//!
//! The order of operations here is the security property, not an implementation detail. The
//! key from the key exchange is compared with the key the caller has already accepted
//! *before* a credential is offered, and the connection that authenticates is the same
//! connection that verified, so there is no window in which an unverified server could
//! collect a password. [`probe_host_key`] exists so a caller can ask a person about a key it
//! has never seen; the answer travels back as a value, and the connection that follows
//! re-checks the key it actually receives against it rather than trusting the record.
//!
//! Nothing here knows about the vault, the session registry or the interface: plain values
//! in, plain values out. Holding the known-host record and deciding whether a new key is
//! trusted belong to the caller.
//!
//! Three things this build does not carry, and says so through an envelope instead of
//! failing in a way nobody can read: agent authentication (`ssh-agent`, Pageant), FIDO
//! security keys, and host certificates.

mod credential;
mod hostkey;
mod session;

mod exec;
mod route;

use std::time::Duration;

use sevenshell_types::PresentedHostKey;

pub use credential::{Credential, KeyDescription, describe_key};
pub use exec::{ExecOutcome, MAX_OUTPUT_BYTES};
pub use hostkey::{fingerprint, probe_host_key, probe_host_key_via};
pub use route::{BoxedStream, LocalForward, Route, TunnelHandle, TunnelStream};
pub use session::SshSession;

/// Where to connect and who to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The name as the person typed it. Resolving it is the network's business, and a name
    /// that does not resolve is one of the ways a connection turns out to be unreachable.
    pub host: String,
    pub port: u16,
    /// The account to authenticate as. It travels with every attempt, so it is also what the
    /// server refuses when it refuses everything.
    pub user: String,
}

/// A terminal's size, in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub columns: u32,
    pub rows: u32,
}

impl Default for TerminalSize {
    /// 80x24 is the size a terminal reports when nothing has told it better, and the size
    /// `stty` and `$COLUMNS` fall back to on the far side.
    fn default() -> Self {
        Self { columns: 80, rows: 24 }
    }
}

/// Everything an open needs.
#[derive(Debug)]
pub struct OpenRequest {
    pub target: Target,
    /// Moved in and dropped by the connection; nothing keeps a copy.
    pub credential: Credential,
    /// The terminal type asked of the server, for example `xterm-256color`. It decides which
    /// terminfo entry the remote applications look up.
    pub terminal_type: String,
    /// Seconds between keep-alives. Zero turns them off.
    pub keepalive_seconds: u16,
    /// The budget for the whole open — connecting, the key exchange, authentication and the
    /// channel — not for each step of it. Zero expires at once; there is no way to ask for
    /// no timeout, because a session that hangs forever is never what a caller wants.
    pub timeout: Duration,
    /// The host key the caller has already accepted. The connection verifies the key it
    /// actually receives against this one and refuses to authenticate if they differ.
    pub expected_host_key: PresentedHostKey,
    /// The size the pty is created with, before anyone resizes it.
    pub size: TerminalSize,
    /// How the connection reaches its target: directly, or through a session that is already open.
    ///
    /// A decision rather than a default, because "straight to the server" and "through a jump
    /// host" are different promises to whoever is reading the logs at the other end.
    pub route: Route,
    /// What the connection is for.
    pub purpose: Purpose,
}

/// What a connection is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A terminal: a session channel is opened, a pty is asked for, and its output is streamed.
    Terminal,
    /// A carrier for something else — the next hop of a route, or the far side of a forward.
    ///
    /// No session channel and no pty: asking a jump host for a shell would leave a process running
    /// on it that nobody asked for and nobody would think to look for.
    Carrier,
}

/// What comes out of a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelEvent {
    /// Bytes the terminal produced, in the order the server sent them.
    Data(Vec<u8>),
    /// The last event on the channel. `reason` explains an end nobody asked for; a clean end
    /// carries `None`, and no session ever ends in silence.
    Closed { reason: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presented() -> PresentedHostKey {
        PresentedHostKey {
            host: "host.example".to_owned(),
            port: 22,
            key_type: "ssh-ed25519".to_owned(),
            public_key: "AAAA".to_owned(),
            fingerprint: "SHA256:AAAA".to_owned(),
        }
    }

    #[test]
    fn a_terminal_defaults_to_the_size_nothing_has_told_it_better() {
        let size = TerminalSize::default();
        assert_eq!((size.columns, size.rows), (80, 24));
    }

    #[test]
    fn an_open_request_can_be_built_without_naming_every_field() {
        let request = OpenRequest {
            target: Target {
                host: "host.example".to_owned(),
                port: 22,
                user: "person".to_owned(),
            },
            credential: Credential::Password("hunter2".to_owned()),
            terminal_type: "xterm-256color".to_owned(),
            keepalive_seconds: 30,
            timeout: Duration::from_secs(15),
            expected_host_key: presented(),
            size: TerminalSize::default(),
            route: Route::Direct,
            purpose: Purpose::Terminal,
        };
        // The credential must not be printable, so this is the one thing about it a test can
        // safely assert without reaching into the value.
        assert!(format!("{request:?}").contains("[redacted]"));
    }
}

//! The server's key as it crosses the wire: read it, compare it, print its fingerprint.
//!
//! A key is compared as key material — the algorithm and the bytes of the blob — not as a
//! fingerprint string. The fingerprint is derived from the key, so two keys cannot be made to
//! look equal by storing the right text next to the wrong key.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client;
use russh::keys::{HashAlg, PublicKey, PublicKeyBase64, PublicKeyOrCertificate};
use sevenshell_error::ShellError;
use sevenshell_types::PresentedHostKey;

use crate::session::{Failure, client_config, failure_envelope, whole_seconds};

/// The key a caller has accepted, in the form the server's key is compared against.
///
/// Only the two fields that decide the comparison are kept: the host and port name the record,
/// and the fingerprint is a view of the key rather than a second opinion about it.
#[derive(Debug, Clone)]
pub(crate) struct ExpectedKey {
    key_type: String,
    public_key: String,
}

impl ExpectedKey {
    pub(crate) fn new(key_type: &str, public_key: &str) -> Self {
        Self {
            key_type: key_type.to_owned(),
            public_key: public_key.to_owned(),
        }
    }

    /// Whether the key the server presented is the key the caller accepted.
    pub(crate) fn accepts(&self, key: &PublicKey) -> bool {
        self.key_type == key.algorithm().as_str() && self.public_key == key.public_key_base64()
    }
}

/// The key inside the server's answer, or nothing when what arrived is a certificate: the
/// caller accepted a key, and only a key can be held against it.
pub(crate) fn plain_key(server_key: &PublicKeyOrCertificate) -> Option<&PublicKey> {
    match server_key {
        PublicKeyOrCertificate::PublicKey { key, .. } => Some(key),
        PublicKeyOrCertificate::Certificate(_) => None,
    }
}

/// A key as it crossed the wire, ready to be stored in a known-hosts record or compared with
/// one.
pub(crate) fn presented(host: &str, port: u16, key: &PublicKey) -> PresentedHostKey {
    PresentedHostKey {
        host: host.to_owned(),
        port,
        key_type: key.algorithm().as_str().to_owned(),
        public_key: key.public_key_base64(),
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    }
}

/// Connects far enough to read the server's host key, then disconnects. Sends no credential.
///
/// This is what a caller shows a person before trusting a host for the first time. The key it
/// returns is a value, not a decision: the connection that follows compares the key it
/// receives with the accepted one again, so trusting this answer cannot be replayed onto a
/// different server.
pub async fn probe_host_key(host: &str, port: u16, timeout: Duration) -> Result<PresentedHostKey, ShellError> {
    let seen: Arc<Mutex<Option<PresentedHostKey>>> = Arc::new(Mutex::new(None));
    let handler = Probe {
        host: host.to_owned(),
        port,
        seen: Arc::clone(&seen),
    };

    // Keep-alives are off: this connection has no life to keep.
    let attempt = client::connect(client_config(0), (host, port), handler);
    match tokio::time::timeout(timeout, attempt).await {
        Err(_) => Err(ShellError::ssh_timeout(host, port, whole_seconds(timeout))),
        Ok(Err(failure)) => match recorded(&seen) {
            // The probe refuses every key on purpose, so the handshake ends at the key
            // exchange — with the key in hand, which is the whole point of the call.
            Some(key) => Ok(key),
            None => Err(failure_envelope(host, port, timeout, failure)),
        },
        Ok(Ok(handle)) => {
            // Unreachable while the handler refuses every key, and still not an error: the
            // key is what was asked for. Dropping the handle ends the connection.
            drop(handle);
            recorded(&seen)
                .ok_or_else(|| ShellError::app_internal("the server completed a handshake without presenting a key"))
        }
    }
}

/// The SHA-256 fingerprint of a public key, in the `SHA256:<base64>` form OpenSSH prints.
///
/// Both spellings a public key travels in are accepted: the one-line `ssh-ed25519 AAAA… comment`
/// form that `ssh-keygen` writes and `authorized_keys` holds, and the bare base64 blob as the
/// protocol carries it and as a known-hosts record stores it.
pub fn fingerprint(public_key: &str) -> Result<String, ShellError> {
    Ok(parse_public_key(public_key)?.fingerprint(HashAlg::Sha256).to_string())
}

fn parse_public_key(public_key: &str) -> Result<PublicKey, ShellError> {
    let trimmed = public_key.trim();
    PublicKey::from_openssh(trimmed)
        .or_else(|_| russh::keys::parse_public_key_base64(trimmed))
        .map_err(|_| ShellError::ssh_key_invalid("the public key is not in the OpenSSH format"))
}

fn recorded(seen: &Arc<Mutex<Option<PresentedHostKey>>>) -> Option<PresentedHostKey> {
    seen.lock().ok().and_then(|slot| slot.clone())
}

/// The probe's handler: it remembers the key and refuses it.
struct Probe {
    host: String,
    port: u16,
    seen: Arc<Mutex<Option<PresentedHostKey>>>,
}

impl client::Handler for Probe {
    type Error = Failure;

    /// Refusing the key aborts the handshake at the key exchange. That is deliberate: nothing
    /// that follows a key exchange — an authentication offer above all — is sent for a key
    /// nobody has looked at yet.
    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let Some(key) = plain_key(server_key) else {
            return Err(Failure::Envelope(ShellError::app_unsupported("host certificates")));
        };
        if let Ok(mut slot) = self.seen.lock() {
            *slot = Some(presented(&self.host, self.port, key));
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_error::ErrorCode;

    /// A throwaway ed25519 public key, generated for this test and used by nothing. A public
    /// key is not a secret; the matching private half is not here.
    const KNOWN_PUBLIC_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJZgY1sRGM3nBMz9WNhJTLbHzAFPMEIkqYZH0bMhHJE7";

    /// What `ssh-keygen -lf` prints for the key above, so the test compares against a value
    /// this crate did not compute.
    const KNOWN_FINGERPRINT: &str = "SHA256:yYao+2QmDSRYkr0nsTjNEs6xMh73TQY1+utdxOFskUg";

    #[test]
    fn a_public_key_line_and_its_bare_blob_fingerprint_the_same() {
        let blob = KNOWN_PUBLIC_KEY.split_whitespace().nth(1).expect("a key blob");
        assert_eq!(fingerprint(KNOWN_PUBLIC_KEY).as_deref(), Ok(KNOWN_FINGERPRINT));
        assert_eq!(fingerprint(blob).as_deref(), Ok(KNOWN_FINGERPRINT));
    }

    #[test]
    fn a_comment_and_surrounding_whitespace_do_not_change_the_fingerprint() {
        let with_comment = format!("  {KNOWN_PUBLIC_KEY} person@example \n");
        assert_eq!(fingerprint(&with_comment).as_deref(), Ok(KNOWN_FINGERPRINT));
    }

    #[test]
    fn the_fingerprint_is_in_the_form_openssh_prints() {
        let printed = fingerprint(KNOWN_PUBLIC_KEY).expect("a valid key");
        let (prefix, body) = printed.split_once(':').expect("a prefixed fingerprint");
        assert_eq!(prefix, "SHA256");
        // Base64 without padding, which is what `ssh-keygen -l` shows.
        assert!(!body.contains('='));
        assert!(body.len() == 43);
    }

    #[test]
    fn something_that_is_not_a_public_key_is_rejected() {
        let error = fingerprint("not a key").expect_err("garbage is not a key");
        assert_eq!(error.code(), ErrorCode::SshKeyInvalid);
        assert_eq!(error.outcome(), sevenshell_error::OperationOutcome::NotSent);
    }

    #[test]
    fn a_key_is_only_accepted_when_both_its_type_and_material_match() {
        let key = parse_public_key(KNOWN_PUBLIC_KEY).expect("a valid key");
        let blob = key.public_key_base64();
        let expected = ExpectedKey::new("ssh-ed25519", &blob);
        assert!(expected.accepts(&key));

        // The same material under a different type is a different key: an RSA modulus is not
        // an ed25519 point, and a record that claims otherwise must not be honoured.
        assert!(!ExpectedKey::new("ssh-rsa", &blob).accepts(&key));
        // A different key under the same type is the mismatch the record exists to catch.
        let mut other = blob.clone();
        other.push('A');
        assert!(!ExpectedKey::new("ssh-ed25519", &other).accepts(&key));
    }

    #[tokio::test]
    async fn a_port_with_nothing_listening_is_unreachable() {
        // Bind a port, learn its number, then let it go: nothing is listening on it by the
        // time the probe arrives.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let port = listener.local_addr().expect("the bound address").port();
        drop(listener);

        let error = probe_host_key("127.0.0.1", port, Duration::from_secs(5))
            .await
            .expect_err("nothing is listening");
        assert_eq!(error.code(), ErrorCode::SshUnreachable);
        assert_eq!(error.outcome(), sevenshell_error::OperationOutcome::NotSent);
        assert_eq!(error.origin().peer(), Some(format!("127.0.0.1:{port}").as_str()));
    }

    #[tokio::test]
    async fn an_address_that_never_answers_times_out() {
        // A reserved address in the documentation range: nothing routes to it, and the clock
        // is what ends the attempt. The budget stays small so the test does.
        let timeout = Duration::from_millis(700);
        let error = probe_host_key("10.255.255.1", 22, timeout)
            .await
            .expect_err("nothing answers here");
        assert_eq!(error.code(), ErrorCode::SshTimeout);
        assert_eq!(error.message_params().get("seconds").map(String::as_str), Some("1"));
    }
}

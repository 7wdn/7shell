//! Credentials in, key material inspected, secrets dropped.
//!
//! [`Credential`] is the only value in this crate that holds a secret, and it holds it for
//! exactly as long as one connection attempt: the buffers are overwritten when it is dropped,
//! its `Debug` prints which kind of secret it is and nothing of it, and nothing here ever
//! formats one into a message.
//!
//! [`describe_key`] answers the other side of the same question: what a key *is*, without the
//! key. It reads the algorithm and the fingerprint out of a private key file and drops the
//! material, so the vault can keep a name and a fingerprint while the key itself stays where
//! it is.

use std::fmt;

use russh::keys::{Algorithm, HashAlg, PrivateKey, decode_secret_key};
use sevenshell_error::{MASK, ShellError};
use zeroize::Zeroize;

/// What to authenticate with. Secrets are moved in and dropped here.
///
/// Deliberately without `Debug`: a credential that reaches a log line, a panic message or a
/// bug report is a disclosure. The manual implementation prints what a person needs to know
/// — which kind of credential this is — and nothing about its contents.
pub enum Credential {
    /// A password. The same value answers a keyboard-interactive challenge, because many
    /// servers offer nothing else and a person only has one password to give.
    Password(String),
    /// The text of a private key file, and the passphrase that opens it if it is encrypted.
    Key {
        contents: String,
        passphrase: Option<String>,
    },
    /// A key held by the agent running on this machine (`ssh-agent`, or OpenSSH's on Windows).
    ///
    /// The agent signs; this build never sees the key. Which agent to ask comes from the
    /// environment, or from OpenSSH's own pipe on Windows.
    Agent,
    /// Pageant, which ships with PuTTY and speaks a protocol of its own.
    ///
    /// Kept separate from [`Credential::Agent`] rather than folded in: an answer of "no agent
    /// there" would be wrong on a machine where Pageant is running, and naming what is missing is
    /// the difference between a person starting the right program and reinstalling this one.
    Pageant,
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The mask comes from the error vocabulary, so a developer who greps for it finds
        // every place a secret was replaced rather than printed.
        match self {
            Self::Password(_) => write!(f, "Credential::Password({MASK})"),
            Self::Key { passphrase, .. } => {
                let passphrase = if passphrase.is_some() { "Some" } else { "None" };
                write!(f, "Credential::Key {{ contents: {MASK}, passphrase: {passphrase} }}")
            }
            Self::Agent => write!(f, "Credential::Agent"),
            Self::Pageant => write!(f, "Credential::Pageant"),
        }
    }
}

impl Credential {
    /// Whether this is the machine's own agent.
    pub fn agent(&self) -> bool {
        matches!(self, Self::Agent)
    }

    /// Whether this is Pageant, which speaks a protocol this build does not.
    pub fn pageant(&self) -> bool {
        matches!(self, Self::Pageant)
    }
}

impl Drop for Credential {
    fn drop(&mut self) {
        // Overwrite before the allocator sees the buffer again. The strings travel through
        // the protocol library as plain copies once they are handed over; the copy that this
        // crate owns is the one it can promise to wipe.
        match self {
            Self::Password(password) => password.zeroize(),
            Self::Key { contents, passphrase } => {
                contents.zeroize();
                passphrase.zeroize();
            }
            // Nothing of the caller's to wipe: the key never left the agent.
            Self::Agent | Self::Pageant => {}
        }
    }
}

impl Credential {
    /// The password, for the exchange that carries it.
    pub(crate) fn password(&self) -> Option<&str> {
        match self {
            Self::Password(password) => Some(password),
            Self::Key { .. } | Self::Agent | Self::Pageant => None,
        }
    }

    /// The key file's text and its passphrase, for the exchange that signs with it.
    pub(crate) fn key(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Self::Key { contents, passphrase } => Some((contents, passphrase.as_deref())),
            Self::Password(_) | Self::Agent | Self::Pageant => None,
        }
    }
}

/// What a private key is, without the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDescription {
    /// The key type as OpenSSH spells it, for example `ssh-ed25519` or `ssh-rsa`.
    pub algorithm: String,
    /// The fingerprint in the `SHA256:<base64>` form `ssh-keygen -l` prints, which is what a
    /// person compares with what the server shows them.
    pub fingerprint: String,
    /// Whether the file is encrypted, and so needs its passphrase before it can sign.
    pub has_passphrase: bool,
}

/// Reads a private key to learn its type and fingerprint without keeping it.
///
/// A passphrase is only needed to open the key; an encrypted key whose public half is stored
/// in the clear — every OpenSSH-format key — is described without one, which is what lets the
/// interface show a fingerprint before it asks for the passphrase. A key that is encrypted in
/// a container that hides even the public half (PKCS#8) is reported as invalid until the
/// passphrase arrives.
pub fn describe_key(contents: &str, passphrase: Option<&str>) -> Result<KeyDescription, ShellError> {
    reject_certificate(contents)?;

    match decode_secret_key(contents, None) {
        // An unencrypted key needs no passphrase, and one handed over anyway is ignored the
        // way OpenSSH ignores it, rather than used to fail a key that reads perfectly well.
        Ok(key) => Ok(description(&key, false)),
        Err(unopened) => match passphrase {
            // A passphrase that does not open the key is not a reason to describe it anyway:
            // reporting a fingerprint for a key the caller cannot use hides the failure.
            Some(passphrase) => decode_secret_key(contents, Some(passphrase))
                .map(|key| description(&key, true))
                .map_err(key_error),
            None => match public_half(contents) {
                // Encrypted, with the public half kept outside the encrypted part: the
                // fingerprint is there to compare, and what is missing is the passphrase.
                Some(key) => Ok(description(&key, true)),
                None => Err(key_error(unopened)),
            },
        },
    }
}

/// The key a connection authenticates with, read from its file text.
///
/// Kept private to the crate: the key material itself must not travel any further than the
/// signer that uses it.
pub(crate) fn load_private_key(contents: &str, passphrase: Option<&str>) -> Result<PrivateKey, ShellError> {
    reject_certificate(contents)?;
    let key = decode_secret_key(contents, passphrase).map_err(key_error)?;
    match key.algorithm() {
        // The kinds OpenSSH and the servers built around it use for keys people hold.
        Algorithm::Ed25519 | Algorithm::Ecdsa { .. } | Algorithm::Rsa { .. } => Ok(key),
        // FIDO keys, DSA keys, and anything newer than this build: named rather than left to
        // fail at signing time with a message that names nothing.
        other => Err(ShellError::ssh_key_unsupported(other.as_str())),
    }
}

/// A user or host certificate is not a private key, and this build carries no certificate
/// authority to check one against.
fn reject_certificate(contents: &str) -> Result<(), ShellError> {
    if contents.contains("-cert-v01@openssh.com") {
        return Err(ShellError::app_unsupported("SSH certificates"));
    }
    Ok(())
}

/// The public half of a key file, when the format keeps it outside the encrypted part.
///
/// Only the OpenSSH container does. It is what makes a fingerprint available before a
/// passphrase has been typed.
pub(crate) fn public_half(contents: &str) -> Option<PrivateKey> {
    if !contents.contains("-----BEGIN OPENSSH PRIVATE KEY-----") {
        return None;
    }
    PrivateKey::from_openssh(contents).ok()
}

fn description(key: &PrivateKey, has_passphrase: bool) -> KeyDescription {
    KeyDescription {
        algorithm: key.algorithm().as_str().to_owned(),
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        has_passphrase,
    }
}

/// Names a failure to read a key.
///
/// The message comes from the decoder and names a reason, never the material: a wrong
/// passphrase and a file that is not a key at all both end up as `SSH-KEY-INVALID`, because
/// telling them apart would let a caller test passphrases against a key file. A key type the
/// build cannot use gets its own code so the interface can say which type is not carried.
fn key_error(error: russh::keys::Error) -> ShellError {
    match error {
        russh::keys::Error::UnsupportedKeyType { key_type_string, .. } => {
            ShellError::ssh_key_unsupported(&key_type_string)
        }
        russh::keys::Error::UnknownAlgorithm(identifier) => ShellError::ssh_key_unsupported(&identifier.to_string()),
        other => ShellError::ssh_key_invalid(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::ssh_key::LineEnding;
    use russh::keys::ssh_key::private::{KeypairData, RsaKeypair};
    use sevenshell_error::{ErrorCode, OperationOutcome};

    /// A file that is not a key at all.
    const GARBAGE: &str = "this is not a key at all";

    /// The passphrase the encrypted fixtures are locked with. It protects nothing: the key it
    /// opens is generated for the test and dropped with it.
    const PASSPHRASE: &str = "a passphrase for the test";

    /// Small enough that generating one in a debug build stays quick. The size of the modulus
    /// is not what these tests are about — the algorithm the file declares is.
    const RSA_BITS: usize = 1024;

    /// A key of the given kind, generated in the test and thrown away with it.
    fn generated(algorithm: Algorithm) -> PrivateKey {
        let mut rng = russh::keys::key::safe_rng();
        PrivateKey::random(&mut rng, algorithm).expect("a generated key")
    }

    fn rsa() -> PrivateKey {
        let mut rng = russh::keys::key::safe_rng();
        let pair = RsaKeypair::random(&mut rng, RSA_BITS).expect("a generated keypair");
        PrivateKey::new(KeypairData::Rsa(pair), "").expect("key data for the generated pair")
    }

    /// The text of a key file, which is what a caller reads off the disk.
    fn text(key: &PrivateKey) -> String {
        key.to_openssh(LineEnding::LF).expect("a serialized key").to_string()
    }

    fn encrypted(key: &PrivateKey) -> String {
        let mut rng = russh::keys::key::safe_rng();
        let locked = key.encrypt(&mut rng, PASSPHRASE).expect("an encrypted key");
        text(&locked)
    }

    #[test]
    fn a_password_is_not_printable() {
        let credential = Credential::Password("hunter2".to_owned());
        let printed = format!("{credential:?}");
        assert!(!printed.contains("hunter2"));
        assert!(printed.contains(MASK));
    }

    #[test]
    fn key_material_and_its_passphrase_are_not_printable() {
        let credential = Credential::Key {
            contents: "-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n-----END OPENSSH PRIVATE KEY-----".to_owned(),
            passphrase: Some("hunter2".to_owned()),
        };
        let printed = format!("{credential:?}");
        assert!(!printed.contains("hunter2"));
        assert!(!printed.contains("secret"));
        assert!(printed.contains(MASK));
    }

    #[test]
    fn credentials_are_dropped_without_panicking() {
        // Dropping is where the zeroizing happens; the test is that it is reachable from a
        // credential that was moved into a request and never used.
        let credential = Credential::Key {
            contents: GARBAGE.to_owned(),
            passphrase: None,
        };
        assert!(credential.key().is_some());
        assert!(credential.password().is_none());
        drop(credential);
    }

    #[test]
    fn an_ed25519_key_is_described_by_the_name_openssh_gives_it() {
        let key = generated(Algorithm::Ed25519);
        let described = describe_key(&text(&key), None).expect("a readable key");
        assert_eq!(described.algorithm, "ssh-ed25519");
        assert!(!described.has_passphrase);
        // The same value the library computes from the key object, which is what a person
        // compares with the server's own `ssh-keygen -l`.
        assert_eq!(described.fingerprint, key.fingerprint(HashAlg::Sha256).to_string());
        assert!(described.fingerprint.starts_with("SHA256:"));
    }

    #[test]
    fn an_rsa_key_is_described_as_rsa() {
        let key = rsa();
        let described = describe_key(&text(&key), None).expect("a readable key");
        assert_eq!(described.algorithm, "ssh-rsa");
        assert!(!described.has_passphrase);
    }

    #[test]
    fn an_ecdsa_key_is_described_with_its_curve() {
        let key = generated(Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP256,
        });
        let described = describe_key(&text(&key), None).expect("a readable key");
        assert_eq!(described.algorithm, "ecdsa-sha2-nistp256");
        assert!(!described.has_passphrase);
    }

    #[test]
    fn an_encrypted_key_reports_that_it_needs_a_passphrase() {
        let key = generated(Algorithm::Ed25519);
        let locked = encrypted(&key);

        // Without the passphrase: the fingerprint is still there to compare, because the
        // OpenSSH container keeps the public half outside the encrypted part.
        let described = describe_key(&locked, None).expect("a describable key");
        assert!(described.has_passphrase);
        assert_eq!(described.algorithm, "ssh-ed25519");
        assert_eq!(described.fingerprint, key.fingerprint(HashAlg::Sha256).to_string());

        // With it: the same description, and the key itself can be used.
        let opened = describe_key(&locked, Some(PASSPHRASE)).expect("the right passphrase");
        assert!(opened.has_passphrase);
        assert_eq!(opened.fingerprint, described.fingerprint);
        assert!(load_private_key(&locked, Some(PASSPHRASE)).is_ok());
    }

    #[test]
    fn a_wrong_passphrase_is_rejected_rather_than_described() {
        let locked = encrypted(&generated(Algorithm::Ed25519));
        let error = describe_key(&locked, Some("not the passphrase")).expect_err("the wrong passphrase");
        assert_eq!(error.code(), ErrorCode::SshKeyInvalid);
        assert_eq!(error.outcome(), OperationOutcome::NotSent);
        assert!(!error.to_string().contains("not the passphrase"));
        assert!(load_private_key(&locked, Some("not the passphrase")).is_err());
    }

    #[test]
    fn an_encrypted_key_without_its_passphrase_cannot_be_used_to_authenticate() {
        let locked = encrypted(&generated(Algorithm::Ed25519));
        let error = load_private_key(&locked, None).expect_err("the key is locked");
        assert_eq!(error.code(), ErrorCode::SshKeyInvalid);
        assert_eq!(error.outcome(), OperationOutcome::NotSent);
    }

    #[test]
    fn a_key_kind_this_build_cannot_use_is_named_rather_than_left_obscure() {
        let error = key_error(russh::keys::Error::UnsupportedKeyType {
            key_type_string: "ssh-dss".to_owned(),
            key_type_raw: Vec::new(),
        });
        assert_eq!(error.code(), ErrorCode::SshKeyUnsupported);
        assert_eq!(error.diagnostics().get("keyType").map(String::as_str), Some("ssh-dss"));
        assert_eq!(error.outcome(), OperationOutcome::NotSent);
    }

    #[test]
    fn garbage_is_rejected_rather_than_guessed_at() {
        let error = describe_key(GARBAGE, None).expect_err("garbage is not a key");
        assert_eq!(error.code(), ErrorCode::SshKeyInvalid);
    }

    #[test]
    fn a_wrong_passphrase_is_rejected_without_panicking() {
        let error = describe_key(GARBAGE, Some("hunter2")).expect_err("garbage stays garbage");
        assert_eq!(error.code(), ErrorCode::SshKeyInvalid);
        assert!(!error.to_string().contains("hunter2"));
    }

    #[test]
    fn an_empty_file_is_rejected() {
        assert!(describe_key("", None).is_err());
        assert!(describe_key("   \n", Some("hunter2")).is_err());
    }

    #[test]
    fn a_certificate_is_named_as_one_instead_of_read_as_a_key() {
        let certificate = "ssh-ed25519-cert-v01@openssh.com AAAAB3NzaC1yc2E cert@example";
        let error = describe_key(certificate, None).expect_err("a certificate is not a private key");
        assert_eq!(error.code(), ErrorCode::AppUnsupported);
        assert!(load_private_key(certificate, None).is_err());
    }
}

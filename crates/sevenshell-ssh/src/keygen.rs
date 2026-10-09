//! Making a key, rather than importing one.
//!
//! An SSH key is two halves with two different lives: the private one never leaves the vault and is
//! what a connection signs with, and the public one is meant to be copied onto machines. This module
//! produces both from nothing — the private half in the OpenSSH container, which is the format
//! `ssh-keygen` writes and every OpenSSH-compatible server accepts, and the public half as the single
//! line that goes into `authorized_keys`.
//!
//! Three algorithms, and they are the three OpenSSH itself offers:
//!
//! * **Ed25519** — the one to pick. Small, fast, and what a modern server is configured for.
//! * **ECDSA (P-256)** — for a server whose policy names a NIST curve.
//! * **RSA** — for a server old enough to accept nothing else. It is the only one that takes
//!   noticeable time, because a large prime has to be found rather than drawn.
//!
//! The randomness comes from the connection library's own operating-system source, not from a
//! generator shaped here: a key made from a weak source is the one failure nobody notices afterwards,
//! and no file in this repository should let somebody say the application rolled its own.
//!
//! Nothing here keeps anything. The private half is moved into the vault and wiped with it, and the
//! `Debug` implementation counts rather than prints — key material is exactly what must not reach a
//! log line.

use russh::keys::key::safe_rng;
use russh::keys::ssh_key::private::{EcdsaKeypair, Ed25519Keypair, KeypairData, RsaKeypair};
use russh::keys::ssh_key::{EcdsaCurve, LineEnding, PrivateKey};
use sevenshell_error::ShellError;
use sevenshell_types::KeyAlgorithm;
use zeroize::Zeroizing;

use crate::credential::{KeyDescription, describe_key};

/// How many bits an RSA key gets.
///
/// The size `ssh-keygen` chooses when it is not told, which is the size somebody comparing two keys
/// by eye expects to see. Generation takes seconds here and minutes at 4096, and a server that needs
/// RSA at all is a server this satisfies.
const RSA_BITS: usize = 3072;

/// A key that was just made: the private half, the public half, and what it is.
pub struct GeneratedKey {
    private_text: Zeroizing<String>,
    public_line: String,
    description: KeyDescription,
}

impl GeneratedKey {
    /// The private half's text, for the one caller that puts it in the vault.
    pub fn private_text(&self) -> &str {
        self.private_text.as_str()
    }

    /// The line a person pastes into `authorized_keys` on a server.
    pub fn public_line(&self) -> &str {
        &self.public_line
    }

    /// What the key is: its type, its fingerprint, and whether a passphrase protects it.
    pub fn description(&self) -> &KeyDescription {
        &self.description
    }
}

impl std::fmt::Debug for GeneratedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // By hand, for the reason every secret in this workspace has one: a derived `Debug` would put
        // a private key one `{:?}` away from a log line. What is worth seeing while debugging is what
        // the key *is*, and that is the description.
        f.debug_struct("GeneratedKey")
            .field("algorithm", &self.description.algorithm)
            .field("fingerprint", &self.description.fingerprint)
            .field("has_passphrase", &self.description.has_passphrase)
            .field("private_text", &format_args!("<{} bytes>", self.private_text.len()))
            .finish()
    }
}

/// Makes a key of the given kind, with an optional passphrase protecting the private half.
///
/// The comment is the trailing part of the public line — conventionally `user@host` — and travels
/// with the key: it is what the server's `authorized_keys` will show, which is how somebody tells
/// two of their own keys apart a year later.
///
/// This is the slow part of making a key and the only function here that is. RSA against a 3072-bit
/// modulus is seconds of arithmetic, so a caller that draws a window around it must keep it off the
/// thread that draws.
pub fn generate(algorithm: KeyAlgorithm, comment: &str, passphrase: Option<&str>) -> Result<GeneratedKey, ShellError> {
    let mut rng = safe_rng();
    let data = match algorithm {
        KeyAlgorithm::Ed25519 => KeypairData::Ed25519(Ed25519Keypair::random(&mut rng)),
        KeyAlgorithm::Ecdsa => {
            let pair = EcdsaKeypair::random(&mut rng, EcdsaCurve::NistP256).map_err(could_not_make)?;
            KeypairData::Ecdsa(pair)
        }
        KeyAlgorithm::Rsa => {
            let pair = RsaKeypair::random(&mut rng, RSA_BITS).map_err(could_not_make)?;
            KeypairData::Rsa(pair)
        }
    };

    let key = PrivateKey::new(data, comment).map_err(could_not_make)?;

    // The public half first: it is what the comment belongs to, and it is the same whichever
    // container the private half ends up in.
    let public_line = key.public_key().to_openssh().map_err(could_not_make)?;

    let private_text = match passphrase {
        // The key comes back in the encrypted container, and `to_openssh` writes that container out
        // rather than the key inside it — which is the whole point of asking for the passphrase.
        Some(passphrase) => key
            .encrypt(&mut safe_rng(), passphrase)
            .map_err(could_not_make)?
            .to_openssh(LineEnding::LF)
            .map_err(could_not_make)?,
        None => key.to_openssh(LineEnding::LF).map_err(could_not_make)?,
    };

    // Described by *reading back* what was written rather than by reporting what was asked for. The
    // interface shows this fingerprint and a person compares it with `ssh-keygen -lf`, so the number
    // has to come from the key; and on the way it proves that the passphrase just used opens the key
    // that was just written, which is otherwise only discovered when somebody tries to connect.
    let description = describe_key(&private_text, passphrase).map_err(|error| {
        ShellError::app_internal(format!("the key that was just generated cannot be read back: {error}"))
    })?;

    Ok(GeneratedKey {
        private_text,
        public_line,
        description,
    })
}

/// The `authorized_keys` line for a private key the vault already holds.
///
/// The public half of a key is derived, never stored: an imported key file has one whether or not
/// the person who imported it kept a copy, and pasting it into a server's `authorized_keys` is the
/// whole of what has to happen for that key to be usable there. An encrypted OpenSSH-format key
/// carries its public half in the clear, so this answers without a passphrase; a key in a container
/// that hides it (PKCS#8) needs the passphrase it was stored with.
pub fn public_line(contents: &str, passphrase: Option<&str>) -> Result<String, ShellError> {
    // `load_private_key` is the same reader a connection uses, so a key whose private half cannot be
    // opened here is reported the way a connection would report it. When it cannot be opened at all
    // — a locked key — the public half is read from the clear part of the OpenSSH container, which
    // is what makes the line available before anybody has typed a passphrase.
    let key = match crate::credential::load_private_key(contents, passphrase) {
        Ok(key) => key,
        Err(locked) => crate::credential::public_half(contents).ok_or(locked)?,
    };

    key.public_key()
        .to_openssh()
        .map_err(|error| ShellError::ssh_key_invalid(format!("the public half could not be written: {error}")))
}

/// Names a failure inside the key library.
///
/// Not an invalid key and not an unsupported one: nothing the person did caused this, and no
/// fingerprint they could compare would help. It is the machine or the library failing, which is what
/// the internal sentence is for.
fn could_not_make(error: impl std::fmt::Display) -> ShellError {
    ShellError::app_internal(format!("a key could not be generated: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A comment like the one `ssh-keygen` puts on a key by default.
    const COMMENT: &str = "someone@example";

    /// The passphrase the protected-key test locks its key with. It protects nothing: the key it
    /// opens is generated here and dropped with the test.
    const PASSPHRASE: &str = "a passphrase for the test";

    #[test]
    fn each_kind_of_key_is_made_and_reads_back_as_itself() {
        for algorithm in KeyAlgorithm::ALL {
            let key = generate(*algorithm, COMMENT, None).expect("a generated key");

            assert_eq!(
                key.description().algorithm,
                algorithm.openssh_name(),
                "{algorithm} came back as another kind of key"
            );
            assert!(
                key.private_text().contains("-----BEGIN OPENSSH PRIVATE KEY-----"),
                "{algorithm} was not written in the container OpenSSH reads"
            );
            assert!(
                key.public_line().starts_with(algorithm.openssh_name()),
                "{algorithm}'s public line starts with something else: {}",
                key.public_line()
            );
            assert!(
                key.public_line().ends_with(COMMENT),
                "the comment travels with the key: {}",
                key.public_line()
            );

            // The description is read back out of the key, so what a person compares with
            // `ssh-keygen -lf` is the fingerprint of the key that was actually written.
            let reread = describe_key(key.private_text(), None).expect("the key reads back");
            assert_eq!(reread, *key.description(), "{algorithm} described itself differently");
            assert!(!reread.has_passphrase);
        }
    }

    #[test]
    fn the_two_halves_of_a_generated_key_are_the_same_key() {
        let key = generate(KeyAlgorithm::Ed25519, COMMENT, None).expect("a generated key");

        // The public half is followed by the comment, and the fingerprint is computed from the key
        // itself — which is the same fingerprint the private half reports. Two keys that disagreed
        // here would authenticate against nothing, and the failure would look like a server problem.
        assert_eq!(
            crate::hostkey::fingerprint(key.public_line()).expect("a public key"),
            key.description().fingerprint
        );
    }

    #[test]
    fn a_key_asked_for_with_a_passphrase_is_locked_with_it() {
        let key = generate(KeyAlgorithm::Ed25519, COMMENT, Some(PASSPHRASE)).expect("a generated key");

        assert!(key.description().has_passphrase);
        assert!(
            describe_key(key.private_text(), None)
                .expect("the public half is readable")
                .has_passphrase,
            "an OpenSSH key says it is locked before anybody types the passphrase"
        );
        assert!(
            describe_key(key.private_text(), Some("not the passphrase")).is_err(),
            "a key that opened with the wrong passphrase would be one anybody could use"
        );
        assert!(describe_key(key.private_text(), Some(PASSPHRASE)).is_ok());

        // And the public half is available for a locked key, which is what makes it possible to put
        // the key on a server before anybody has typed the passphrase anywhere. The comment is the
        // part that can go missing — a locked key keeps it inside the encrypted section — so what is
        // compared is the key itself.
        assert_eq!(
            identity(&public_line(key.private_text(), None).expect("the public half is in the clear")),
            identity(key.public_line())
        );
    }

    #[test]
    fn the_public_half_of_an_imported_key_is_derived_from_it() {
        // The other half of "export the public key": a key that was imported has a public half
        // whether or not the file it came from is still on this machine.
        let key = generate(KeyAlgorithm::Ecdsa, COMMENT, Some(PASSPHRASE)).expect("a generated key");

        assert_eq!(
            identity(&public_line(key.private_text(), Some(PASSPHRASE)).expect("read with the passphrase")),
            identity(key.public_line())
        );
        assert_eq!(
            identity(&public_line(key.private_text(), None).expect("read without it, from the clear half")),
            identity(key.public_line())
        );

        // A key with no passphrase keeps its comment through the round trip, which is what somebody
        // comparing the line they exported with the file they imported will see.
        let plain = generate(KeyAlgorithm::Ed25519, COMMENT, None).expect("a generated key");
        assert_eq!(
            public_line(plain.private_text(), None).expect("read"),
            plain.public_line()
        );
    }

    /// The identity of a public line: the type and the key, without the comment after them.
    fn identity(line: &str) -> String {
        line.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn two_keys_are_never_the_same_key() {
        let first = generate(KeyAlgorithm::Ed25519, COMMENT, None).expect("a generated key");
        let second = generate(KeyAlgorithm::Ed25519, COMMENT, None).expect("another");

        assert_ne!(
            first.description().fingerprint,
            second.description().fingerprint,
            "two keys made one after the other came out identical"
        );
    }

    #[test]
    fn a_generated_key_is_never_printed() {
        let key = generate(KeyAlgorithm::Ed25519, COMMENT, None).expect("a generated key");

        let printed = format!("{key:?}");

        assert!(
            !printed.contains("BEGIN OPENSSH PRIVATE KEY"),
            "key material reached Debug output: {printed}"
        );
        assert!(
            printed.contains(&key.description().fingerprint),
            "the fingerprint is what is worth seeing"
        );
    }
}

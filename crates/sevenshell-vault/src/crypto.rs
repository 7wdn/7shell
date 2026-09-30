//! Master-key derivation and authenticated encryption for the vault.
//!
//! Four decisions, and why they are not negotiable:
//!
//! * The document is **one AEAD blob**, never a field at a time. Sealing individual fields is
//!   how nonce reuse and length leaks happen, and it would also make a session count readable
//!   from a file an attacker already has.
//! * Derivation is **Argon2id** with the cost recorded in the file, so a future release can
//!   raise it while vaults written by this one keep opening.
//! * Each ciphertext is bound to a context through the AEAD's associated data, so the payload
//!   can never be replayed as the verifier or the other way round.
//! * Randomness comes from the operating system through `getrandom`, not from a re-exported
//!   `OsRng` whose version would have to track the cipher crate's.
//!
//! The two associated-data strings are fixed by the file format: an existing vault must keep
//! opening after this crate is rewritten, so changing them is a migration, not an edit.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use sevenshell_error::ShellError;
use zeroize::Zeroizing;

/// The derived key length, in bytes. XChaCha20-Poly1305 uses 32.
pub const KEY_LEN: usize = 32;

/// The salt length, in bytes. Argon2's own recommendation, and enough that two vaults never
/// share a derived key.
pub const SALT_LEN: usize = 16;

/// The XChaCha20-Poly1305 nonce length, in bytes, prepended to every ciphertext.
pub const NONCE_LEN: usize = 24;

/// The only key derivation function this build writes or reads.
pub const KDF_NAME: &str = "argon2id";

/// 64 MiB, three passes, one lane: comfortably above the OWASP minimum and about a fifth of a
/// second on a desktop, which is slow enough to hurt a guessing attacker and fast enough that
/// unlocking does not feel broken.
pub const KDF_MEMORY_KIB: u32 = 64 * 1024;
pub const KDF_ITERATIONS: u32 = 3;
pub const KDF_PARALLELISM: u32 = 1;

/// The Argon2 version this build derives with, pinned in the file format by convention.
const ARGON2_VERSION: Version = Version::V0x13;

/// Binds the verifier ciphertext to its role in the envelope.
pub(crate) const AAD_VERIFIER: &[u8] = b"sevenshell/vault-verifier/v1";

/// Binds the document ciphertext to its role in the envelope.
pub(crate) const AAD_PAYLOAD: &[u8] = b"sevenshell/vault-payload/v1";

/// What the verifier decrypts to when the password is right. Its value only has to be fixed:
/// it is never stored anywhere an attacker can see it.
const VERIFIER_PLAINTEXT: &[u8] = b"sevenshell-vault-verifier";

/// A derived key, wiped on drop and never printed.
pub(crate) struct VaultKey(Zeroizing<[u8; KEY_LEN]>);

impl VaultKey {
    fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey(<redacted>)")
    }
}

/// A fresh base64 salt for a new vault.
pub(crate) fn random_salt() -> Result<String, ShellError> {
    Ok(BASE64.encode(random_bytes::<SALT_LEN>()?))
}

/// Derives the vault key from the master password.
///
/// Every failure here is a `VAULT-KDF-FAILED`: the parameters in the file are unusable, which
/// is a different kind of problem from a password that does not match, and the parameters are
/// public metadata anyway.
pub(crate) fn derive_key(password: &str, salt: &str, kdf: &KdfParams) -> Result<VaultKey, ShellError> {
    if kdf.name != KDF_NAME {
        return Err(kdf_failed(format!("key derivation '{}' is not supported", kdf.name)));
    }

    let salt = BASE64
        .decode(salt)
        .map_err(|_| kdf_failed("the salt is not valid base64".to_owned()))?;
    if salt.len() != SALT_LEN {
        return Err(kdf_failed(format!("the salt is {} bytes, not {SALT_LEN}", salt.len())));
    }

    let params = Params::new(kdf.memory_kib, kdf.iterations, kdf.parallelism, Some(KEY_LEN))
        .map_err(|error| kdf_failed(format!("invalid Argon2 parameters: {error}")))?;

    let argon = Argon2::new(Algorithm::Argon2id, ARGON2_VERSION, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password.as_bytes(), &salt, key.as_mut())
        .map_err(|error| kdf_failed(format!("derivation failed: {error}")))?;

    Ok(VaultKey(key))
}

/// Encrypts `plaintext`, returning base64 of `nonce || ciphertext || tag`.
pub(crate) fn encrypt(key: &VaultKey, plaintext: &[u8], aad: &[u8]) -> Result<String, ShellError> {
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_bytes())
        .map_err(|error| internal(format!("the cipher could not be initialised: {error}")))?;
    let nonce_bytes = random_bytes::<NONCE_LEN>()?;
    let ciphertext = cipher
        .encrypt(&XNonce::from(nonce_bytes), Payload { msg: plaintext, aad })
        .map_err(|error| internal(format!("encryption failed: {error}")))?;

    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(blob))
}

/// Decrypts a blob produced by [`encrypt`].
///
/// Everything that can go wrong here reports `VAULT-UNLOCK-FAILED` with no detail: a wrong
/// password, a truncated blob, a flipped byte and a blob encrypted under a different context
/// are the same event to a caller, and telling them apart would turn the error into an oracle
/// for anyone holding a copy of the file.
pub(crate) fn decrypt(key: &VaultKey, blob: &str, aad: &[u8]) -> Result<Vec<u8>, ShellError> {
    let raw = BASE64.decode(blob).map_err(|_| unlock_failed())?;
    if raw.len() <= NONCE_LEN {
        return Err(unlock_failed());
    }

    let (nonce_bytes, ciphertext) = raw.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_bytes()).map_err(|_| unlock_failed())?;
    cipher
        .decrypt(
            &XNonce::try_from(nonce_bytes).map_err(|_| unlock_failed())?,
            Payload { msg: ciphertext, aad },
        )
        .map_err(|_| unlock_failed())
}

/// Decrypts the verifier and proves it says what a verifier says.
///
/// A blob that decrypts to something else was produced by something other than this format,
/// and is reported exactly like a wrong password.
pub(crate) fn decrypt_verifier(key: &VaultKey, blob: &str) -> Result<(), ShellError> {
    let plaintext = Zeroizing::new(decrypt(key, blob, AAD_VERIFIER)?);
    if plaintext.as_slice() != VERIFIER_PLAINTEXT {
        return Err(unlock_failed());
    }
    Ok(())
}

/// The verifier for a freshly derived key.
pub(crate) fn encrypt_verifier(key: &VaultKey) -> Result<String, ShellError> {
    encrypt(key, VERIFIER_PLAINTEXT, AAD_VERIFIER)
}

/// The KDF parameters as they are recorded in the envelope. The file is the authority: these
/// values are read back on every unlock so a vault written with different costs still opens.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KdfParams {
    /// `argon2id` is the only name this build accepts.
    pub name: String,
    /// Memory cost in KiB.
    pub memory_kib: u32,
    /// Number of passes.
    pub iterations: u32,
    /// Degree of parallelism.
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            name: KDF_NAME.to_owned(),
            memory_kib: KDF_MEMORY_KIB,
            iterations: KDF_ITERATIONS,
            parallelism: KDF_PARALLELISM,
        }
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], ShellError> {
    let mut buffer = [0u8; N];
    getrandom::fill(&mut buffer)
        .map_err(|error| internal(format!("the operating system random source failed: {error}")))?;
    Ok(buffer)
}

fn kdf_failed(detail: String) -> ShellError {
    // The detail stays in the envelope's scrubbed slot; it never contains key material, only
    // the shape of the parameters that could not be used.
    ShellError::vault_kdf_failed().with_detail(detail)
}

fn unlock_failed() -> ShellError {
    ShellError::vault_unlock_failed()
}

fn internal(detail: String) -> ShellError {
    ShellError::app_internal(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_params() -> KdfParams {
        KdfParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
            ..KdfParams::default()
        }
    }

    #[test]
    fn a_key_round_trips_through_its_blob() {
        let salt = random_salt().expect("salt");
        let key = derive_key("correct horse battery staple", &salt, &fast_params()).expect("derive");

        let blob = encrypt(&key, b"session data", AAD_PAYLOAD).expect("encrypt");

        assert_eq!(decrypt(&key, &blob, AAD_PAYLOAD).expect("decrypt"), b"session data");
    }

    #[test]
    fn a_different_password_derives_a_different_key() {
        let salt = random_salt().expect("salt");
        let right = derive_key("right", &salt, &fast_params()).expect("derive");
        let wrong = derive_key("wrong", &salt, &fast_params()).expect("derive");

        let blob = encrypt(&right, b"secret", AAD_PAYLOAD).expect("encrypt");

        assert!(decrypt(&wrong, &blob, AAD_PAYLOAD).is_err());
    }

    #[test]
    fn a_blob_cannot_be_replayed_under_the_other_context() {
        let salt = random_salt().expect("salt");
        let key = derive_key("pw", &salt, &fast_params()).expect("derive");

        let blob = encrypt(&key, VERIFIER_PLAINTEXT, AAD_VERIFIER).expect("encrypt");

        assert!(decrypt(&key, &blob, AAD_PAYLOAD).is_err());
        assert_eq!(decrypt_verifier(&key, &blob), Ok(()));
    }

    #[test]
    fn two_salts_and_two_blobs_differ() {
        let salt = random_salt().expect("salt");
        let key = derive_key("pw", &salt, &fast_params()).expect("derive");

        let first = encrypt(&key, b"same plaintext", AAD_PAYLOAD).expect("encrypt");
        let second = encrypt(&key, b"same plaintext", AAD_PAYLOAD).expect("encrypt");

        assert_ne!(random_salt().expect("salt"), salt);
        // A deterministic nonce would make these equal, and that would be a bug worth failing on.
        assert_ne!(first, second);
    }

    #[test]
    fn an_unknown_kdf_name_is_refused_as_a_kdf_failure() {
        let salt = random_salt().expect("salt");
        let params = KdfParams {
            name: "scrypt".to_owned(),
            ..fast_params()
        };

        let error = derive_key("pw", &salt, &params).expect_err("scrypt is not implemented");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::VaultKdfFailed);
    }

    #[test]
    fn a_malformed_salt_is_refused() {
        let error = derive_key("pw", "not base64!", &fast_params()).expect_err("salt");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::VaultKdfFailed);
    }

    #[test]
    fn the_default_parameters_are_the_ones_the_format_promises() {
        let params = KdfParams::default();

        assert_eq!(params.name, "argon2id");
        assert_eq!(params.memory_kib, 65536);
        assert_eq!(params.iterations, 3);
        assert_eq!(params.parallelism, 1);
    }
}

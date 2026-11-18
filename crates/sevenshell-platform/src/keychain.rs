//! The system account's store for the vault's unlock material.
//!
//! The vault keeps its own encryption and its own file format. This module is only where the
//! material that opens it may rest outside the process, under the platform's own account
//! protection:
//!
//! * **Windows** — DPAPI (`CryptProtectData`) seals the bytes, and they live in one owner-only file
//!   beside the vault. The blob is bound to the current Windows account; copying the file to another
//!   machine or account makes it useless.
//! * **macOS** — the login keychain holds one generic-password item, so the platform decides who can
//!   read it and the item survives application updates while the signing identity does.
//! * **Everything else** — unavailable. A desktop without a keychain falls back to the master
//!   password, which is the honest answer rather than a plaintext file.
//!
//! The bytes stored here are the caller's business: this module never parses them, never prints
//! them and wipes them where it can.

use std::fmt;

use zeroize::Zeroizing;

/// The service name on macOS. Kept in step with `tauri.conf.json`'s identifier by the callers.
pub const SERVICE: &str = "com.sevenshell.app";

/// The one keychain item this application keeps: the vault's unlock material.
pub const VAULT_UNLOCK_ACCOUNT: &str = "vault-unlock";

/// The sealed file's name on Windows, inside the configuration directory.
pub const SEALED_FILE_NAME: &str = "vault-unlock.sealed";

/// A secret larger than this is not a derived key, and writing it would only hide a bug.
const MAX_SECRET_BYTES: usize = 4096;

/// macOS's `errSecItemNotFound`, stable in the Security framework's OSStatus space.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25_300;

/// Why the system store could not be used.
///
/// No variant carries the secret. A message is for a person reading a log, and the caller turns
/// [`KeychainError::Unavailable`] into "this machine has no keychain" rather than an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeychainError {
    /// The platform has no system store this build can use.
    Unavailable,
    /// The caller handed over something too large to be unlock material.
    TooLarge,
    /// The platform store refused or failed. The message names the operation, never the bytes.
    Failed(String),
}

impl fmt::Display for KeychainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("this platform has no system keychain"),
            Self::TooLarge => f.write_str("the secret is too large for the system keychain"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

/// Whether this machine has a store this build can use.
pub fn available() -> bool {
    #[cfg(windows)]
    {
        config_path().is_some()
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}

/// Whether an entry exists, without exposing anything about it.
///
/// A keychain that cannot be read is reported as "not enrolled": the caller's next move is the
/// master password either way, and a status line is not worth a second authentication prompt.
pub fn enrolled() -> bool {
    matches!(load(), Ok(Some(_)))
}

/// Stores the caller's bytes, replacing any previous entry.
pub fn store(secret: &[u8]) -> Result<(), KeychainError> {
    if secret.len() > MAX_SECRET_BYTES {
        return Err(KeychainError::TooLarge);
    }

    #[cfg(windows)]
    {
        let path = config_path().ok_or(KeychainError::Unavailable)?;
        store_at(&path, secret)
    }
    #[cfg(target_os = "macos")]
    {
        security_framework::passwords::set_generic_password(SERVICE, VAULT_UNLOCK_ACCOUNT, secret)
            .map_err(|error| KeychainError::Failed(format!("the keychain refused the entry: {}", error.code())))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = secret;
        Err(KeychainError::Unavailable)
    }
}

/// Reads what [`store`] wrote, or nothing when no entry exists.
pub fn load() -> Result<Option<Zeroizing<Vec<u8>>>, KeychainError> {
    #[cfg(windows)]
    {
        let path = config_path().ok_or(KeychainError::Unavailable)?;
        load_at(&path)
    }
    #[cfg(target_os = "macos")]
    {
        use security_framework::passwords::{PasswordOptions, generic_password};

        match generic_password(PasswordOptions::new_generic_password(SERVICE, VAULT_UNLOCK_ACCOUNT)) {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(KeychainError::Failed(format!(
                "the keychain could not be read: {}",
                error.code()
            ))),
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err(KeychainError::Unavailable)
    }
}

/// Removes the entry. A missing entry is success: the caller asked for it to be gone.
pub fn delete() -> Result<(), KeychainError> {
    #[cfg(windows)]
    {
        let path = config_path().ok_or(KeychainError::Unavailable)?;
        delete_at(&path)
    }
    #[cfg(target_os = "macos")]
    {
        match security_framework::passwords::delete_generic_password(SERVICE, VAULT_UNLOCK_ACCOUNT) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(KeychainError::Failed(format!(
                "the keychain entry could not be removed: {}",
                error.code()
            ))),
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err(KeychainError::Unavailable)
    }
}

#[cfg(windows)]
fn config_path() -> Option<std::path::PathBuf> {
    super::paths::config_dir().map(|directory| directory.join(SEALED_FILE_NAME))
}

#[cfg(windows)]
fn store_at(path: &std::path::Path, secret: &[u8]) -> Result<(), KeychainError> {
    let sealed = seal(secret)?;
    write_atomically(path, &sealed)
}

#[cfg(windows)]
fn load_at(path: &std::path::Path) -> Result<Option<Zeroizing<Vec<u8>>>, KeychainError> {
    match std::fs::read(path) {
        Ok(sealed) => unseal(&sealed).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(KeychainError::Failed(format!(
            "the sealed key could not be read: {error}"
        ))),
    }
}

#[cfg(windows)]
fn delete_at(path: &std::path::Path) -> Result<(), KeychainError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(KeychainError::Failed(format!(
            "the sealed key could not be removed: {error}"
        ))),
    }
}

#[cfg(windows)]
fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> Result<(), KeychainError> {
    use std::io::Write as _;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            KeychainError::Failed(format!("the configuration directory could not be created: {error}"))
        })?;
    }

    let temporary = path.with_extension("sealed.tmp");
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    };

    if let Err(error) = write() {
        let _ = std::fs::remove_file(&temporary);
        return Err(KeychainError::Failed(format!(
            "the sealed key could not be written: {error}"
        )));
    }

    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        KeychainError::Failed(format!("the sealed key could not be replaced: {error}"))
    })
}

/// Binds the sealed bytes to this application, so another program using DPAPI has to know the
/// entropy as well as run as the same account. Not a secret, and not a guarantee against a process
/// already running as the user — only a boundary around the obvious.
#[cfg(windows)]
const ENTROPY: &[u8] = b"sevenshell/vault-unlock/v1";

#[cfg(windows)]
fn seal(secret: &[u8]) -> Result<Vec<u8>, KeychainError> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
    use windows::core::PCWSTR;

    let mut data = Zeroizing::new(secret.to_vec());
    let mut entropy = ENTROPY.to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len())
            .map_err(|_| KeychainError::Failed("the secret is too large for DPAPI".to_owned()))?,
        pbData: data.as_mut_ptr(),
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(entropy.len())
            .map_err(|_| KeychainError::Failed("the entropy is too large for DPAPI".to_owned()))?,
        pbData: entropy.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    // Safety: both input blobs point at live buffers for the duration of the call; the output blob
    // is freed with `LocalFree`, which is the allocator DPAPI documents.
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            Some(&entropy_blob as *const _),
            None,
            None,
            0x1, // CRYPTPROTECT_UI_FORBIDDEN
            &mut output,
        )
        .map_err(|error| KeychainError::Failed(format!("the unlock key could not be sealed: {error}")))?;

        let sealed = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(sealed)
    }
}

#[cfg(windows)]
fn unseal(sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeychainError> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};

    let mut data = sealed.to_vec();
    let mut entropy = ENTROPY.to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len())
            .map_err(|_| KeychainError::Failed("the sealed key is too large to read".to_owned()))?,
        pbData: data.as_mut_ptr(),
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(entropy.len())
            .map_err(|_| KeychainError::Failed("the entropy is too large for DPAPI".to_owned()))?,
        pbData: entropy.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    // Safety: the same contract as `seal`; the output is freed with the allocator DPAPI used.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            Some(&entropy_blob as *const _),
            None,
            None,
            0x1, // CRYPTPROTECT_UI_FORBIDDEN
            &mut output,
        )
        .map_err(|error| KeychainError::Failed(format!("the unlock key could not be opened: {error}")))?;

        let opened = Zeroizing::new(std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec());
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(opened)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn temporary() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    #[test]
    fn a_sealed_secret_round_trips_without_appearing_on_disk() {
        let directory = temporary();
        let path = directory.path().join(SEALED_FILE_NAME);
        let secret = b"a derived key that must not be readable";

        store_at(&path, secret).expect("the secret is stored");
        let on_disk = std::fs::read(&path).expect("the sealed file exists");
        assert!(
            !on_disk.windows(secret.len()).any(|window| window == secret),
            "the plaintext must not be in the file"
        );

        let loaded = load_at(&path).expect("the sealed file reads").expect("an entry exists");
        assert_eq!(loaded.as_slice(), secret);
    }

    #[test]
    fn storing_again_replaces_the_previous_secret() {
        let directory = temporary();
        let path = directory.path().join(SEALED_FILE_NAME);
        store_at(&path, b"first").expect("the first secret is stored");
        store_at(&path, b"second").expect("the second secret replaces it");

        let loaded = load_at(&path).expect("the sealed file reads").expect("an entry exists");
        assert_eq!(loaded.as_slice(), b"second");
    }

    #[test]
    fn a_missing_entry_is_not_a_failure() {
        let directory = temporary();
        let path = directory.path().join(SEALED_FILE_NAME);
        assert_eq!(load_at(&path).expect("a missing file is an answer"), None);
        delete_at(&path).expect("deleting something that is not there is success");
    }

    #[test]
    fn deleting_removes_the_entry() {
        let directory = temporary();
        let path = directory.path().join(SEALED_FILE_NAME);
        store_at(&path, b"secret").expect("the secret is stored");
        delete_at(&path).expect("the entry is removed");
        assert_eq!(load_at(&path).expect("the file is gone"), None);
    }

    #[test]
    fn a_tampered_file_is_refused_rather_than_returned() {
        let directory = temporary();
        let path = directory.path().join(SEALED_FILE_NAME);
        std::fs::write(&path, b"not a DPAPI blob").expect("a damaged file");
        assert!(load_at(&path).is_err(), "garbage must not be treated as a key");
    }

    #[test]
    fn an_oversized_secret_is_refused_before_it_is_written() {
        let too_large = vec![0u8; MAX_SECRET_BYTES + 1];
        assert_eq!(store(&too_large), Err(KeychainError::TooLarge));
    }
}

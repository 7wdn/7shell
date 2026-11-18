//! The vault's unlock material, handed to the system's own store.
//!
//! The vault file is unchanged: this module only keeps the derived key somewhere the operating
//! system protects for the current account, so a restart does not ask for the master password
//! again. Three rules hold it together:
//!
//! * **Enrollment is opt-in.** Nothing is written to the keychain until the person asks for it, and
//!   a vault that was never enrolled keeps asking for its password.
//! * **A sealed key belongs to one vault.** The record carries the envelope's public salt; a key
//!   enrolled for a vault that was moved aside or replaced is noticed before the cipher is asked to
//!   open anything.
//! * **A stale record falls back, it does not fail.** A key that no longer matches, a keychain that
//!   is locked and a record written by a future build all leave the password form in place. The
//!   caller never sees a keychain error as the reason it cannot open the vault.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use sevenshell_error::ShellError;
use sevenshell_platform::keychain::{self, KeychainError};
use sevenshell_types::{AppSettings, VaultKeychainStatus};
use zeroize::{Zeroize, Zeroizing};

use crate::state::ShellState;

/// The record format version. A future shape re-seals, it does not guess.
const RECORD_VERSION: u32 = 1;

/// What the system store holds: the vault's derived key, bound to the vault it belongs to.
///
/// The key is a base64 copy of the 32 bytes the vault derived. It is wiped when the record is
/// dropped, and the record never implements `Debug` — a key in a panic message is a disclosure.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SealedUnlock {
    version: u32,
    binding: String,
    key: String,
}

impl Drop for SealedUnlock {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl fmt::Debug for SealedUnlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SealedUnlock(<redacted>)")
    }
}

/// Whether the store exists and whether it holds a key for the vault that is in place now.
pub fn status(state: &ShellState) -> VaultKeychainStatus {
    let available = keychain::available();
    VaultKeychainStatus {
        available,
        // Only an open vault is compared. The settings page is the reader of this field, and it
        // lives behind the unlock screen; a locked application asking the keychain a question just
        // to draw a checkbox would be a system prompt for no decision.
        enrolled: available
            && state.vault().status() == sevenshell_vault::VaultStatus::Unlocked
            && enrolled_for_current(state),
    }
}

/// Seals the open vault's derived key. The vault must be unlocked.
pub fn enroll(state: &ShellState) -> Result<VaultKeychainStatus, ShellError> {
    if !keychain::available() {
        return Err(ShellError::vault_keychain_unavailable());
    }

    seal_current(state)?;
    Ok(status(state))
}

/// Removes the entry. Asking again for a password is the whole effect on the open vault.
pub fn forget(state: &ShellState) -> Result<VaultKeychainStatus, ShellError> {
    if !keychain::available() {
        return Err(ShellError::vault_keychain_unavailable());
    }

    keychain::delete().map_err(keychain_error)?;
    Ok(status(state))
}

/// Tries the sealed key once, at application start.
///
/// Reads the entry only when the vault is locked. Each failure — no entry, a different vault, a
/// stale key, a keychain that will not open — is the same quiet answer: the password form is
/// still the way in, and none of those is a reason to refuse to show the window.
pub fn unlock_at_startup(state: &ShellState, settings: &AppSettings) -> bool {
    if !keychain::available() || state.vault().status() != sevenshell_vault::VaultStatus::Locked {
        return false;
    }

    let Some(record) = current_record(state) else {
        return false;
    };
    let Ok(bytes) = BASE64.decode(&record.key) else {
        return false;
    };

    let key = Zeroizing::new(bytes);
    if state.vault().unlock_with_key(key.as_slice()).is_err() {
        return false;
    }

    // The same two things a typed unlock does: the external servers can read their secrets again,
    // and the agent's snapshot can fold in the groups a locked vault could not name.
    state.mcp_client().on_vault_unlocked(settings);
    state.apply_agent_policy(settings);
    true
}

/// Re-seals after an unlock that used the password, so a stale record heals itself.
///
/// Only an entry that already belongs to this vault is refreshed: enrolling a vault nobody asked
/// to enroll would quietly change what opens it next time.
pub fn reseal_if_enrolled(state: &ShellState) {
    if !keychain::available() || !enrolled_for_current(state) {
        return;
    }
    if let Err(error) = seal_current(state) {
        log::warn!("the system keychain could not be refreshed after the vault opened: {error}");
    }
}

/// Re-seals after a password change. The caller checks enrollment before the change, because the
/// old record's binding is what the change is about to replace.
pub fn reseal_after_password_change(state: &ShellState) {
    if let Err(error) = seal_current(state) {
        log::warn!("the system keychain could not be refreshed after the password changed: {error}");
    }
}

/// Whether a record for the current vault exists, without exposing the key.
fn enrolled_for_current(state: &ShellState) -> bool {
    current_record(state).is_some()
}

/// Reads the record and requires it to name the vault that is in place now.
fn current_record(state: &ShellState) -> Option<SealedUnlock> {
    record_for(state.vault())
}

/// The record for one vault, so the startup path and its test share the one reading rule.
fn record_for(vault: &sevenshell_vault::Vault) -> Option<SealedUnlock> {
    let Ok(Some(bytes)) = keychain::load() else {
        return None;
    };
    let Ok(record) = serde_json::from_slice::<SealedUnlock>(&bytes) else {
        return None;
    };
    if record.version != RECORD_VERSION {
        return None;
    }
    let Ok(binding) = vault.binding() else {
        return None;
    };
    record_names_vault(&record, &binding).then_some(record)
}

/// Writes the open vault's key under its current binding.
fn seal_current(state: &ShellState) -> Result<(), ShellError> {
    seal_vault(state.vault())
}

fn seal_vault(vault: &sevenshell_vault::Vault) -> Result<(), ShellError> {
    let key = vault.export_key()?;
    let binding = vault.binding()?;
    let record = SealedUnlock {
        version: RECORD_VERSION,
        binding,
        key: BASE64.encode(key.as_slice()),
    };

    let text = Zeroizing::new(serde_json::to_string(&record).map_err(|error| {
        ShellError::app_internal(format!("the sealed unlock record could not be written: {error}"))
    })?);
    keychain::store(text.as_bytes()).map_err(keychain_error)
}

fn keychain_error(error: KeychainError) -> ShellError {
    match error {
        KeychainError::Unavailable => ShellError::vault_keychain_unavailable(),
        other => ShellError::vault_keychain_failed(other.to_string()),
    }
}

/// Whether a record names this vault. Used by the tests below and by nothing that decides access
/// on its own.
fn record_names_vault(record: &SealedUnlock, binding: &str) -> bool {
    record.version == RECORD_VERSION && record.binding == binding
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(binding: &str, key: &str) -> SealedUnlock {
        SealedUnlock {
            version: RECORD_VERSION,
            binding: binding.to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn a_record_names_exactly_the_vault_it_was_written_for() {
        let sealed = record("salt-one", "a2V5");
        assert!(record_names_vault(&sealed, "salt-one"));
        assert!(!record_names_vault(&sealed, "salt-two"));
    }

    #[test]
    fn a_record_from_a_future_format_is_not_used() {
        let mut sealed = record("salt", "a2V5");
        sealed.version = RECORD_VERSION + 1;
        assert!(!record_names_vault(&sealed, "salt"));
    }

    #[test]
    fn a_sealed_key_does_not_appear_in_the_debug_form() {
        let sealed = record("salt", "c2VjcmV0LWtleQ==");
        let printed = format!("{sealed:?}");
        assert!(!printed.contains("c2VjcmV0"));
        assert!(printed.contains("redacted"));
    }

    #[test]
    fn the_record_round_trips_through_json() {
        let sealed = record("salt", "a2V5");
        let text = serde_json::to_string(&sealed).expect("serialize");
        assert!(text.contains("\"binding\":\"salt\""));
        assert!(text.contains("\"key\":\"a2V5\""));
        let read: SealedUnlock = serde_json::from_str(&text).expect("parse");
        assert!(record_names_vault(&read, "salt"));
    }

    /// The real store, a real vault and the real DPAPI path. Windows only: the macOS test runner
    /// has no UI to answer a keychain prompt, and the Linux build has no store to test.
    #[cfg(windows)]
    mod system_store {
        use super::*;
        use std::ffi::OsString;
        use std::path::Path;
        use std::sync::Mutex;

        use sevenshell_vault::{Vault, VaultStatus};

        /// The environment variable is process-wide; these tests are the only ones that move it.
        static ENV: Mutex<()> = Mutex::new(());

        struct ConfigDir {
            previous: Option<OsString>,
        }

        impl ConfigDir {
            fn at(path: &Path) -> Self {
                let previous = std::env::var_os(sevenshell_platform::paths::CONFIG_DIR_OVERRIDE);
                unsafe { std::env::set_var(sevenshell_platform::paths::CONFIG_DIR_OVERRIDE, path) };
                Self { previous }
            }
        }

        impl Drop for ConfigDir {
            fn drop(&mut self) {
                match self.previous.take() {
                    Some(value) => unsafe { std::env::set_var(sevenshell_platform::paths::CONFIG_DIR_OVERRIDE, value) },
                    None => unsafe { std::env::remove_var(sevenshell_platform::paths::CONFIG_DIR_OVERRIDE) },
                }
            }
        }

        #[test]
        fn a_vault_enrolled_in_the_system_store_opens_without_its_password() {
            let _guard = ENV.lock().expect("the environment lock");
            let directory = tempfile::tempdir().expect("a temporary directory");
            let _config = ConfigDir::at(&directory.path().join("config"));

            let vault = Vault::open_at(directory.path().join("vault.json"));
            vault.create("master").expect("create");
            seal_vault(&vault).expect("the vault is enrolled");

            let record = record_for(&vault).expect("the record names this vault");
            let key = Zeroizing::new(BASE64.decode(&record.key).expect("the record carries a key"));
            vault.lock();
            vault
                .unlock_with_key(key.as_slice())
                .expect("the sealed key opens the vault");
            assert_eq!(vault.status(), VaultStatus::Unlocked);

            keychain::delete().expect("the entry is removed");
            assert!(record_for(&vault).is_none(), "a removed entry is not enrolled");
        }

        #[test]
        fn a_record_written_for_one_vault_does_not_open_another() {
            let _guard = ENV.lock().expect("the environment lock");
            let directory = tempfile::tempdir().expect("a temporary directory");
            let _config = ConfigDir::at(&directory.path().join("config"));

            let enrolled = Vault::open_at(directory.path().join("vault.json"));
            enrolled.create("master").expect("create the first vault");
            seal_vault(&enrolled).expect("the first vault is enrolled");

            let other = Vault::open_at(directory.path().join("other-vault.json"));
            other
                .create("different salt, different vault")
                .expect("create the second vault");
            assert!(
                record_for(&other).is_none(),
                "a sealed key must not be offered to a vault it was not written for"
            );

            keychain::delete().expect("the entry is removed");
        }
    }
}

//! The vault itself: one file, one password, and an explicit locked state.
//!
//! The file is a small JSON envelope holding the KDF parameters, a verifier and the encrypted
//! document:
//!
//! ```json
//! { "version": 1,
//!   "kdf": { "name": "argon2id", "memoryKib": 65536, "iterations": 3, "parallelism": 1 },
//!   "salt": "<base64, 16 bytes>",
//!   "verifier": "<base64 nonce || ciphertext>",
//!   "payload":  "<base64 nonce || ciphertext>" }
//! ```
//!
//! Five rules hold the whole design together:
//!
//! 1. **Nothing is readable before an unlock.** Every document operation answers `VAULT-LOCKED`
//!    while locked, and [`Vault::lock`] drops both the derived key and the decrypted document.
//! 2. **A vault that does not open says so once.** A wrong password, a truncated file, a
//!    flipped byte and a verifier that belongs to another vault all report `VAULT-UNLOCK-FAILED`
//!    with the same code, the same message and no detail, because an error that tells them
//!    apart is an oracle for anyone holding a copy of the file.
//! 3. **Writes are atomic.** Temporary file in the same directory, flush, rename. An
//!    interrupted write leaves the previous vault readable.
//! 4. **Absent means unchanged.** A session saved without a secret keeps the secret it had.
//! 5. **A newer document is refused, not downgraded.** `STORE-SCHEMA-NEWER` names both versions
//!    and `document::migrate` is where the next one gets its step.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::{CommandHistoryEntry, KeyInfo, KnownHostEntry, Session, SessionGroup, Snippet, SnippetGroup};
use zeroize::Zeroizing;

use crate::crypto::{self, KdfParams, VaultKey};
use crate::document::{self, Document, KeyMaterial, SessionSecret};

/// The vault file name inside the platform configuration directory.
pub const VAULT_FILE_NAME: &str = "vault.json";

/// The envelope version this build writes. Distinct from the document's `schemaVersion`: this
/// one covers the file around the document, the document's covers the document itself.
const VAULT_VERSION: u32 = 1;

/// What is actually on disk.
#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u32,
    kdf: KdfParams,
    salt: String,
    verifier: String,
    payload: String,
}

/// An envelope as an earlier build of this application wrote it.
///
/// The cryptography never changed — the same KDF parameters, the same associated data strings, the
/// same nonce layout — so the two layouts differ only in where two fields sit: the salt lived
/// inside the `kdf` object, and the algorithm was called `algorithm`. Reading that file matters,
/// because the alternative for anyone holding one is to throw it away: a vault that cannot be
/// opened cannot be created over either.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEnvelope {
    #[serde(default)]
    version: u32,
    kdf: RawKdf,
    #[serde(default)]
    salt: Option<String>,
    verifier: String,
    payload: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawKdf {
    /// `algorithm` is what the earlier layout called it.
    #[serde(default, alias = "algorithm")]
    name: String,
    #[serde(default)]
    memory_kib: u32,
    #[serde(default)]
    iterations: u32,
    #[serde(default)]
    parallelism: u32,
    #[serde(default)]
    salt: Option<String>,
}

/// Reads either envelope layout into the current one.
fn envelope_from(raw: &[u8]) -> Result<Envelope, serde_json::Error> {
    let raw: RawEnvelope = serde_json::from_slice(raw)?;
    let salt = raw
        .salt
        .or(raw.kdf.salt)
        .ok_or_else(|| serde::de::Error::missing_field("salt"))?;

    Ok(Envelope {
        version: raw.version,
        kdf: KdfParams {
            name: raw.kdf.name,
            memory_kib: raw.kdf.memory_kib,
            iterations: raw.kdf.iterations,
            parallelism: raw.kdf.parallelism,
        },
        salt,
        verifier: raw.verifier,
        payload: raw.payload,
    })
}

/// Whether a vault can be read right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultStatus {
    /// No vault file exists yet: this is a fresh installation, and `create` is the next step.
    Missing,
    /// A vault file exists, but its key and document are not in memory.
    Locked,
    /// The key and the document are in memory; document operations work.
    Unlocked,
}

/// A vault at an explicit path.
///
/// Constructing one reads nothing: [`Vault::open_at`] only records where the file would be, so
/// a caller can ask for its [`Vault::status`] or create it without touching the disk first.
/// All state sits behind a mutex, so the same value can be shared by the desktop shell's
/// commands, each of which takes `&Vault`.
pub struct Vault {
    path: PathBuf,
    slot: Mutex<Slot>,
}

/// What the vault holds while it is open. `Locked` is the absence of a key and a document, not
/// a flag on them: dropping the variant is what erases both. The open state is boxed because it
/// is large — a document, a KDF record and a key — and every locked vault would otherwise carry
/// that much empty space around.
enum Slot {
    Locked,
    Unlocked(Box<Unlocked>),
}

/// The open state: the envelope's parameters, so the same salt and format are reused on every
/// save, the derived key, and the decrypted document.
struct Unlocked {
    kdf: KdfParams,
    salt: String,
    verifier: String,
    key: VaultKey,
    document: Document,
}

impl Unlocked {
    fn write_to(&self, path: &Path) -> Result<(), ShellError> {
        Self::seal(path, &self.kdf, &self.salt, &self.verifier, &self.key, &self.document)
    }

    /// Serializes the document, re-encrypts it and replaces the file atomically.
    ///
    /// The serialized document is the only buffer in this function that holds secrets in the
    /// clear; it is wrapped so that it is wiped when it goes out of scope, and it is never
    /// formatted anywhere.
    fn seal(
        path: &Path,
        kdf: &KdfParams,
        salt: &str,
        verifier: &str,
        key: &VaultKey,
        document: &Document,
    ) -> Result<(), ShellError> {
        let payload = Zeroizing::new(serde_json::to_vec(document).map_err(|error| {
            ShellError::app_internal(format!("the vault document could not be serialized: {error}"))
        })?);

        let envelope = Envelope {
            version: VAULT_VERSION,
            kdf: kdf.clone(),
            salt: salt.to_owned(),
            verifier: verifier.to_owned(),
            payload: crypto::encrypt(key, payload.as_slice(), crypto::AAD_PAYLOAD)?,
        };

        let bytes = serde_json::to_vec_pretty(&envelope).map_err(|error| {
            ShellError::app_internal(format!("the vault envelope could not be serialized: {error}"))
        })?;

        write_atomically(path, &bytes)
    }
}

impl Slot {
    /// The open state, or `VAULT-LOCKED`.
    fn open(&mut self) -> Result<&mut Unlocked, ShellError> {
        match self {
            Self::Unlocked(state) => Ok(state),
            Self::Locked => Err(ShellError::vault_locked()),
        }
    }
}

impl Vault {
    /// Points a vault at `path` without reading it.
    pub fn open_at(path: PathBuf) -> Self {
        Self {
            path,
            slot: Mutex::new(Slot::Locked),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file exists and whether it is open in this process.
    pub fn status(&self) -> VaultStatus {
        if matches!(&*self.slot(), Slot::Unlocked(_)) {
            return VaultStatus::Unlocked;
        }
        if self.path.is_file() {
            VaultStatus::Locked
        } else {
            VaultStatus::Missing
        }
    }

    /// Creates a new vault and leaves it unlocked.
    ///
    /// An existing file is never replaced, even by a caller who meant to re-initialise: the
    /// cost of refusing a mistake is an error message, and the cost of accepting one is every
    /// saved credential at once.
    pub fn create(&self, password: &str) -> Result<(), ShellError> {
        if self.path.exists() {
            return Err(ShellError::store_io(format!(
                "a vault already exists at {}; create never replaces one",
                self.path.display()
            )));
        }

        let kdf = KdfParams::default();
        let salt = crypto::random_salt()?;
        let key = crypto::derive_key(password, &salt, &kdf)?;
        let verifier = crypto::encrypt_verifier(&key)?;

        let state = Unlocked {
            kdf,
            salt,
            verifier,
            key,
            document: Document::empty(),
        };
        state.write_to(&self.path)?;

        *self.slot() = Slot::Unlocked(Box::new(state));
        Ok(())
    }

    /// Opens the vault and leaves it unlocked until [`Vault::lock`].
    pub fn unlock(&self, password: &str) -> Result<(), ShellError> {
        let raw = match std::fs::read(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ShellError::vault_not_found());
            }
            Err(error) => {
                return Err(ShellError::store_io(format!(
                    "cannot read {}: {error}",
                    self.path.display()
                )));
            }
        };

        // A file that is not even an envelope is reported exactly like a wrong password: from
        // the caller's side both mean "this did not open", and that is all they may know.
        let envelope: Envelope = match envelope_from(&raw) {
            Ok(envelope) => envelope,
            Err(_) => return Err(ShellError::vault_unlock_failed()),
        };

        // The envelope version is plaintext, so refusing a newer one says nothing a reader of
        // the file did not already know — and it is the one failure a user can act on.
        if envelope.version > VAULT_VERSION {
            return Err(
                ShellError::store_schema_newer(envelope.version, VAULT_VERSION).with_detail(format!(
                    "{} was written by a newer version of 7Shell",
                    self.path.display()
                )),
            );
        }

        let key = crypto::derive_key(password, &envelope.salt, &envelope.kdf)?;
        crypto::decrypt_verifier(&key, &envelope.verifier)?;

        let payload = Zeroizing::new(crypto::decrypt(&key, &envelope.payload, crypto::AAD_PAYLOAD)?);
        let document = document::from_slice(payload.as_slice())?;

        *self.slot() = Slot::Unlocked(Box::new(Unlocked {
            kdf: envelope.kdf,
            salt: envelope.salt,
            verifier: envelope.verifier,
            key,
            document,
        }));
        Ok(())
    }

    /// Drops the derived key and the decrypted document.
    ///
    /// Dropping is the erasure: the key is a zeroizing buffer and every secret in the document
    /// is wrapped the same way, so nothing is left for the allocator to hand out.
    pub fn lock(&self) {
        *self.slot() = Slot::Locked;
    }

    /// Moves the vault file aside and reports the name it was given.
    ///
    /// This exists for one situation, and it is a real one: a vault exists, nobody remembers its
    /// password, and the alternative is a person staring at an unlock screen with no way forward.
    /// Nothing is deleted — the file is renamed beside itself — so the way back is a rename, and
    /// the answer is the file's *name*, never the directory it lives in.
    ///
    /// Afterwards the vault is [`VaultStatus::Missing`], so the caller can offer to create one.
    pub fn set_aside(&self) -> Result<String, ShellError> {
        if matches!(&*self.slot(), Slot::Unlocked(_)) {
            return Err(ShellError::store_io("lock the vault before setting it aside"));
        }

        let Some(file_name) = self.path.file_name().map(|name| name.to_string_lossy().into_owned()) else {
            return Err(ShellError::store_io("the vault file has no name to rename"));
        };

        for attempt in 1..=99 {
            let candidate = if attempt == 1 {
                format!("{file_name}.old")
            } else {
                format!("{file_name}.{attempt}.old")
            };
            let target = self.path.with_file_name(&candidate);
            if target.exists() {
                continue;
            }

            std::fs::rename(&self.path, &target)
                .map_err(|error| ShellError::store_io(format!("the vault file could not be moved aside: {error}")))?;
            return Ok(candidate);
        }

        Err(ShellError::store_io(
            "there are already too many previous vault files beside this one",
        ))
    }

    /// Re-encrypts the document under a new password.
    ///
    /// The current password has to be given again even though the vault is unlocked: an
    /// unlocked window is not by itself permission to change what opens the vault. The new
    /// envelope is written before the running state adopts it, so a failed write leaves a
    /// vault that still opens with the old password.
    pub fn change_password(&self, current: &str, next: &str) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        let current_key = crypto::derive_key(current, &state.salt, &state.kdf)?;
        crypto::decrypt_verifier(&current_key, &state.verifier)?;

        let salt = crypto::random_salt()?;
        let key = crypto::derive_key(next, &salt, &state.kdf)?;
        let verifier = crypto::encrypt_verifier(&key)?;

        Unlocked::seal(&self.path, &state.kdf, &salt, &verifier, &key, &state.document)?;

        state.salt = salt;
        state.verifier = verifier;
        state.key = key;
        Ok(())
    }

    pub fn sessions(&self) -> Result<Vec<Session>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.sessions())
    }

    /// Adds or replaces a session, and its secret when one is given.
    ///
    /// `secret: None` keeps whatever secret the session already had; there is deliberately no
    /// way to say "remove the secret" here, because the interface cannot express the
    /// difference and keeping a password is the reading that cannot lose data.
    pub fn save_session(&self, session: Session, secret: Option<SessionSecret>) -> Result<Session, ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        let saved = state.document.save_session(session, secret);
        state.write_to(&self.path)?;
        Ok(saved)
    }

    pub fn delete_session(&self, id: &str) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.delete_session(id);
        state.write_to(&self.path)
    }

    pub fn groups(&self) -> Result<Vec<SessionGroup>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.groups.clone())
    }

    pub fn save_group(&self, group: SessionGroup) -> Result<SessionGroup, ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        let saved = state.document.save_group(group);
        state.write_to(&self.path)?;
        Ok(saved)
    }

    /// Removes a group. What was inside it survives, at the root of the tree.
    pub fn delete_group(&self, id: &str) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.delete_group(id);
        state.write_to(&self.path)
    }

    pub fn known_hosts(&self) -> Result<Vec<KnownHostEntry>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.known_hosts.clone())
    }

    pub fn find_known_host(&self, host: &str, port: u16) -> Result<Option<KnownHostEntry>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.find_known_host(host, port))
    }

    /// Records a trusted key for an endpoint, replacing any earlier one for it.
    pub fn trust_host(&self, entry: KnownHostEntry) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.trust_host(entry);
        state.write_to(&self.path)
    }

    pub fn forget_host(&self, host: &str, port: u16) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.forget_host(host, port);
        state.write_to(&self.path)
    }

    /// The saved commands, grouped, in the order the interface lists them.
    ///
    /// Kept in the vault rather than in a settings file: a saved command is a line somebody runs on
    /// servers, and those lines carry host names, paths and sometimes passwords. Nothing about them
    /// belongs in plaintext on disk.
    pub fn snippet_groups(&self) -> Result<Vec<SnippetGroup>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.snippet_groups())
    }

    /// Adds or renames a group. A group with no id is a new one.
    pub fn save_snippet_group(&self, group: SnippetGroup) -> Result<SnippetGroup, ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        let saved = state.document.save_snippet_group(group);
        state.write_to(&self.path)?;
        Ok(saved)
    }

    /// Removes a group and the commands in it. The default group cannot be removed.
    pub fn delete_snippet_group(&self, id: &str) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.delete_snippet_group(id)?;
        state.write_to(&self.path)
    }

    /// Adds or updates a saved command. A command with no id is a new one.
    pub fn save_snippet(&self, snippet: Snippet) -> Result<Snippet, ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        let saved = state.document.save_snippet(snippet);
        state.write_to(&self.path)?;
        Ok(saved)
    }

    pub fn delete_snippet(&self, id: &str) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.delete_snippet(id);
        state.write_to(&self.path)
    }

    /// What was sent, newest first.
    pub fn command_history(&self) -> Result<Vec<CommandHistoryEntry>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.command_history())
    }

    /// Remembers a command that was sent, at the time it was sent.
    pub fn remember_command(&self, command: &str, at: u64) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.remember_command(command, at);
        state.write_to(&self.path)
    }

    pub fn clear_command_history(&self) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.clear_command_history();
        state.write_to(&self.path)
    }

    /// Stores key material and returns the `KeyInfo` to show for it, with the handle the vault
    /// assigned and the truth about whether a passphrase is held.
    pub fn import_key(
        &self,
        contents: String,
        passphrase: Option<String>,
        info: KeyInfo,
    ) -> Result<KeyInfo, ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        let imported = state.document.import_key(contents, passphrase, info)?;
        state.write_to(&self.path)?;
        Ok(imported)
    }

    /// The imported keys, with their fingerprints and nothing they were imported with.
    pub fn keys(&self) -> Result<Vec<KeyInfo>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.keys())
    }

    pub fn key_material(&self, key_ref: &str) -> Result<Option<KeyMaterial>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.key_material(key_ref))
    }

    pub fn forget_key(&self, key_ref: &str) -> Result<(), ShellError> {
        let mut guard = self.slot();
        let state = guard.open()?;

        state.document.forget_key(key_ref);
        state.write_to(&self.path)
    }

    pub fn secret_for_session(&self, id: &str) -> Result<Option<SessionSecret>, ShellError> {
        let mut guard = self.slot();
        Ok(guard.open()?.document.secret_for_session(id))
    }

    /// The only accessor for the state slot.
    ///
    /// A poisoned lock is recovered rather than reported: the mutation it interrupted was a
    /// bug, but the vault is still the best copy of the user's data and refusing to read it
    /// would turn one panic into a lost vault.
    fn slot(&self) -> MutexGuard<'_, Slot> {
        match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("path", &self.path)
            .field("status", &self.status())
            .finish()
    }
}

/// Replaces `path` atomically: a temporary file in the same directory, flushed, then renamed.
///
/// An interrupted write leaves the previous vault readable; the temporary file is removed on
/// every failure path so the next write is not tripped up by a half-written remnant.
fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), ShellError> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ShellError::store_io(format!("cannot create {}: {error}", parent.display())))?;
    }

    let temporary = path.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        Ok(())
    };

    if let Err(error) = write() {
        let _ = std::fs::remove_file(&temporary);
        return Err(ShellError::store_io(format!(
            "cannot write {}: {error}",
            temporary.display()
        )));
    }

    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        ShellError::store_io(format!("cannot replace {}: {error}", path.display()))
    })
}

/// The vault of the running application: one `Vault` at the platform configuration directory,
/// created on first use and shared by every caller.
static FACADE: Mutex<Option<Vault>> = Mutex::new(None);

/// Where the running application's vault lives: `vault.json` beside the non-secret settings.
pub fn default_path() -> Option<PathBuf> {
    sevenshell_platform::config_dir().map(|directory| directory.join(VAULT_FILE_NAME))
}

/// Runs `action` against the application vault, opening the slot on first use.
///
/// The lock is held for the length of the action: there is one vault, and two threads changing
/// it at once is not a state worth supporting.
fn with_vault<T>(action: impl FnOnce(&Vault) -> Result<T, ShellError>) -> Result<T, ShellError> {
    let mut facade = facade();

    if let Some(vault) = facade.as_ref() {
        return action(vault);
    }

    let path = default_path()
        .ok_or_else(|| ShellError::store_io("the configuration directory could not be located on this system"))?;
    let vault = Vault::open_at(path);
    let outcome = action(&vault);
    *facade = Some(vault);
    outcome
}

fn facade() -> MutexGuard<'static, Option<Vault>> {
    match FACADE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Whether the application vault can be read right now.
///
/// A machine with no configuration directory has nowhere to keep a vault, so "missing" is the
/// honest answer there rather than an error.
pub fn status() -> VaultStatus {
    if default_path().is_none() {
        return VaultStatus::Missing;
    }

    match with_vault(|vault| Ok(vault.status())) {
        Ok(status) => status,
        // The slot exists but cannot be reached; locked is the only safe thing to promise.
        Err(_) => VaultStatus::Locked,
    }
}

/// Creates the application vault. See [`Vault::create`].
pub fn create(password: &str) -> Result<(), ShellError> {
    with_vault(|vault| vault.create(password))
}

/// Opens the application vault. See [`Vault::unlock`].
pub fn unlock(password: &str) -> Result<(), ShellError> {
    with_vault(|vault| vault.unlock(password))
}

/// Locks the application vault. See [`Vault::lock`].
///
/// There is nothing to report: a slot that cannot be reached is already locked.
pub fn lock() {
    let _ = with_vault(|vault| {
        vault.lock();
        Ok(())
    });
}

/// Re-encrypts the application vault under a new password. See [`Vault::change_password`].
pub fn change_password(current: &str, next: &str) -> Result<(), ShellError> {
    with_vault(|vault| vault.change_password(current, next))
}

/// The saved sessions of the application vault. See [`Vault::sessions`].
pub fn sessions() -> Result<Vec<Session>, ShellError> {
    with_vault(|vault| vault.sessions())
}

/// Adds or replaces a saved session. See [`Vault::save_session`].
pub fn save_session(session: Session, secret: Option<SessionSecret>) -> Result<Session, ShellError> {
    with_vault(|vault| vault.save_session(session, secret))
}

/// Removes a saved session and its secret. See [`Vault::delete_session`].
pub fn delete_session(id: &str) -> Result<(), ShellError> {
    with_vault(|vault| vault.delete_session(id))
}

/// The session groups of the application vault. See [`Vault::groups`].
pub fn groups() -> Result<Vec<SessionGroup>, ShellError> {
    with_vault(|vault| vault.groups())
}

/// Adds or replaces a group. See [`Vault::save_group`].
pub fn save_group(group: SessionGroup) -> Result<SessionGroup, ShellError> {
    with_vault(|vault| vault.save_group(group))
}

/// Removes a group without removing what was inside it. See [`Vault::delete_group`].
pub fn delete_group(id: &str) -> Result<(), ShellError> {
    with_vault(|vault| vault.delete_group(id))
}

/// The trusted host keys of the application vault. See [`Vault::known_hosts`].
pub fn known_hosts() -> Result<Vec<KnownHostEntry>, ShellError> {
    with_vault(|vault| vault.known_hosts())
}

/// The trusted key for one endpoint. See [`Vault::find_known_host`].
pub fn find_known_host(host: &str, port: u16) -> Result<Option<KnownHostEntry>, ShellError> {
    with_vault(|vault| vault.find_known_host(host, port))
}

/// Records a trusted host key. See [`Vault::trust_host`].
pub fn trust_host(entry: KnownHostEntry) -> Result<(), ShellError> {
    with_vault(|vault| vault.trust_host(entry))
}

/// Forgets the trusted keys of one endpoint. See [`Vault::forget_host`].
pub fn forget_host(host: &str, port: u16) -> Result<(), ShellError> {
    with_vault(|vault| vault.forget_host(host, port))
}

/// Stores key material. See [`Vault::import_key`].
pub fn import_key(contents: String, passphrase: Option<String>, info: KeyInfo) -> Result<KeyInfo, ShellError> {
    with_vault(|vault| vault.import_key(contents, passphrase, info))
}

/// Reads imported key material. See [`Vault::key_material`].
pub fn key_material(key_ref: &str) -> Result<Option<KeyMaterial>, ShellError> {
    with_vault(|vault| vault.key_material(key_ref))
}

/// Removes imported key material. See [`Vault::forget_key`].
pub fn forget_key(key_ref: &str) -> Result<(), ShellError> {
    with_vault(|vault| vault.forget_key(key_ref))
}

/// The secret stored for one session. See [`Vault::secret_for_session`].
pub fn secret_for_session(id: &str) -> Result<Option<SessionSecret>, ShellError> {
    with_vault(|vault| vault.secret_for_session(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_error::ErrorCode;
    use sevenshell_types::{AuthMethod, DEFAULT_GROUP_ID, Protocol, SessionAuth, TerminalSettings};
    use tempfile::tempdir;

    const KEY_CONTENTS: &str =
        "-----BEGIN OPENSSH PRIVATE KEY-----\nnot a real key\n-----END OPENSSH PRIVATE KEY-----\n";

    fn sample_session(id: &str, name: &str) -> Session {
        Session {
            id: id.to_owned(),
            name: name.to_owned(),
            group_id: None,
            protocol: Protocol::Ssh,
            host: "secret.example".to_owned(),
            port: 22,
            user: "root".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 1,
        }
    }

    fn sample_group(id: &str) -> SessionGroup {
        SessionGroup {
            id: id.to_owned(),
            name: "servers".to_owned(),
            parent_id: None,
            order: 0,
        }
    }

    fn sample_host(fingerprint: &str) -> KnownHostEntry {
        KnownHostEntry {
            host: "host.example".to_owned(),
            port: 22,
            key_type: "ssh-ed25519".to_owned(),
            public_key: "AAAAC3NzaC1lZDI1NTE5AAAAI".to_owned(),
            fingerprint: fingerprint.to_owned(),
            added_at: 1,
        }
    }

    fn key_info() -> KeyInfo {
        KeyInfo {
            key_ref: None,
            display_name: "id_ed25519".to_owned(),
            algorithm: "ssh-ed25519".to_owned(),
            fingerprint: "SHA256:abc".to_owned(),
            has_passphrase: false,
        }
    }

    /// The stored secret, as plain text, for assertions that never print it.
    fn stored_secret(vault: &Vault, id: &str) -> String {
        vault
            .secret_for_session(id)
            .expect("secret")
            .expect("stored")
            .expose()
            .to_owned()
    }

    fn envelope_at(raw: &[u8]) -> Envelope {
        serde_json::from_slice(raw).expect("the file is an envelope")
    }

    fn write_envelope(path: &Path, envelope: &Envelope) {
        let bytes = serde_json::to_vec_pretty(envelope).expect("serialize");
        write_atomically(path, &bytes).expect("write");
    }

    /// Rewrites an envelope in the layout an earlier build of this application used: the salt
    /// lived inside the `kdf` object, the algorithm was called `algorithm`, and the record carried
    /// its own version number.
    fn as_earlier_layout(envelope: &Envelope) -> serde_json::Value {
        let mut kdf = serde_json::to_value(&envelope.kdf).expect("serialize kdf");
        let object = kdf.as_object_mut().expect("the kdf record is an object");
        let name = object.remove("name").expect("name");
        object.insert("algorithm".to_owned(), name);
        object.insert("version".to_owned(), serde_json::Value::from(19));
        object.insert("salt".to_owned(), serde_json::Value::from(envelope.salt.clone()));

        serde_json::json!({
            "version": envelope.version,
            "kdf": kdf,
            "verifier": envelope.verifier,
            "payload": envelope.payload,
        })
    }

    #[test]
    fn a_vault_written_by_an_earlier_build_still_opens() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());

        vault.create("master").expect("create");
        vault
            .save_session(
                sample_session("s1", "web-1"),
                Some(SessionSecret::Password("hunter2".to_owned())),
            )
            .expect("save");
        let secret_before = stored_secret(&vault, "s1");

        // Re-shape the file the way the earlier build wrote it, then reopen from scratch.
        let earlier = as_earlier_layout(&envelope_at(&std::fs::read(&path).expect("read")));
        std::fs::write(&path, serde_json::to_vec_pretty(&earlier).expect("serialize")).expect("write an older file");

        let reopened = Vault::open_at(path.clone());
        reopened
            .unlock("master")
            .expect("an earlier vault opens with the same password");

        let sessions = reopened.sessions().expect("sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "web-1");
        assert_eq!(stored_secret(&reopened, "s1"), secret_before, "its secrets survived");

        // The next write lands in the current layout, so the old shape is left behind rather than
        // carried forever.
        reopened
            .save_group(SessionGroup {
                id: "g1".to_owned(),
                name: "work".to_owned(),
                parent_id: None,
                order: 1,
            })
            .expect("save");
        let upgraded = std::fs::read_to_string(&path).expect("read");
        let parsed: serde_json::Value = serde_json::from_str(&upgraded).expect("json");
        assert!(parsed.get("salt").is_some(), "the salt is back at the top level");
        assert_eq!(parsed["kdf"]["name"], serde_json::Value::from("argon2id"));
        assert!(
            parsed["kdf"].get("salt").is_none(),
            "and it is no longer inside the kdf record"
        );
    }

    #[test]
    fn a_vault_with_no_salt_anywhere_does_not_open() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");

        let mut envelope = as_earlier_layout(&envelope_at(&std::fs::read(&path).expect("read")));
        envelope["kdf"].as_object_mut().expect("kdf").remove("salt");
        std::fs::write(&path, serde_json::to_vec_pretty(&envelope).expect("serialize")).expect("write");

        let error = Vault::open_at(path).unlock("master").expect_err("no salt, no key");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::VaultUnlockFailed);
    }

    #[test]
    fn setting_a_vault_aside_keeps_it_and_frees_the_way_forward() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");
        vault.save_session(sample_session("s1", "web-1"), None).expect("save");
        vault.lock();

        let name = vault.set_aside().expect("set aside");

        assert_eq!(name, "vault.json.old");
        assert!(!path.exists(), "the place is free for a new vault");
        assert_eq!(vault.status(), VaultStatus::Missing);
        assert!(
            directory.path().join(&name).exists(),
            "the old file is still there, under its new name"
        );

        // Creating again works, and a second set-aside does not overwrite the first.
        vault.create("a new master").expect("create again");
        vault.lock();
        assert_eq!(vault.set_aside().expect("set aside again"), "vault.json.2.old");
        assert!(
            directory.path().join("vault.json.old").exists(),
            "the first one is untouched"
        );
    }

    #[test]
    fn an_open_vault_refuses_to_be_set_aside() {
        let directory = tempdir().expect("temp dir");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("create");

        let error = vault
            .set_aside()
            .expect_err("an open vault is not moved out from under its commands");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreIo);
    }

    /// One character of a base64 blob, changed to another character of the alphabet.
    fn flip_one_character(blob: &str) -> String {
        let mut bytes = blob.as_bytes().to_vec();
        let middle = bytes.len() / 2;
        bytes[middle] = if bytes[middle] == b'A' { b'B' } else { b'A' };
        String::from_utf8(bytes).expect("base64 is ascii")
    }

    #[test]
    fn a_vault_survives_being_locked_and_unlocked() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());

        assert_eq!(vault.status(), VaultStatus::Missing);
        assert_eq!(vault.path(), path);

        vault.create("master").expect("create");
        assert_eq!(vault.status(), VaultStatus::Unlocked);

        let session = sample_session("s1", "web-1");
        let saved = vault
            .save_session(session.clone(), Some(SessionSecret::Password("hunter2".to_owned())))
            .expect("save");
        assert_eq!(saved.id, session.id);
        assert!(saved.auth.has_secret);

        vault.lock();
        assert_eq!(vault.status(), VaultStatus::Locked);

        // A second value pointed at the same file sees the same vault.
        let reopened = Vault::open_at(path.clone());
        assert_eq!(reopened.status(), VaultStatus::Locked);
        reopened.unlock("master").expect("unlock");
        assert_eq!(reopened.status(), VaultStatus::Unlocked);

        let sessions = reopened.sessions().expect("sessions");
        assert_eq!(sessions, vec![saved]);
    }

    #[test]
    fn a_wrong_password_and_a_tampered_file_report_the_same_failure() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        Vault::open_at(path.clone()).create("master").expect("create");
        let original = std::fs::read(&path).expect("read");

        let wrong_password = Vault::open_at(path.clone())
            .unlock("not-master")
            .expect_err("wrong password");

        // Half a file: the envelope no longer parses at all.
        std::fs::write(&path, &original[..original.len() / 2]).expect("truncate");
        let truncated = Vault::open_at(path.clone()).unlock("master").expect_err("truncated");

        // One flipped character inside the payload's base64.
        std::fs::write(&path, &original).expect("restore");
        let mut envelope = envelope_at(&original);
        envelope.payload = flip_one_character(&envelope.payload);
        write_envelope(&path, &envelope);
        let flipped = Vault::open_at(path.clone()).unlock("master").expect_err("flipped byte");

        // A verifier that belongs to something else, in a file that is otherwise intact.
        std::fs::write(&path, &original).expect("restore");
        let mut envelope = envelope_at(&original);
        std::mem::swap(&mut envelope.verifier, &mut envelope.payload);
        write_envelope(&path, &envelope);
        let swapped = Vault::open_at(path.clone())
            .unlock("master")
            .expect_err("swapped verifier");

        for error in [&wrong_password, &truncated, &flipped, &swapped] {
            assert_eq!(error.code(), ErrorCode::VaultUnlockFailed);
            assert_eq!(error.message_key(), wrong_password.message_key());
            assert_eq!(error.message_params(), wrong_password.message_params());
            assert_eq!(error.detail(), None, "an unlock failure explains nothing, on purpose");
            assert_eq!(error.to_string(), wrong_password.to_string());
        }
    }

    #[test]
    fn a_secret_round_trips_and_an_absent_secret_means_unchanged() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");

        let session = sample_session("s1", "web-1");
        vault
            .save_session(session.clone(), Some(SessionSecret::Password("first".to_owned())))
            .expect("save");
        assert!(stored_secret(&vault, "s1") == "first", "the secret did not round-trip");

        // Saving without a secret keeps the one already stored.
        let mut renamed = session.clone();
        renamed.name = "web-1 renamed".to_owned();
        vault.save_session(renamed.clone(), None).expect("save");
        assert!(
            stored_secret(&vault, "s1") == "first",
            "an absent secret changed the stored one"
        );
        assert!(vault.sessions().expect("sessions")[0].auth.has_secret);

        // Saving with one replaces it.
        vault
            .save_session(renamed.clone(), Some(SessionSecret::KeyPassphrase("second".to_owned())))
            .expect("save");
        assert!(stored_secret(&vault, "s1") == "second", "the secret was not replaced");
        assert!(
            matches!(
                vault.secret_for_session("s1").expect("secret"),
                Some(SessionSecret::Password(_))
            ),
            "a password session reports a password"
        );

        // And it is still there after the vault has been closed and opened again.
        vault.lock();
        vault.unlock("master").expect("unlock");
        assert!(
            stored_secret(&vault, "s1") == "second",
            "the secret did not survive a reopen"
        );
    }

    #[test]
    fn deleting_a_session_removes_it_and_its_secret() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");

        vault
            .save_session(
                sample_session("s1", "web-1"),
                Some(SessionSecret::Password("one".to_owned())),
            )
            .expect("save");
        vault
            .save_session(
                sample_session("s2", "web-2"),
                Some(SessionSecret::Password("two".to_owned())),
            )
            .expect("save");

        vault.delete_session("s1").expect("delete");

        let sessions = vault.sessions().expect("sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s2");
        assert!(vault.secret_for_session("s1").expect("secret").is_none());
        assert!(vault.secret_for_session("s2").expect("secret").is_some());

        // Deleting what is already gone asks for a state that already holds.
        vault.delete_session("s1").expect("delete again");
    }

    #[test]
    fn imported_key_material_comes_back_and_can_be_forgotten() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");

        let imported = vault
            .import_key(KEY_CONTENTS.to_owned(), Some("passphrase".to_owned()), key_info())
            .expect("import");
        let key_ref = imported.key_ref.clone().expect("a handle was assigned");
        assert!(imported.has_passphrase);
        assert_eq!(imported.fingerprint, "SHA256:abc");

        let material = vault.key_material(&key_ref).expect("read").expect("stored");
        assert!(material.contents == KEY_CONTENTS, "the key material did not round-trip");
        assert!(
            material.passphrase.as_deref() == Some("passphrase"),
            "the passphrase did not round-trip"
        );

        vault.lock();
        vault.unlock("master").expect("unlock");
        assert!(vault.key_material(&key_ref).expect("read").is_some());

        vault.forget_key(&key_ref).expect("forget");
        assert!(vault.key_material(&key_ref).expect("read").is_none());
    }

    #[test]
    fn an_empty_key_is_refused() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path);
        vault.create("master").expect("create");

        let error = vault
            .import_key("   \n".to_owned(), None, key_info())
            .expect_err("an empty key is not a key");

        assert_eq!(error.code(), ErrorCode::SshKeyInvalid);
    }

    #[test]
    fn trusted_hosts_match_on_host_and_port() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path);
        vault.create("master").expect("create");

        vault.trust_host(sample_host("SHA256:one")).expect("trust");

        let found = vault
            .find_known_host("host.example", 22)
            .expect("find")
            .expect("trusted");
        assert_eq!(found.fingerprint, "SHA256:one");
        assert!(vault.find_known_host("host.example", 2222).expect("find").is_none());
        assert!(vault.find_known_host("other.example", 22).expect("find").is_none());

        // A rotated key replaces the old one rather than adding a second entry.
        vault.trust_host(sample_host("SHA256:two")).expect("rotate");
        assert_eq!(vault.known_hosts().expect("known hosts").len(), 1);
        assert_eq!(
            vault
                .find_known_host("host.example", 22)
                .expect("find")
                .expect("trusted")
                .fingerprint,
            "SHA256:two"
        );

        vault.forget_host("host.example", 22).expect("forget");
        assert!(vault.known_hosts().expect("known hosts").is_empty());
        assert!(vault.find_known_host("host.example", 22).expect("find").is_none());
    }

    #[test]
    fn a_document_from_a_newer_schema_is_refused() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        Vault::open_at(path.clone()).create("master").expect("create");

        // Rewrite the payload the way a build from the future would: same KDF, newer document.
        // One past this build's shape, not a number written down — see the sibling test in
        // `document.rs` for why.
        let found = document::VAULT_SCHEMA_VERSION + 1;
        let original = std::fs::read(&path).expect("read");
        let mut envelope = envelope_at(&original);
        let key = crypto::derive_key("master", &envelope.salt, &envelope.kdf).expect("derive");
        let payload = format!(r#"{{"schemaVersion":{found},"sessions":[]}}"#);
        envelope.payload = crypto::encrypt(&key, payload.as_bytes(), crypto::AAD_PAYLOAD).expect("seal");
        write_envelope(&path, &envelope);

        let error = Vault::open_at(path.clone())
            .unlock("master")
            .expect_err("a newer document");

        assert_eq!(error.code(), ErrorCode::StoreSchemaNewer);
        assert_eq!(
            error.message_params().get("found").map(String::as_str),
            Some(found.to_string().as_str())
        );
        assert_eq!(
            error.message_params().get("supported").map(String::as_str),
            Some(document::VAULT_SCHEMA_VERSION.to_string().as_str())
        );

        // A newer envelope never gets as far as derivation, let alone the document.
        let mut envelope = envelope_at(&original);
        envelope.version = 2;
        write_envelope(&path, &envelope);

        let error = Vault::open_at(path).unlock("master").expect_err("envelope version 2");

        assert_eq!(error.code(), ErrorCode::StoreSchemaNewer);
        assert_eq!(error.message_params().get("found").map(String::as_str), Some("2"));
    }

    #[test]
    fn every_document_operation_reports_locked_while_locked() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path);
        vault.create("master").expect("create");
        vault.lock();

        let locked = |error: ShellError| assert_eq!(error.code(), ErrorCode::VaultLocked, "{error}");

        locked(vault.sessions().expect_err("sessions"));
        locked(
            vault
                .save_session(sample_session("s1", "web-1"), None)
                .expect_err("save"),
        );
        locked(vault.delete_session("s1").expect_err("delete"));
        locked(vault.groups().expect_err("groups"));
        locked(vault.save_group(sample_group("g1")).expect_err("save group"));
        locked(vault.delete_group("g1").expect_err("delete group"));
        locked(vault.known_hosts().expect_err("known hosts"));
        locked(vault.find_known_host("host.example", 22).expect_err("find"));
        locked(vault.trust_host(sample_host("SHA256:one")).expect_err("trust"));
        locked(vault.forget_host("host.example", 22).expect_err("forget host"));
        locked(
            vault
                .import_key(KEY_CONTENTS.to_owned(), None, key_info())
                .expect_err("import"),
        );
        locked(vault.key_material("key-1").expect_err("material"));
        locked(vault.forget_key("key-1").expect_err("forget key"));
        locked(vault.secret_for_session("s1").expect_err("secret"));
        locked(vault.change_password("master", "next").expect_err("change password"));
    }

    #[test]
    fn a_password_change_keeps_the_document_and_retires_the_old_password() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");
        vault
            .save_session(
                sample_session("s1", "web-1"),
                Some(SessionSecret::Password("hunter2".to_owned())),
            )
            .expect("save");
        vault.trust_host(sample_host("SHA256:one")).expect("trust");

        // Holding an unlocked vault is not permission to change what opens it.
        let error = vault
            .change_password("wrong", "next")
            .expect_err("the current password is checked");
        assert_eq!(error.code(), ErrorCode::VaultUnlockFailed);

        vault.change_password("master", "next").expect("change");

        // The running copy keeps what it had, under the new key.
        assert!(
            stored_secret(&vault, "s1") == "hunter2",
            "the secret was lost in the change"
        );
        assert_eq!(vault.sessions().expect("sessions").len(), 1);
        vault.lock();

        let reopened = Vault::open_at(path.clone());
        let error = reopened.unlock("master").expect_err("the old password is gone");
        assert_eq!(error.code(), ErrorCode::VaultUnlockFailed);

        reopened.unlock("next").expect("the new password opens it");
        assert!(
            stored_secret(&reopened, "s1") == "hunter2",
            "the secret did not survive the change"
        );
        assert_eq!(reopened.sessions().expect("sessions").len(), 1);
        assert_eq!(
            reopened
                .find_known_host("host.example", 22)
                .expect("find")
                .expect("trusted")
                .fingerprint,
            "SHA256:one"
        );
    }

    #[test]
    fn the_vault_file_never_contains_a_stored_secret() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");
        vault
            .save_session(
                sample_session("s1", "web-1"),
                Some(SessionSecret::Password("recognisable-password".to_owned())),
            )
            .expect("save");
        vault
            .import_key(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nrecognisable-key-material\n-----END OPENSSH PRIVATE KEY-----\n"
                    .to_owned(),
                Some("recognisable-passphrase".to_owned()),
                key_info(),
            )
            .expect("import");

        let raw = String::from_utf8_lossy(&std::fs::read(&path).expect("read")).into_owned();

        for leaked in [
            "recognisable-password",
            "recognisable-passphrase",
            "recognisable-key-material",
            "secret.example",
            "web-1",
        ] {
            assert!(!raw.contains(leaked), "the vault file contains {leaked}");
        }
    }

    #[test]
    fn a_failed_write_leaves_the_previous_vault_readable() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");
        vault.save_session(sample_session("s1", "web-1"), None).expect("save");

        // Block the temporary file the next write needs: a directory cannot be created as a
        // file, so the write fails on its first step, before anything is replaced.
        std::fs::create_dir(path.with_extension("json.tmp")).expect("block the temporary path");

        let error = vault
            .save_session(sample_session("s2", "web-2"), None)
            .expect_err("the write must fail");
        assert_eq!(error.code(), ErrorCode::StoreIo);

        let reopened = Vault::open_at(path);
        reopened.unlock("master").expect("the previous vault still opens");
        let sessions = reopened.sessions().expect("sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "web-1");
    }

    #[test]
    fn creating_never_replaces_an_existing_vault() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("vault.json");
        let vault = Vault::open_at(path.clone());
        vault.create("master").expect("create");
        vault.save_session(sample_session("s1", "web-1"), None).expect("save");

        let error = vault
            .create("second")
            .expect_err("an existing vault is never re-initialised");

        assert_eq!(error.code(), ErrorCode::StoreIo);
        let reopened = Vault::open_at(path);
        reopened.unlock("master").expect("the original password still opens it");
        assert_eq!(reopened.sessions().expect("sessions").len(), 1);
    }

    #[test]
    fn a_missing_vault_says_so() {
        let directory = tempdir().expect("temp dir");
        let vault = Vault::open_at(directory.path().join("vault.json"));

        assert_eq!(vault.status(), VaultStatus::Missing);
        assert_eq!(
            vault.unlock("master").expect_err("no file").code(),
            ErrorCode::VaultNotFound
        );
    }

    #[test]
    fn the_application_facade_uses_one_vault() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join(VAULT_FILE_NAME);
        *facade() = Some(Vault::open_at(path.clone()));

        assert_eq!(status(), VaultStatus::Missing);
        create("master").expect("create");
        assert_eq!(status(), VaultStatus::Unlocked);

        save_session(
            sample_session("s1", "web-1"),
            Some(SessionSecret::Password("hunter2".to_owned())),
        )
        .expect("save");
        save_group(sample_group("g1")).expect("save group");
        trust_host(sample_host("SHA256:one")).expect("trust");

        lock();
        assert_eq!(status(), VaultStatus::Locked);
        assert_eq!(sessions().expect_err("locked").code(), ErrorCode::VaultLocked);

        unlock("master").expect("unlock");
        assert_eq!(sessions().expect("sessions").len(), 1);
        assert_eq!(groups().expect("groups").len(), 1);
        assert!(find_known_host("host.example", 22).expect("find").is_some());
        assert!(secret_for_session("s1").expect("secret").is_some());
        delete_session("s1").expect("delete");
        delete_group("g1").expect("delete group");
        forget_host("host.example", 22).expect("forget");
        change_password("master", "next").expect("change password");

        *facade() = None;
    }

    #[test]
    fn the_default_path_is_the_vault_file_beside_the_settings() {
        // A machine with no configuration directory has no default vault path; the check is
        // vacuous there, which is the honest outcome.
        if let Some(path) = default_path() {
            assert_eq!(path.file_name().and_then(|name| name.to_str()), Some(VAULT_FILE_NAME));
            assert_eq!(path.parent(), sevenshell_platform::config_dir().as_deref());
        }
    }

    fn snippet_of(name: &str, command: &str, group_id: &str) -> Snippet {
        Snippet {
            id: String::new(),
            group_id: group_id.to_owned(),
            name: name.to_owned(),
            command: command.to_owned(),
            append_cr: true,
            order: 0,
            slot_history: Vec::new(),
        }
    }

    #[test]
    fn a_saved_command_lands_in_its_group_and_survives_a_lock() {
        let directory = tempdir().expect("temp dir");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("create");

        let group = vault
            .save_snippet_group(SnippetGroup {
                id: String::new(),
                name: "maintenance".to_owned(),
                order: 0,
                commands: Vec::new(),
            })
            .expect("save group");
        assert!(!group.id.is_empty(), "a group with no id is a new one");

        let first = vault
            .save_snippet(snippet_of("uptime", "uptime", &group.id))
            .expect("save");
        let second = vault
            .save_snippet(snippet_of("restart nginx", "systemctl restart #1", &group.id))
            .expect("save");
        assert_eq!((first.order, second.order), (1, 2), "new commands land at the end");

        // Through the file, not just in memory: this is the only test that proves the library is
        // really stored encrypted rather than kept beside the document.
        vault.lock();
        vault.unlock("master").expect("unlock");

        let groups = vault.snippet_groups().expect("read");
        assert_eq!(groups.len(), 2, "the default group is still there");
        let saved = groups.iter().find(|entry| entry.id == group.id).expect("the group");
        assert_eq!(saved.name, "maintenance");
        assert_eq!(
            saved
                .commands
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["uptime", "restart nginx"]
        );
    }

    #[test]
    fn a_command_whose_group_is_gone_lands_in_the_default_group() {
        let directory = tempdir().expect("temp dir");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("create");

        let saved = vault
            .save_snippet(snippet_of("stray", "uptime", "a-group-that-never-existed"))
            .expect("save");

        assert_eq!(saved.group_id, DEFAULT_GROUP_ID);
        let groups = vault.snippet_groups().expect("read");
        let default = groups
            .iter()
            .find(|group| group.id == DEFAULT_GROUP_ID)
            .expect("default");
        assert_eq!(default.commands.len(), 1);
    }

    #[test]
    fn the_key_list_carries_fingerprints_and_nothing_private() {
        let directory = tempdir().expect("temp dir");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("create");

        let imported = vault
            .import_key(
                KEY_CONTENTS.to_owned(),
                Some("the passphrase".to_owned()),
                KeyInfo {
                    key_ref: None,
                    display_name: "id_ed25519".to_owned(),
                    algorithm: "ssh-ed25519".to_owned(),
                    fingerprint: "SHA256:AAAA".to_owned(),
                    has_passphrase: false,
                },
            )
            .expect("import");

        let keys = vault.keys().expect("list");

        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key_ref.as_deref(), imported.key_ref.as_deref());
        assert_eq!(keys[0].display_name, "id_ed25519");
        assert_eq!(keys[0].fingerprint, "SHA256:AAAA");
        assert!(
            keys[0].has_passphrase,
            "the flag reports the passphrase; it is not the passphrase"
        );
    }

    #[test]
    fn the_default_group_cannot_be_deleted() {
        let directory = tempdir().expect("temp dir");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("create");

        let error = vault
            .delete_snippet_group(DEFAULT_GROUP_ID)
            .expect_err("the default group is where commands land");

        assert_eq!(error.code(), ErrorCode::AppUnsupported);
        assert_eq!(vault.snippet_groups().expect("read").len(), 1);
    }

    #[test]
    fn sending_the_same_command_again_moves_it_up_the_history() {
        let directory = tempdir().expect("temp dir");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("create");

        vault.remember_command("uptime", 10).expect("remember");
        vault.remember_command("df -h", 20).expect("remember");
        vault.remember_command("uptime", 30).expect("remember");

        let history = vault.command_history().expect("read");
        assert_eq!(
            history.iter().map(|entry| entry.command.as_str()).collect::<Vec<_>>(),
            ["uptime", "df -h"],
            "a repeat moves to the front instead of adding a second copy"
        );
        assert_eq!(history[0].at, 30, "and it carries the time it was last sent");

        vault.remember_command("   ", 40).expect("blank is not a command");
        assert_eq!(vault.command_history().expect("read").len(), 2);

        vault.clear_command_history().expect("clear");
        assert!(vault.command_history().expect("read").is_empty());
    }

    #[test]
    fn the_history_is_bounded() {
        let directory = tempdir().expect("temp dir");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("create");

        for index in 0..document::COMMAND_HISTORY_LIMIT + 5 {
            vault
                .remember_command(&format!("command-{index}"), index as u64)
                .expect("remember");
        }

        let history = vault.command_history().expect("read");
        assert_eq!(history.len(), document::COMMAND_HISTORY_LIMIT);
        assert_eq!(
            history[0].command,
            format!("command-{}", document::COMMAND_HISTORY_LIMIT + 4)
        );
    }
}

//! The encrypted store: sessions, groups, imported keys, per-session secrets and trusted host
//! keys, behind one password.
//!
//! This crate is the only place in 7Shell where a secret is allowed to rest. It exists so that
//! everything above it — the command layer, the terminal, the connection — can work without
//! ever holding a password longer than one call: the vault is opened once, and what travels out
//! of it is what the caller asked for and nothing else.
//!
//! # How it is used
//!
//! A host keeps a [`Vault`] at an explicit path when it has one to keep — a command line with a
//! `--vault` flag, a test — and the desktop shell uses the free functions instead, which share
//! one vault at the platform configuration directory:
//!
//! ```text
//! use sevenshell_vault::{Vault, VaultStatus};
//!
//! let vault = Vault::open_at(std::path::PathBuf::from("example-vault.json"));
//! assert_eq!(vault.status(), VaultStatus::Missing);
//! ```
//!
//! That block is an illustration, not a doctest: a doctest in this workspace fails to link on
//! Windows (see "Doctests on Windows" in `docs/architecture.md`). The same usage runs as an
//! ordinary test inside this crate instead of being claimed as verified here.
//!
//! # The rules this crate is held to
//!
//! * **No secret leaves the process.** Nothing that holds one implements `Debug` through a
//!   derive, the two values that do carry one ([`KeyMaterial`] and [`SessionSecret`]) wipe
//!   themselves on drop, and a stored secret is never formatted into an error, a log line or a
//!   panic message.
//! * **A vault that does not open says so once.** A wrong password, a truncated file, a flipped
//!   byte and a verifier from another vault all produce `VAULT-UNLOCK-FAILED` with no detail,
//!   because an error that distinguishes them is an oracle for anyone with a copy of the file.
//! * **Locked means locked.** Every document operation answers `VAULT-LOCKED` until an unlock,
//!   and locking drops the derived key and the decrypted document.
//! * **Writes are atomic.** A temporary file in the same directory, flushed, then renamed; an
//!   interrupted write leaves the previous vault readable.
//! * **Absent means unchanged.** Saving a session with no secret keeps the secret it had, so
//!   an interface never has to send a password back just to leave it alone.
//!
//! The file format (version 1) is documented on [`Vault`]: Argon2id with 64 MiB, three passes
//! and one lane; XChaCha20-Poly1305 with a fresh 24-byte nonce per ciphertext; and two fixed
//! associated-data strings, one for the verifier and one for the payload, which are part of the
//! format rather than an implementation detail — changing them would strand every existing
//! vault, so they change through the migration chain, never by edit.

mod crypto;
mod document;
mod transfer;
mod vault;

pub use crypto::KdfParams;
pub use document::{KeyMaterial, SessionSecret, VAULT_SCHEMA_VERSION};
pub use transfer::{BUNDLE_FORMAT, BUNDLE_VERSION, MAX_BUNDLE_BYTES, SessionBundle};
pub use vault::{
    VAULT_FILE_NAME, Vault, VaultStatus, change_password, create, default_path, delete_group, delete_session,
    find_known_host, forget_host, forget_key, groups, import_key, key_material, known_hosts, lock, save_group,
    save_session, secret_for_session, sessions, status, trust_host, unlock,
};

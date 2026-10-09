//! Backups: what a snapshot would hold, writing one, reading one, and applying one.
//!
//! The rules live in `sevenshell-sync`; this is the layer that owns files. The native dialog runs
//! here and the file is read and written here, so the interface is never handed a path — the same
//! rule the session export follows, and for the same reason: a script in the webview cannot read a
//! backup or plant one.
//!
//! Reading a file and applying it are two calls rather than one, because somebody is owed a look at
//! what is inside before it is written over their library. The file that was chosen waits in
//! [`ShellState`] between them, so the second call re-reads the same file rather than making the
//! person pick it again — and the interface still never learns where it is.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_sync::{Destination, OpenedSnapshot, SyncSnapshot, plan_restore};
use sevenshell_types::{SyncApplySummary, SyncCatalog, SyncContents, SyncInspectOutcome, SyncReceipt, SyncSelection};
use sevenshell_vault::Vault;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::state::ShellState;

/// What a snapshot would hold if everybody asked for everything.
///
/// The ceiling rather than a prediction: a dialog reads numbers out of this for the switches that
/// are on, so somebody ticking "the commands that were sent" learns how many there are before they
/// decide. Counting the logins is the sessions' own `has_secret`, which is the vault's answer about
/// its secrets table — so no value is read out of it to say how many there are.
#[tauri::command]
pub fn sync_available(state: State<'_, ShellState>) -> Result<SyncContents, ShellError> {
    available(state.vault())
}

fn available(vault: &Vault) -> Result<SyncContents, ShellError> {
    let sessions = vault.sessions()?;
    let settings = sevenshell_platform::settings::load()?;

    Ok(SyncContents {
        sessions: sessions.len() as u32,
        groups: vault.groups()?.len() as u32,
        snippets: vault
            .snippet_groups()?
            .iter()
            .map(|group| group.commands.len())
            .sum::<usize>() as u32,
        words: vault.completion_words()?.len() as u32,
        history: vault.command_history()?.len() as u32,
        known_hosts: vault.known_hosts()?.len() as u32,
        settings: sevenshell_sync::PortableSettings::of(&settings).count(),
        secrets: sessions.iter().filter(|session| session.auth.has_secret).count() as u32,
        keys: vault.keys()?.len() as u32,
    })
}

/// Writes a backup where the user says, with what they chose and under the passphrase they gave.
///
/// The passphrase is the whole of what the interface decides about credentials, and the two answers
/// are the rule rather than a preference: given one, the file carries the stored logins and key
/// material sealed; given none, it carries neither. No value of this argument writes a password in
/// the clear.
///
/// Returns `Ok(None)` when the dialog was dismissed, which is not a failure.
#[tauri::command]
pub async fn sync_backup_export(
    app: AppHandle,
    state: State<'_, ShellState>,
    title: String,
    file_name: String,
    selection: SyncSelection,
    passphrase: Option<String>,
) -> Result<Option<SyncReceipt>, ShellError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let settings = sevenshell_platform::settings::load()?;
    let built = sevenshell_sync::build(state.vault(), &settings, &selection, passphrase.as_deref(), now)?;
    let receipt = SyncReceipt {
        contents: built.contents,
        sealed: built.sealed,
        file_name: file_name.clone(),
    };

    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title(title)
            .set_file_name(file_name)
            .add_filter("7Shell backup", &["json"])
            .blocking_save_file()
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("the file dialog could not be opened: {error}")))?;

    let Some(file) = picked else {
        return Ok(None);
    };

    let path = file
        .into_path()
        .map_err(|error| ShellError::sync_io(format!("that file cannot be written to: {error}")))?;
    sevenshell_sync::write(&path, &built.text)?;

    Ok(Some(receipt))
}

/// Reads a backup the user picks and says what is inside it.
///
/// `passphrase` is `None` on the first call, which either reads the file or answers that it is
/// sealed and is now waiting. The second call carries the answer and reads that same file again.
/// A dismissed dialog is `Ok(None)`.
#[tauri::command]
pub async fn sync_backup_inspect(
    app: AppHandle,
    state: State<'_, ShellState>,
    title: String,
    passphrase: Option<String>,
) -> Result<Option<SyncInspectOutcome>, ShellError> {
    let path = match passphrase {
        Some(passphrase) => {
            let Some(path) = state.pending_restore().path() else {
                return Err(ShellError::app_internal(
                    "there is no backup waiting for a passphrase; start the restore again",
                ));
            };
            return inspect_from(path, Some(&passphrase));
        }
        None => pick_file(app, title).await?,
    };

    let Some(path) = path else {
        return Ok(None);
    };

    // Remembered before it is read: whether the file is sealed is only known once it has been
    // parsed, and the answer to that is what decides whether a question follows.
    state.pending_restore().remember(path.clone());
    inspect_from(path, None)
}

/// Applies the backup that was read, with the passphrase it was read under.
///
/// `replace` is the one thing the interface decides about what an arrival means: off, an item whose
/// id is already here arrives as a second copy; on, the file's version is written over it. The
/// default is off — a restore that quietly replaced a library would be the worst kind of surprise.
#[tauri::command]
pub async fn sync_backup_restore(
    state: State<'_, ShellState>,
    replace: bool,
    passphrase: Option<String>,
) -> Result<SyncApplySummary, ShellError> {
    let Some(path) = state.pending_restore().path() else {
        return Err(ShellError::app_internal(
            "there is no backup waiting to be applied; read one first",
        ));
    };
    let outcome = restore_from(&state, path, passphrase.as_deref(), replace);
    state.pending_restore().clear();
    outcome
}

/// Asks the platform for a file, in Rust. `Ok(None)` is somebody closing the dialog.
async fn pick_file(app: AppHandle, title: String) -> Result<Option<PathBuf>, ShellError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title(title)
            .add_filter("7Shell backup", &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("the file dialog could not be opened: {error}")))?;

    match picked {
        Some(file) => file
            .into_path()
            .map(Some)
            .map_err(|error| ShellError::sync_io(format!("that file cannot be read: {error}"))),
        None => Ok(None),
    }
}

/// Reads a chosen file — for a second time, when the first one stopped to ask — and says what it is.
///
/// A file that was read keeps waiting, because reading is not applying: the next call is the one
/// that writes, and it reads the same file again. A file that was *refused* is let go, because
/// nothing will be asked about it and leaving it there would put an unusable path behind the next
/// answer somebody gives.
fn inspect_from(path: PathBuf, passphrase: Option<&str>) -> Result<Option<SyncInspectOutcome>, ShellError> {
    read_for_inspection(path, passphrase)
}

fn read_for_inspection(path: PathBuf, passphrase: Option<&str>) -> Result<Option<SyncInspectOutcome>, ShellError> {
    let file_name = file_name_of(&path);
    let text = sevenshell_sync::read(&path)?;

    match sevenshell_sync::open(&text)? {
        OpenedSnapshot::Plain(snapshot) => Ok(Some(read_outcome(&snapshot, file_name, false))),
        OpenedSnapshot::Sealed(envelope) => match passphrase {
            Some(passphrase) => {
                let snapshot = envelope.unseal(passphrase)?;
                Ok(Some(read_outcome(&snapshot, file_name, true)))
            }
            // Still waiting: a wrong passphrase on the next call needs the same file.
            None => Ok(Some(SyncInspectOutcome::NeedsPassphrase { file_name })),
        },
    }
}

/// The catalog of a file that has been read: counts, and what the interface is allowed to know.
fn read_outcome(snapshot: &SyncSnapshot, file_name: String, sealed: bool) -> SyncInspectOutcome {
    SyncInspectOutcome::Read {
        catalog: SyncCatalog {
            contents: snapshot.contents(),
            sealed,
            exported_at: snapshot.exported_at,
            app_version: snapshot.app_version.clone(),
        },
        file_name,
    }
}

/// Applies a file that has already been read: the vault in one write, and the settings after it.
fn restore_from(
    state: &ShellState,
    path: PathBuf,
    passphrase: Option<&str>,
    replace: bool,
) -> Result<SyncApplySummary, ShellError> {
    let text = sevenshell_sync::read(&path)?;
    let snapshot = match sevenshell_sync::open(&text)? {
        OpenedSnapshot::Plain(snapshot) => *snapshot,
        OpenedSnapshot::Sealed(envelope) => match passphrase {
            Some(passphrase) => envelope.unseal(passphrase)?,
            None => {
                return Err(ShellError::sync_passphrase());
            }
        },
    };

    let destination = Destination::of(state.vault())?;
    let (restore, travelling, summary) = plan_restore(snapshot, &destination, replace).into_parts();

    // Read before writing: a settings file that cannot be read is a refusal, and a refusal that
    // arrives after the library has been replaced is a restore that half happened.
    let settings_to_write = match travelling {
        Some(travelling) => {
            let mut current = sevenshell_platform::settings::load()?;
            travelling.apply_to(&mut current);
            Some(current)
        }
        None => None,
    };

    state.vault().apply_restore(restore)?;
    if let Some(settings) = settings_to_write {
        sevenshell_platform::settings::save(&settings)?;
    }

    Ok(summary)
}

fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

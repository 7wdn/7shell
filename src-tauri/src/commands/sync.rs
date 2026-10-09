//! Backups: what a snapshot would hold, writing one, reading one, and applying one — to a file, and
//! to a server.
//!
//! The rules live in `sevenshell-sync`; this is the layer that owns files, the settings and the
//! vault's secrets. The native dialog runs here and the file is read and written here, so the
//! interface is never handed a path — the same rule the session export follows, and for the same
//! reason: a script in the webview cannot read a backup or plant one.
//!
//! Reading a file and applying it are two calls rather than one, because somebody is owed a look at
//! what is inside before it is written over their library. The file that was chosen waits in
//! [`ShellState`] between them, so the second call re-reads the same file rather than making the
//! person pick it again — and the interface still never learns where it is.
//!
//! A **server** needs none of that waiting. Where a file's contents have to be remembered between
//! two calls, a server's can simply be asked again: the second call fetches the same file, which is
//! also the more honest thing to do — what gets applied is what the server holds *now*.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_sync::{
    DEFAULT_REMOTE_PATH, Destination, OpenedSnapshot, SyncService, SyncSnapshot, WebDav, plan_restore,
};
use sevenshell_types::{
    SyncApplySummary, SyncCatalog, SyncContents, SyncDownloadOutcome, SyncInspectOutcome, SyncReceipt, SyncSelection,
    SyncServiceStatus, SyncUploadReceipt,
};
use sevenshell_vault::{ServicePassword, Vault};
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

// -- The server --------------------------------------------------------------

/// What this installation has for a sync server.
///
/// Two answers about what is kept and nothing about what it is: the address and the user name are
/// in the settings the interface already holds, and the password and the passphrase are never sent
/// anywhere but a header and a file.
#[tauri::command]
pub fn sync_status(state: State<'_, ShellState>) -> Result<SyncServiceStatus, ShellError> {
    Ok(SyncServiceStatus {
        has_password: state.vault().sync_password()?.is_some(),
        has_passphrase: state.vault().sync_passphrase()?.is_some(),
    })
}

/// Keeps the server's password, or forgets the one that was kept.
///
/// `None` is forgetting rather than "leave it alone": there is one control for this — a box beside
/// the field — and unticking it has to mean something.
#[tauri::command]
pub fn sync_password(state: State<'_, ShellState>, password: Option<String>) -> Result<SyncServiceStatus, ShellError> {
    state.vault().set_sync_password(password)?;
    sync_status(state)
}

/// Keeps the passphrase an upload is sealed under, or forgets it.
#[tauri::command]
pub fn sync_passphrase(
    state: State<'_, ShellState>,
    passphrase: Option<String>,
) -> Result<SyncServiceStatus, ShellError> {
    state.vault().set_sync_passphrase(passphrase)?;
    sync_status(state)
}

/// Asks the configured server whether it is one.
///
/// The question a person asks before trusting a setup: `PROPFIND` with depth zero is what every
/// WebDAV server answers and what a plain file server refuses. Nothing is written.
#[tauri::command]
pub async fn sync_test(state: State<'_, ShellState>) -> Result<(), ShellError> {
    let dav = client(&state)?;
    dav.probe().await
}

/// Writes the library to the server, with what was chosen and under the passphrase in force.
///
/// The passphrase the caller sends wins over the one that was kept, and an empty string means
/// "nothing was typed" rather than "a passphrase of nothing": the field is cleared after it is
/// saved, and pressing a button with an empty field is the ordinary way to use the kept one.
#[tauri::command]
pub async fn sync_upload(
    state: State<'_, ShellState>,
    selection: SyncSelection,
    passphrase: Option<String>,
) -> Result<SyncUploadReceipt, ShellError> {
    let settings = sevenshell_platform::settings::load()?;
    let sealing = sealing_passphrase(&state, passphrase)?;

    // Two answers decide whether the credentials travel, and both have to say yes: the person
    // ticking the box in the dialog, and the switch on the page saying this installation syncs
    // credentials at all. The passphrase is what can actually seal them, and a build asked for
    // credentials with nothing to seal them under writes a snapshot without any.
    let mut wanted = selection;
    wanted.credentials = wanted.credentials && settings.sync_include_secrets;

    let built = sevenshell_sync::build(
        state.vault(),
        &settings,
        &wanted,
        sealing.as_ref().map(ServicePassword::expose),
        now(),
    )?;

    let dav = client(&state)?;
    dav.write(&built.text).await?;

    Ok(SyncUploadReceipt {
        contents: built.contents,
        sealed: built.sealed,
        remote_path: dav.remote_path().to_owned(),
    })
}

/// Reads the file the server holds, and says what is inside it.
#[tauri::command]
pub async fn sync_download(
    state: State<'_, ShellState>,
    passphrase: Option<String>,
) -> Result<SyncDownloadOutcome, ShellError> {
    let dav = client(&state)?;
    let Some(text) = dav.read().await? else {
        return Ok(SyncDownloadOutcome::NothingYet);
    };

    let sealing = sealing_passphrase(&state, passphrase)?;
    match sevenshell_sync::open(&text)? {
        OpenedSnapshot::Plain(snapshot) => Ok(SyncDownloadOutcome::Read {
            catalog: catalog_of(&snapshot, false),
        }),
        OpenedSnapshot::Sealed(envelope) => match sealing.as_ref().map(ServicePassword::expose) {
            Some(passphrase) => Ok(SyncDownloadOutcome::Read {
                catalog: catalog_of(&envelope.unseal(passphrase)?, true),
            }),
            None => Ok(SyncDownloadOutcome::NeedsPassphrase),
        },
    }
}

/// Applies the file the server holds, fetching it again.
///
/// Read again rather than held between two calls, unlike the file case: the server's copy is the
/// authority, and what gets applied is what is there now. A wrapper that kept the first answer would
/// apply something the server may have replaced in the meantime.
#[tauri::command]
pub async fn sync_apply(
    state: State<'_, ShellState>,
    replace: bool,
    passphrase: Option<String>,
) -> Result<SyncApplySummary, ShellError> {
    let dav = client(&state)?;
    let Some(text) = dav.read().await? else {
        return Err(ShellError::sync_snapshot_invalid("the server has no backup there"));
    };

    let sealing = sealing_passphrase(&state, passphrase)?;
    let snapshot = match sevenshell_sync::open(&text)? {
        OpenedSnapshot::Plain(snapshot) => *snapshot,
        OpenedSnapshot::Sealed(envelope) => match sealing.as_ref().map(ServicePassword::expose) {
            Some(passphrase) => envelope.unseal(passphrase)?,
            None => return Err(ShellError::sync_passphrase()),
        },
    };

    apply(state.vault(), snapshot, replace)
}

// -- Backups in a file -------------------------------------------------------

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
    let now = now();
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

// -- The pieces the commands above are made of -------------------------------

/// The client this installation's settings and vault describe.
///
/// Built here rather than handed in: the configuration is three fields from the settings file and
/// one secret from the vault, and neither of those belongs in a wire type — an address crosses to
/// the interface for the form, but the form is not what dials.
fn client(state: &ShellState) -> Result<WebDav, ShellError> {
    let settings = sevenshell_platform::settings::load()?;

    WebDav::new(&SyncService {
        endpoint: settings.sync_endpoint.clone().unwrap_or_default(),
        username: settings.sync_username.clone(),
        password: state.vault().sync_password()?,
        remote_path: settings
            .sync_remote_path
            .clone()
            .unwrap_or_else(|| DEFAULT_REMOTE_PATH.to_owned()),
    })
}

/// The passphrase an upload is sealed under, or one is opened with.
///
/// The typed one wins over the kept one, and an empty string is "nothing was typed": the field is
/// cleared once it is saved, so pressing a button with it empty is the ordinary way to use what the
/// vault holds.
fn sealing_passphrase(state: &ShellState, typed: Option<String>) -> Result<Option<ServicePassword>, ShellError> {
    if let Some(value) = typed.filter(|value| !value.is_empty()) {
        return Ok(Some(ServicePassword::new(value)));
    }
    state.vault().sync_passphrase()
}

/// What a snapshot holds, in the shape a catalog is.
fn catalog_of(snapshot: &SyncSnapshot, sealed: bool) -> SyncCatalog {
    SyncCatalog {
        contents: snapshot.contents(),
        sealed,
        exported_at: snapshot.exported_at,
        app_version: snapshot.app_version.clone(),
    }
}

/// Writes everything a snapshot holds, in one document write, and the settings after it.
fn apply(vault: &Vault, snapshot: SyncSnapshot, replace: bool) -> Result<SyncApplySummary, ShellError> {
    let destination = Destination::of(vault)?;
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

    vault.apply_restore(restore)?;
    if let Some(settings) = settings_to_write {
        sevenshell_platform::settings::save(&settings)?;
    }

    Ok(summary)
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
        OpenedSnapshot::Plain(snapshot) => Ok(Some(SyncInspectOutcome::Read {
            catalog: catalog_of(&snapshot, false),
            file_name,
        })),
        OpenedSnapshot::Sealed(envelope) => match passphrase {
            Some(passphrase) => {
                let snapshot = envelope.unseal(passphrase)?;
                Ok(Some(SyncInspectOutcome::Read {
                    catalog: catalog_of(&snapshot, true),
                    file_name,
                }))
            }
            // Still waiting: a wrong passphrase on the next call needs the same file.
            None => Ok(Some(SyncInspectOutcome::NeedsPassphrase { file_name })),
        },
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
            None => return Err(ShellError::sync_passphrase()),
        },
    };

    apply(state.vault(), snapshot, replace)
}

fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Seconds since the Unix epoch, for a snapshot's own record of when it was taken.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

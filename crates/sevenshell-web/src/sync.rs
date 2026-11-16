//! Server and snippet backups for the browser host: the same rules as the desktop, without the
//! native file dialogs. Everything here takes the vault it reads and writes, so the caller decides
//! which installation's secrets are in play.

use std::time::{SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_sync::{
    BuiltSnapshot, DEFAULT_REMOTE_PATH, Destination, OpenedSnapshot, SnippetSync, Snippets, SyncService, SyncSnapshot,
    WebDav, plan_restore, snippet_account,
};
use sevenshell_types::{
    AppSettings, SnippetProvider, SnippetStatus, SnippetUploadReceipt, SyncApplySummary, SyncCatalog, SyncContents,
    SyncDownloadOutcome, SyncInspectOutcome, SyncReceipt, SyncSelection, SyncServiceStatus, SyncUploadReceipt,
};
use sevenshell_vault::{ServicePassword, Vault};

pub fn available(vault: &Vault) -> Result<SyncContents, ShellError> {
    let sessions = vault.sessions()?;
    let settings = sevenshell_platform::load()?;

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

pub fn status(vault: &Vault) -> Result<SyncServiceStatus, ShellError> {
    Ok(SyncServiceStatus {
        has_password: vault.sync_password()?.is_some(),
        has_passphrase: vault.sync_passphrase()?.is_some(),
    })
}

pub fn set_password(vault: &Vault, password: Option<String>) -> Result<SyncServiceStatus, ShellError> {
    vault.set_sync_password(password)?;
    status(vault)
}

pub fn set_passphrase(vault: &Vault, passphrase: Option<String>) -> Result<SyncServiceStatus, ShellError> {
    vault.set_sync_passphrase(passphrase)?;
    status(vault)
}

pub async fn test(vault: &Vault) -> Result<(), ShellError> {
    client(vault)?.probe().await
}

pub async fn upload(
    vault: &Vault,
    selection: SyncSelection,
    passphrase: Option<String>,
) -> Result<SyncUploadReceipt, ShellError> {
    let built = built_snapshot(vault, selection, passphrase)?;
    let dav = client(vault)?;
    dav.write(&built.text).await?;

    Ok(SyncUploadReceipt {
        contents: built.contents,
        sealed: built.sealed,
        remote_path: dav.remote_path().to_owned(),
    })
}

pub async fn download(vault: &Vault, passphrase: Option<String>) -> Result<SyncDownloadOutcome, ShellError> {
    let dav = client(vault)?;
    let Some(text) = dav.read().await? else {
        return Ok(SyncDownloadOutcome::NothingYet);
    };
    let sealing = sealing_passphrase(vault, passphrase)?;
    read_backup(&text, sealing.as_ref())
}

pub async fn apply_sync(
    vault: &Vault,
    replace: bool,
    passphrase: Option<String>,
) -> Result<SyncApplySummary, ShellError> {
    let dav = client(vault)?;
    let Some(text) = dav.read().await? else {
        return Err(ShellError::sync_snapshot_invalid("the server has no backup there"));
    };
    let sealing = sealing_passphrase(vault, passphrase)?;
    let snapshot = opened_backup(&text, sealing.as_ref())?;
    apply(vault, snapshot, replace)
}

pub fn snippet_status(vault: &Vault) -> Result<SnippetStatus, ShellError> {
    let settings = sevenshell_platform::load()?;
    let account = configured_account(&settings)?;
    Ok(SnippetStatus {
        has_token: vault.snippet_token(&account)?.is_some(),
        account,
    })
}

pub fn set_snippet_token(vault: &Vault, token: Option<String>) -> Result<SnippetStatus, ShellError> {
    let settings = sevenshell_platform::load()?;
    let account = configured_account(&settings)?;
    vault.set_snippet_token(&account, token)?;
    snippet_status(vault)
}

pub async fn snippet_test(vault: &Vault) -> Result<(), ShellError> {
    let (snippets, _) = snippet_client(vault)?;
    snippets.probe().await
}

pub async fn snippet_upload(
    vault: &Vault,
    selection: SyncSelection,
    passphrase: Option<String>,
) -> Result<SnippetUploadReceipt, ShellError> {
    let settings = sevenshell_platform::load()?;
    let sealing = sealing_passphrase(vault, passphrase)?;
    let Some(sealing) = sealing else {
        return Err(ShellError::sync_passphrase());
    };

    let wanted = wanted_selection(selection, &settings);
    let mut snapshot = sevenshell_sync::gather(vault, &settings, &wanted, Some(sealing.expose()), now())?;
    let contents = snapshot.contents();
    sevenshell_sync::with_credentials(vault, &mut snapshot)?;
    let text = sevenshell_sync::seal(&snapshot, sealing.expose(), &Default::default())?;

    let (snippets, provider) = snippet_client(vault)?;
    let snippet_id = snippets.write(&text).await?;

    Ok(SnippetUploadReceipt {
        contents,
        sealed: true,
        provider,
        snippet_id,
    })
}

pub async fn snippet_download(vault: &Vault, passphrase: Option<String>) -> Result<SyncDownloadOutcome, ShellError> {
    let text = read_snippet(vault).await?;
    let sealing = sealing_passphrase(vault, passphrase)?;
    read_backup(&text, sealing.as_ref())
}

pub async fn snippet_apply(
    vault: &Vault,
    replace: bool,
    passphrase: Option<String>,
) -> Result<SyncApplySummary, ShellError> {
    let text = read_snippet(vault).await?;
    let sealing = sealing_passphrase(vault, passphrase)?;
    let snapshot = opened_backup(&text, sealing.as_ref())?;
    apply(vault, snapshot, replace)
}

fn client(vault: &Vault) -> Result<WebDav, ShellError> {
    let settings = sevenshell_platform::load()?;
    WebDav::new(&SyncService {
        endpoint: settings.sync_endpoint.clone().unwrap_or_default(),
        username: settings.sync_username.clone(),
        password: vault.sync_password()?,
        remote_path: settings
            .sync_remote_path
            .clone()
            .unwrap_or_else(|| DEFAULT_REMOTE_PATH.to_owned()),
    })
}

fn sealing_passphrase(vault: &Vault, typed: Option<String>) -> Result<Option<ServicePassword>, ShellError> {
    if let Some(value) = typed.filter(|value| !value.is_empty()) {
        return Ok(Some(ServicePassword::new(value)));
    }
    vault.sync_passphrase()
}

fn built_snapshot(
    vault: &Vault,
    selection: SyncSelection,
    passphrase: Option<String>,
) -> Result<BuiltSnapshot, ShellError> {
    let settings = sevenshell_platform::load()?;
    let sealing = sealing_passphrase(vault, passphrase)?;
    let wanted = wanted_selection(selection, &settings);
    sevenshell_sync::build(
        vault,
        &settings,
        &wanted,
        sealing.as_ref().map(ServicePassword::expose),
        now(),
    )
}

fn wanted_selection(selection: SyncSelection, settings: &AppSettings) -> SyncSelection {
    let mut wanted = selection;
    wanted.credentials = wanted.credentials && settings.sync_include_secrets;
    wanted
}

fn read_backup(text: &str, sealing: Option<&ServicePassword>) -> Result<SyncDownloadOutcome, ShellError> {
    match sevenshell_sync::open(text)? {
        OpenedSnapshot::Plain(snapshot) => Ok(SyncDownloadOutcome::Read {
            catalog: catalog_of(&snapshot, false),
        }),
        OpenedSnapshot::Sealed(envelope) => match sealing.map(ServicePassword::expose) {
            Some(passphrase) => Ok(SyncDownloadOutcome::Read {
                catalog: catalog_of(&envelope.unseal(passphrase)?, true),
            }),
            None => Ok(SyncDownloadOutcome::NeedsPassphrase),
        },
    }
}

fn opened_backup(text: &str, sealing: Option<&ServicePassword>) -> Result<SyncSnapshot, ShellError> {
    match sevenshell_sync::open(text)? {
        OpenedSnapshot::Plain(snapshot) => Ok(*snapshot),
        OpenedSnapshot::Sealed(envelope) => match sealing.map(ServicePassword::expose) {
            Some(passphrase) => envelope.unseal(passphrase),
            None => Err(ShellError::sync_passphrase()),
        },
    }
}

fn configured_account(settings: &AppSettings) -> Result<String, ShellError> {
    let provider = settings.sync_snippet_provider.unwrap_or_default();
    snippet_account(provider, settings.sync_snippet_instance.as_deref())
}

fn snippet_client(vault: &Vault) -> Result<(Snippets, SnippetProvider), ShellError> {
    let settings = sevenshell_platform::load()?;
    let provider = settings.sync_snippet_provider.unwrap_or_default();
    let instance_url = settings.sync_snippet_instance.clone();
    let account = snippet_account(provider, instance_url.as_deref())?;

    let token = vault
        .snippet_token(&account)?
        .unwrap_or_else(|| ServicePassword::new(String::new()));

    let sync = SnippetSync {
        provider,
        instance_url,
        token,
        snippet_id: settings.sync_snippet_ids.get(&account).cloned(),
    };

    Ok((Snippets::new(&sync)?, provider))
}

async fn read_snippet(vault: &Vault) -> Result<String, ShellError> {
    let (snippets, _) = snippet_client(vault)?;
    snippets.read().await
}

fn catalog_of(snapshot: &SyncSnapshot, sealed: bool) -> SyncCatalog {
    SyncCatalog {
        contents: snapshot.contents(),
        sealed,
        exported_at: snapshot.exported_at,
        app_version: snapshot.app_version.clone(),
    }
}

fn apply(vault: &Vault, snapshot: SyncSnapshot, replace: bool) -> Result<SyncApplySummary, ShellError> {
    let destination = Destination::of(vault)?;
    let (restore, travelling, summary) = plan_restore(snapshot, &destination, replace).into_parts();

    let settings_to_write = match travelling {
        Some(travelling) => {
            let mut current = sevenshell_platform::load()?;
            travelling.apply_to(&mut current);
            Some(current)
        }
        None => None,
    };

    vault.apply_restore(restore)?;
    if let Some(settings) = settings_to_write {
        sevenshell_platform::save(&settings)?;
    }

    Ok(summary)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

pub fn export_text(
    vault: &Vault,
    selection: SyncSelection,
    passphrase: Option<String>,
    file_name: String,
) -> Result<(SyncReceipt, String), ShellError> {
    let settings = sevenshell_platform::load()?;
    let built = sevenshell_sync::build(vault, &settings, &selection, passphrase.as_deref(), now())?;
    let receipt = SyncReceipt {
        contents: built.contents,
        sealed: built.sealed,
        file_name,
    };
    Ok((receipt, built.text))
}

pub fn inspect_text(text: &str, file_name: String, passphrase: Option<&str>) -> Result<SyncInspectOutcome, ShellError> {
    match sevenshell_sync::open(text)? {
        OpenedSnapshot::Plain(snapshot) => Ok(SyncInspectOutcome::Read {
            catalog: catalog_of(&snapshot, false),
            file_name,
        }),
        OpenedSnapshot::Sealed(envelope) => match passphrase {
            Some(passphrase) => {
                let snapshot = envelope.unseal(passphrase)?;
                Ok(SyncInspectOutcome::Read {
                    catalog: catalog_of(&snapshot, true),
                    file_name,
                })
            }
            None => Ok(SyncInspectOutcome::NeedsPassphrase { file_name }),
        },
    }
}

pub fn restore_text(
    vault: &Vault,
    text: &str,
    passphrase: Option<&str>,
    replace: bool,
) -> Result<SyncApplySummary, ShellError> {
    let sealing = passphrase.map(|value| ServicePassword::new(value.to_owned()));
    let snapshot = opened_backup(text, sealing.as_ref())?;
    apply(vault, snapshot, replace)
}

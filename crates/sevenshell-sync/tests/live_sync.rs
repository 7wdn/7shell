//! A backup on a real server, over a real network.
//!
//! Ignored by default: it needs a WebDAV server this machine can reach. Anything that speaks WebDAV
//! will do — a NAS, AList, Nextcloud, nginx with the dav module — and the four variables are the four
//! things a person would type into the settings page:
//!
//! ```text
//! SEVENSHELL_TEST_WEBDAV_URL=https://dav.example.com/remote.php/dav/files/alice/backups \
//!   SEVENSHELL_TEST_WEBDAV_USER=alice SEVENSHELL_TEST_WEBDAV_PASSWORD=... \
//!   SEVENSHELL_TEST_WEBDAV_FOLDER=7shell-live \
//!   cargo test -p sevenshell-sync --test live_sync -- --ignored --test-threads=1
//! ```
//!
//! **What only a real server can show.** The unit tests in `webdav.rs` answer with a fake server this
//! repository wrote, which is a description of what a WebDAV server does rather than evidence that
//! one does it. Here the four verbs meet a server somebody else wrote: `PROPFIND` on a folder that
//! exists, `MKCOL` for a folder that does not, `PUT`, and a `GET` that several servers answer with a
//! redirect to their object store's own address — where the file is then read from *without* the
//! credentials, because the redirect points somewhere else and that is the rule that keeps a
//! password out of a stranger's log.
//!
//! **What the tests leave behind.** The files they wrote, in the folder the variables name: a backup
//! is a thing a server keeps, and nothing here deletes one. Remove the folder when the run is over.
//!
//! ## The same for a code host
//!
//! The other half needs a service that has somewhere to put a small file — a *gist* on GitHub, a
//! *snippet* on GitLab, Gitee's own gist. **Not every code host has one**: GitCode, for instance, has
//! repositories, issues and merge requests and no gist at all, so there is nothing there for this
//! feature to write to and no amount of configuration would change that.
//!
//! ```text
//! SEVENSHELL_TEST_SNIPPET_PROVIDER=github SEVENSHELL_TEST_SNIPPET_TOKEN=... \
//!   cargo test -p sevenshell-sync --test live_sync -- --ignored --test-threads=1 a_real_code_host
//! ```
//!
//! Three more variables: `SEVENSHELL_TEST_SNIPPET_INSTANCE` (GitLab only, the installation's
//! address) and `SEVENSHELL_TEST_SNIPPET_ID`, which is optional and worth setting — the test writes
//! to the file with that id instead of making a new one, so a second run does not leave a second
//! gist behind. The id it used is printed, and nothing here deletes it: removing it is a person's
//! decision about their own account.

use std::time::{SystemTime, UNIX_EPOCH};

use sevenshell_error::ErrorCode;
use sevenshell_sync::{
    Destination, OpenedSnapshot, SnippetSync, Snippets, SyncService, WebDav, build, gather, open, open_snippet,
    plan_restore, seal, with_credentials,
};
use sevenshell_types::{
    AppSettings, AuthMethod, Protocol, Session, SessionAuth, SnippetProvider, SyncSelection, TerminalSettings,
};
use sevenshell_vault::{ServicePassword, SessionSecret, Vault};
use tempfile::{TempDir, tempdir};

/// The server the environment describes, or `None` when there is nothing to talk to.
///
/// `None` is a skip rather than a failure, which is what every live test here does: a machine without
/// a lab server must not fail the suite, and a run that quietly tested nothing must not look green.
fn configured_server(folder: &str, password: Option<&str>) -> Option<SyncService> {
    let (Some(endpoint), Some(folder_name)) = (
        environment("SEVENSHELL_TEST_WEBDAV_URL"),
        environment("SEVENSHELL_TEST_WEBDAV_FOLDER"),
    ) else {
        eprintln!(
            "no WebDAV server is configured; skipping (set SEVENSHELL_TEST_WEBDAV_URL and \
             SEVENSHELL_TEST_WEBDAV_FOLDER, and usually _USER and _PASSWORD too)"
        );
        return None;
    };

    Some(SyncService {
        endpoint,
        username: environment("SEVENSHELL_TEST_WEBDAV_USER"),
        password: password
            .map(|password| ServicePassword::new(password.to_owned()))
            .or_else(|| environment("SEVENSHELL_TEST_WEBDAV_PASSWORD").map(ServicePassword::new)),
        // Each test writes to its own name inside the folder, so running the whole file in one go
        // cannot have one test's file answer another test's question.
        remote_path: format!("{folder_name}/{folder}.json"),
    })
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn client(service: &SyncService) -> WebDav {
    WebDav::new(service).expect("a client")
}

/// A vault on a fresh path, created and left open.
fn vault(area: &TempDir, name: &str) -> Vault {
    let vault = Vault::open_at(area.path().join(format!("{name}.json")));
    vault.create("master password").expect("create");
    vault
}

/// A library with something of everything worth carrying, and one stored login.
fn library(vault: &Vault) {
    vault
        .save_session(
            Session {
                id: "web".to_owned(),
                name: "web".to_owned(),
                group_id: None,
                protocol: Protocol::Ssh,
                host: "web.example".to_owned(),
                port: 22,
                user: "root".to_owned(),
                auth: SessionAuth::with_password(AuthMethod::Password),
                terminal: TerminalSettings::default(),
                jump: Vec::new(),
                forwards: Vec::new(),
                updated_at: 1_700_000_000,
            },
            Some(SessionSecret::Password("hunter2".to_owned())),
        )
        .expect("session");
    vault
        .save_snippet_group(sevenshell_types::SnippetGroup {
            id: "commands".to_owned(),
            name: "Ops".to_owned(),
            order: 1,
            commands: Vec::new(),
        })
        .expect("command group");
    vault
        .save_snippet(sevenshell_types::Snippet {
            id: "c1".to_owned(),
            group_id: "commands".to_owned(),
            name: "uptime".to_owned(),
            prefix: String::new(),
            command: "uptime".to_owned(),
            append_cr: true,
            enabled: true,
            order: 0,
            slot_history: Vec::new(),
        })
        .expect("command");
    vault
        .save_completion_word(sevenshell_types::CompletionWord {
            id: "w1".to_owned(),
            command: "systemctl".to_owned(),
            word: "restart".to_owned(),
            note: String::new(),
        })
        .expect("word");
}

/// Everything a backup carries, with the logins, under a passphrase.
fn everything() -> SyncSelection {
    SyncSelection {
        credentials: true,
        settings: true,
        ..SyncSelection::default()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "needs a real WebDAV server: set SEVENSHELL_TEST_WEBDAV_*"]
async fn a_sealed_backup_survives_a_real_server_and_restores_into_another_vault() {
    let Some(service) = configured_server("sealed", None) else {
        return;
    };
    let area = tempdir().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    let built = build(
        &source,
        &AppSettings::default(),
        &everything(),
        Some("correct horse"),
        now(),
    )
    .expect("built");

    let dav = client(&service);
    dav.probe()
        .await
        .expect("the address is a WebDAV one and the credentials work");
    dav.write(&built.text).await.expect("the file went up");

    // Read back the way the application reads it, and put it into a vault that has nothing.
    let text = dav
        .read()
        .await
        .expect("the server answered")
        .expect("the file that was just written is there");

    let OpenedSnapshot::Sealed(envelope) = open(&text).expect("read as a file") else {
        panic!("a sealed upload must come back sealed");
    };
    let snapshot = envelope.unseal("correct horse").expect("the passphrase opens it");

    let destination = vault(&area, "destination");
    let (restore, _, summary) =
        plan_restore(snapshot, &Destination::of(&destination).expect("destination"), false).into_parts();
    destination.apply_restore(restore).expect("restored");

    let restored = destination.sessions().expect("sessions");
    assert_eq!(restored.len(), 1, "the session travelled");
    assert_eq!(restored[0].name, "web");
    assert!(restored[0].auth.has_secret, "and the stored login came with it, sealed");
    assert_eq!(summary.copied, 0, "nothing was already there to replace or copy");
    assert_eq!(destination.completion_words().expect("words").len(), 1);
}

#[tokio::test]
#[ignore = "needs a real WebDAV server: set SEVENSHELL_TEST_WEBDAV_*"]
async fn a_plain_backup_goes_to_the_same_server_and_comes_back_without_the_logins() {
    let Some(service) = configured_server("plain", None) else {
        return;
    };
    let area = tempdir().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    // No passphrase, so the logins stay here — which the file records rather than leaves to be
    // guessed, and which is the answer most people's first backup has.
    let built = build(&source, &AppSettings::default(), &everything(), None, now()).expect("built");
    assert!(!built.sealed);

    let dav = client(&service);
    dav.write(&built.text).await.expect("the file went up");
    let text = dav.read().await.expect("the server answered").expect("there");

    let OpenedSnapshot::Plain(snapshot) = open(&text).expect("read as a file") else {
        panic!("an upload with no passphrase must come back plain");
    };
    assert!(snapshot.secrets.is_none(), "a plain backup carries no logins");
    assert!(
        !text.contains("hunter2"),
        "and the stored password is not anywhere in what the server now holds"
    );
}

#[tokio::test]
#[ignore = "needs a real WebDAV server: set SEVENSHELL_TEST_WEBDAV_*"]
async fn a_file_that_was_never_written_is_an_answer_and_a_wrong_password_is_a_refusal() {
    let Some(service) = configured_server("absent", None) else {
        return;
    };
    let dav = client(&service);

    assert_eq!(
        dav.read().await.expect("the server answered"),
        None,
        "a file that is not there is the ordinary state of a sync that has never run"
    );

    // A server may be open to anybody on the network — that is a setup somebody may deliberately
    // have — so the sentence is asserted only where there is one to assert, and what is asserted is
    // *which* sentence it is: never a status code somebody has to look up.
    let Some(wrong) = configured_server("absent", Some("definitely-not-the-password")) else {
        return;
    };
    match client(&wrong).probe().await {
        Ok(()) => eprintln!("this server asks for no credentials; nothing to assert about a wrong one"),
        Err(error) => assert_eq!(
            error.code(),
            ErrorCode::SyncAuthFailed,
            "a refused password has its own sentence, not a status to read"
        ),
    }
}

/// The code host the environment describes, or `None` when there is nothing to talk to.
///
/// The provider is a name rather than a list of addresses, because the three services are three APIs
/// and the client needs to know which one it is talking to — which is also why a service with no gist
/// cannot be added by configuration alone.
fn configured_host() -> Option<SnippetSync> {
    let (Some(provider), Some(token)) = (
        environment("SEVENSHELL_TEST_SNIPPET_PROVIDER").and_then(|name| SnippetProvider::from_id(&name)),
        environment("SEVENSHELL_TEST_SNIPPET_TOKEN"),
    ) else {
        eprintln!(
            "no code host is configured; skipping (set SEVENSHELL_TEST_SNIPPET_PROVIDER to github, gitee or \
             gitlab, and SEVENSHELL_TEST_SNIPPET_TOKEN to a token with permission to write one)"
        );
        return None;
    };

    Some(SnippetSync {
        provider,
        instance_url: environment("SEVENSHELL_TEST_SNIPPET_INSTANCE"),
        token: ServicePassword::new(token),
        snippet_id: environment("SEVENSHELL_TEST_SNIPPET_ID"),
    })
}

#[tokio::test]
#[ignore = "needs a real code host with gists: set SEVENSHELL_TEST_SNIPPET_*"]
async fn a_real_code_host_takes_a_backup_and_gives_it_back() {
    let Some(sync) = configured_host() else {
        return;
    };
    let area = tempdir().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    // The same snapshot a server or a file gets, in the envelope a snippet carries — assembled the
    // way the host assembles it, in the same two steps, so this test cannot pass by taking a path
    // the application does not take.
    let passphrase = "correct horse battery staple";
    let mut snapshot =
        gather(&source, &AppSettings::default(), &everything(), Some(passphrase), now()).expect("assembled");
    with_credentials(&source, &mut snapshot).expect("the logins were read");
    let text = seal(&snapshot, passphrase, &Default::default()).expect("sealed");

    let client = Snippets::new(&sync).expect("a client");
    client.probe().await.expect("the token is good and the account answers");

    // A token the service does not know is the credentials sentence, against a real 401 rather than
    // one this repository wrote.
    let wrong = SnippetSync {
        token: ServicePassword::new("definitely-not-a-token".to_owned()),
        ..sync.clone()
    };
    assert_eq!(
        Snippets::new(&wrong)
            .expect("a client")
            .probe()
            .await
            .expect_err("refused")
            .code(),
        ErrorCode::SyncAuthFailed
    );

    let id = client.write(&text).await.expect("the file was written");
    eprintln!(
        "the snippet this run used is {id} on {provider}",
        provider = sync.provider
    );

    let reader = Snippets::new(&SnippetSync {
        snippet_id: Some(id.clone()),
        ..sync.clone()
    })
    .expect("a client");

    let held = reader.read().await.expect("the file came back");
    let snapshot = open_snippet(&held, passphrase).expect("it is this application's envelope");
    assert_eq!(snapshot.sessions.len(), 1, "the session travelled");
    assert_eq!(snapshot.sessions[0].name, "web");
    assert!(
        snapshot.secrets.is_some(),
        "and the stored login came with it, because a snippet is always sealed"
    );
    assert!(
        open_snippet(&held, "not the passphrase").is_err(),
        "a snippet that opened without the passphrase would be one anybody could read"
    );

    // Writing again patches the same file instead of making a second one: the id comes back
    // unchanged, which is what a person's account looks like when this works.
    assert_eq!(
        reader.write(&text).await.expect("written over"),
        id,
        "the second upload went to the same snippet"
    );
}

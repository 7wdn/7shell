//! A backup, taken and put back.
//!
//! These are the rules of the file format and of a restore, exercised through the crate's own
//! public surface and against real vaults on a real disk: what travels, what stays, what happens
//! when an id is already taken, and what a file says about itself when it is opened.

use sevenshell_error::ErrorCode;
use sevenshell_sync::{
    Destination, OpenedSnapshot, PortableSettings, SyncSnapshot, build, open, plan_restore, read, write,
};
use sevenshell_types::{
    AppSettings, AuthMethod, CompletionWord, KeyInfo, KnownHostEntry, Protocol, Session, SessionAuth, SessionGroup,
    Snippet, SnippetGroup, SyncApplySummary, SyncSelection, TerminalSettings,
};
use sevenshell_vault::{SessionSecret, Vault, VaultStatus};
use tempfile::TempDir;

/// A vault on a fresh path, created and left open.
fn vault(directory: &TempDir, name: &str) -> Vault {
    let vault = Vault::open_at(directory.path().join(format!("{name}.json")));
    assert_eq!(vault.status(), VaultStatus::Missing);
    vault.create("master password").expect("create");
    vault
}

fn session(id: &str, name: &str) -> Session {
    Session {
        id: id.to_owned(),
        name: name.to_owned(),
        group_id: None,
        protocol: Protocol::Ssh,
        host: format!("{name}.example"),
        port: 22,
        user: "root".to_owned(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 1_700_000_000,
    }
}

fn group(id: &str, name: &str) -> SessionGroup {
    SessionGroup {
        id: id.to_owned(),
        name: name.to_owned(),
        parent_id: None,
        order: 0,
        pinned: false,
    }
}

fn command(id: &str, group_id: &str, name: &str) -> Snippet {
    Snippet {
        id: id.to_owned(),
        group_id: group_id.to_owned(),
        name: name.to_owned(),
        prefix: String::new(),
        command: "uptime".to_owned(),
        append_cr: true,
        enabled: true,
        order: 0,
        slot_history: Vec::new(),
    }
}

fn word(id: &str, word: &str) -> CompletionWord {
    CompletionWord {
        id: id.to_owned(),
        command: "systemctl".to_owned(),
        word: word.to_owned(),
        note: String::new(),
    }
}

fn host(host: &str) -> KnownHostEntry {
    KnownHostEntry {
        host: host.to_owned(),
        port: 22,
        key_type: "ssh-ed25519".to_owned(),
        public_key: "AAAAC3NzaC1lZDI1NTE5AAAAI".to_owned(),
        fingerprint: format!("SHA256:{host}"),
        added_at: 1,
    }
}

fn key_info(name: &str) -> KeyInfo {
    KeyInfo {
        key_ref: None,
        display_name: name.to_owned(),
        algorithm: "ssh-ed25519".to_owned(),
        fingerprint: "SHA256:key-one".to_owned(),
        has_passphrase: false,
    }
}

const KEY_CONTENTS: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nnot a real key\n-----END OPENSSH PRIVATE KEY-----\n";

/// A library with one of everything worth carrying.
fn library(vault: &Vault) {
    vault.save_group(group("work", "Work")).expect("group");
    vault
        .save_session(
            Session {
                group_id: Some("work".to_owned()),
                ..session("web", "web")
            },
            Some(SessionSecret::Password("hunter2".to_owned())),
        )
        .expect("session");
    vault.save_session(session("db", "db"), None).expect("session");

    vault
        .save_snippet_group(SnippetGroup {
            id: "commands".to_owned(),
            name: "Ops".to_owned(),
            order: 1,
            commands: Vec::new(),
        })
        .expect("snippet group");
    vault
        .save_snippet(command("c1", "commands", "uptime"))
        .expect("snippet");
    vault.save_completion_word(word("w1", "restart")).expect("word");
    vault.remember_command("ls -la /etc", 1_700_000_001).expect("history");
    vault.trust_host(host("web.example")).expect("trust");
    vault
        .import_key(
            KEY_CONTENTS.to_owned(),
            Some("key secret".to_owned()),
            key_info("id_ed25519"),
        )
        .expect("key");
}

/// Takes a snapshot of a vault, through the same call the interface makes.
fn snapshot(vault: &Vault, selection: &SyncSelection, passphrase: Option<&str>) -> String {
    build(vault, &AppSettings::default(), selection, passphrase, 1_700_000_000)
        .expect("build")
        .text
}

/// How many settings a snapshot of the default settings carries.
fn default_settings_count() -> u32 {
    PortableSettings::of(&AppSettings::default()).count()
}

/// A file with nothing in it but the sessions given, for the rules about what a file may point at.
///
/// Spelled out field by field rather than built by the writer, because what these tests are about is
/// what a *reader* does with a file — including one this build would never write.
fn file_with(sessions: Vec<Session>) -> SyncSnapshot {
    SyncSnapshot {
        format: sevenshell_sync::SNAPSHOT_FORMAT.to_owned(),
        version: sevenshell_sync::SNAPSHOT_VERSION,
        exported_at: 1_700_000_000,
        app_version: "elsewhere".to_owned(),
        selection: SyncSelection::default(),
        sessions,
        groups: Vec::new(),
        snippets: Vec::new(),
        words: Vec::new(),
        history: None,
        known_hosts: None,
        settings: None,
        secrets: None,
        keys: None,
    }
}

/// Restores a snapshot's text into a vault: open, unseal when it is sealed, plan, and write.
///
/// The two writes are the host's, and this is what the host does with them: the vault's document in
/// one call, and the settings afterwards.
fn restore(text: &str, passphrase: Option<&str>, into: &Vault, replace: bool) -> SyncApplySummary {
    let snapshot = match open(text).expect("open") {
        OpenedSnapshot::Plain(snapshot) => *snapshot,
        OpenedSnapshot::Sealed(envelope) => envelope.unseal(passphrase.expect("a passphrase")).expect("unseal"),
    };
    let destination = Destination::of(into).expect("destination");
    let (restore, settings, summary) = plan_restore(snapshot, &destination, replace).into_parts();

    into.apply_restore(restore).expect("apply");

    if let Some(travelling) = settings {
        let mut current = AppSettings::default();
        travelling.apply_to(&mut current);
    }

    summary
}

#[test]
fn everything_worth_carrying_goes_and_comes_back() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    let selection = SyncSelection {
        history: true,
        known_hosts: true,
        ..SyncSelection::default()
    };

    let text = snapshot(&source, &selection, None);
    let target = vault(&area, "target");
    let summary = restore(&text, None, &target, false);

    assert_eq!(summary.contents.sessions, 2);
    assert_eq!(summary.contents.groups, 1);
    assert_eq!(summary.contents.snippets, 1);
    assert_eq!(summary.contents.words, 1);
    assert_eq!(summary.contents.history, 1);
    assert_eq!(summary.contents.known_hosts, 1);
    assert_eq!(summary.contents.settings, default_settings_count());
    assert!(default_settings_count() >= 18, "and that number is not vacuous");
    assert_eq!(summary.contents.secrets, 0, "no passphrase, so no logins");
    assert_eq!(summary.contents.keys, 0);

    let sessions = target.sessions().expect("sessions");
    assert_eq!(sessions.len(), 2);
    let web = sessions.iter().find(|entry| entry.name == "web").expect("web");
    assert_eq!(web.group_id.as_deref(), Some("work"));
    assert!(!web.auth.has_secret, "the login did not travel, and the flag says so");

    let groups = target.groups().expect("groups");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "Work");

    let commands = target.snippet_groups().expect("snippets");
    assert_eq!(commands.iter().map(|group| group.commands.len()).sum::<usize>(), 1);
    assert_eq!(target.completion_words().expect("words").len(), 1);
    assert_eq!(target.command_history().expect("history").len(), 1);
    assert_eq!(target.known_hosts().expect("hosts").len(), 1);
}

#[test]
fn a_snapshot_with_a_passphrase_carries_the_logins_and_the_keys() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    let selection = SyncSelection {
        credentials: true,
        ..SyncSelection::default()
    };

    let text = snapshot(&source, &selection, Some("correct horse"));
    let target = vault(&area, "target");
    let summary = restore(&text, Some("correct horse"), &target, false);

    assert_eq!(summary.contents.secrets, 1);
    assert_eq!(summary.contents.keys, 1);

    let sessions = target.sessions().expect("sessions");
    let web = sessions.iter().find(|entry| entry.name == "web").expect("web");
    assert!(web.auth.has_secret, "the login travelled");
    assert!(
        target.secret_for_session(&web.id).expect("read").is_some(),
        "and it is stored against the session that arrived"
    );

    let keys = target.keys().expect("keys");
    assert_eq!(keys.len(), 1);
    assert!(keys[0].has_passphrase, "the key's own passphrase came with it");
    let restored = target
        .key_material(keys[0].key_ref.as_deref().expect("a handle"))
        .expect("read")
        .expect("stored");
    assert_eq!(restored.into_parts().1.as_deref(), Some("key secret"));
}

#[test]
fn a_sealed_snapshot_says_nothing_about_what_is_inside_it() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    let selection = SyncSelection {
        credentials: true,
        ..SyncSelection::default()
    };
    let text = snapshot(&source, &selection, Some("correct horse"));

    for leak in ["hunter2", "web.example", "key secret", "not a real key", "Work"] {
        assert!(
            !text.contains(leak),
            "a sealed snapshot must not name what it holds, and it spells out {leak}"
        );
    }

    let OpenedSnapshot::Sealed(_) = open(&text).expect("open") else {
        panic!("a sealed file must be read as sealed");
    };
}

#[test]
fn a_login_without_a_passphrase_is_not_written_in_the_clear() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    let selection = SyncSelection {
        credentials: true,
        ..SyncSelection::default()
    };

    // Asked for, with nothing to seal them under: the snapshot is written without them.
    let text = snapshot(&source, &selection, None);

    assert!(!text.contains("hunter2"));
    let OpenedSnapshot::Plain(snapshot) = open(&text).expect("open") else {
        panic!("nothing was sealed, so nothing should be sealed");
    };
    assert!(!snapshot.selection.credentials, "the file says what it does hold");
    assert_eq!(snapshot.contents().secrets, 0);
}

#[test]
fn a_passphrase_too_short_to_be_one_is_refused() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");

    let selection = SyncSelection {
        credentials: true,
        ..SyncSelection::default()
    };

    let error = build(&source, &AppSettings::default(), &selection, Some("abc"), 0).expect_err("too short");

    assert_eq!(error.code(), ErrorCode::SyncPassphraseShort);
}

#[test]
fn the_wrong_passphrase_opens_nothing() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    let selection = SyncSelection {
        credentials: true,
        ..SyncSelection::default()
    };
    let text = snapshot(&source, &selection, Some("correct horse"));

    let OpenedSnapshot::Sealed(envelope) = open(&text).expect("open") else {
        panic!("sealed");
    };
    let error = envelope.unseal("Correct Horse").expect_err("not the passphrase");

    assert_eq!(error.code(), ErrorCode::SyncPassphrase);
}

#[test]
fn a_snapshot_from_a_future_build_is_refused_rather_than_guessed_at() {
    let sealed = r#"{"format":"sevenshell-sync","version":99,"exportedAt":0,"protection":"passphrase"}"#;
    let error = open(sealed).expect_err("newer than this build");
    assert_eq!(error.code(), ErrorCode::StoreSchemaNewer);

    let plain = r#"{"format":"sevenshell-sync","version":99,"exportedAt":0,"appVersion":"9","selection":{}}"#;
    let error = open(plain).expect_err("newer than this build");
    assert_eq!(error.code(), ErrorCode::StoreSchemaNewer);
}

#[test]
fn a_file_that_is_not_a_backup_is_refused_by_its_format_field() {
    let other = r#"{"format":"sevenshell-sessions","version":1,"exportedAt":0}"#;
    let error = open(other).expect_err("somebody else's file");
    assert_eq!(error.code(), ErrorCode::SyncSnapshotInvalid);
}

#[test]
fn a_sealed_file_is_not_read_as_a_plain_one_with_nothing_in_it() {
    // The failure this guards against is silent: every field a sealed envelope does not have is
    // optional, so a plain read of one would hand back an empty library rather than an error.
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    let selection = SyncSelection {
        credentials: true,
        ..SyncSelection::default()
    };
    let text = snapshot(&source, &selection, Some("correct horse"));

    assert!(matches!(open(&text).expect("open"), OpenedSnapshot::Sealed(_)));
}

#[test]
fn restoring_the_same_file_twice_makes_a_copy_rather_than_replacing_what_is_here() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);
    let text = snapshot(&source, &SyncSelection::default(), None);

    let target = vault(&area, "target");
    let first = restore(&text, None, &target, false);
    let second = restore(&text, None, &target, false);

    assert_eq!(first.copied, 0, "nothing was here to collide with");
    assert_eq!(
        second.copied, 4,
        "two sessions, their group and a command arrived as copies"
    );
    assert_eq!(second.replaced, 0);
    assert_eq!(target.sessions().expect("sessions").len(), 4);

    let ids: std::collections::HashSet<String> = target
        .sessions()
        .expect("sessions")
        .into_iter()
        .map(|session| session.id)
        .collect();
    assert_eq!(ids.len(), 4, "every copy has an id of its own");
}

#[test]
fn replacing_writes_over_what_is_here_and_keeps_the_ids() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);
    let target = vault(&area, "target");

    // The same library on both sides, with one name changed on the target: a replacement is what
    // brings it back into step.
    restore(
        &snapshot(&source, &SyncSelection::default(), None),
        None,
        &target,
        false,
    );
    let renamed = target
        .sessions()
        .expect("sessions")
        .into_iter()
        .find(|session| session.name == "web")
        .expect("web");
    target
        .save_session(
            Session {
                name: "renamed here".to_owned(),
                ..renamed.clone()
            },
            None,
        )
        .expect("rename");

    let summary = restore(&snapshot(&source, &SyncSelection::default(), None), None, &target, true);

    // Two sessions, their group, a command — and a word, which is the one kind of thing that
    // collides by *content* rather than by id: without replacement the row that is here is left
    // alone (and is not counted as a copy, because nothing was copied), and with it the file's
    // version is written over it. That is why this number is one larger than the copy count in the
    // test above.
    assert_eq!(
        summary.replaced, 5,
        "two sessions, their group, a command and a word were written over"
    );
    assert_eq!(summary.copied, 0);
    assert_eq!(target.sessions().expect("sessions").len(), 2, "no duplicates");
    let sessions = target.sessions().expect("sessions");
    let names: Vec<&str> = sessions.iter().map(|session| session.name.as_str()).collect();
    assert!(names.contains(&"web"), "and the file's version won: {names:?}");
    assert!(!names.contains(&"renamed here"));
}

#[test]
fn a_session_whose_group_was_not_carried_lands_at_the_root() {
    let area = TempDir::new().expect("temp dir");
    let target = vault(&area, "target");

    // A file that names a group it does not carry: somebody edited it, or a later build narrowed it.
    // Either way the rule is the same — what arrived must not nest itself inside something that
    // happens to be on this machine, and the file has no group to put it in.
    let snapshot = file_with(vec![Session {
        group_id: Some("a-group-this-file-does-not-have".to_owned()),
        ..session("web", "web")
    }]);

    let destination = Destination::of(&target).expect("destination");
    let (restore, _, _) = plan_restore(snapshot, &destination, false).into_parts();
    target.apply_restore(restore).expect("apply");

    let sessions = target.sessions().expect("sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].group_id, None);
}

#[test]
fn a_hop_the_file_does_not_carry_is_dropped_and_counted() {
    let area = TempDir::new().expect("temp dir");
    let target = vault(&area, "target");

    // A file naming two machines as hops and carrying only one of them: the other names nothing, and
    // a session that went through it would go somewhere nobody chose.
    let snapshot = file_with(vec![
        Session {
            jump: vec!["web".to_owned(), "not-in-this-file".to_owned()],
            ..session("through", "through")
        },
        session("web", "web"),
    ]);

    let destination = Destination::of(&target).expect("destination");
    let (restore, _, summary) = plan_restore(snapshot, &destination, false).into_parts();
    target.apply_restore(restore).expect("apply");

    assert_eq!(summary.dropped_hops, 1);
    let sessions = target.sessions().expect("sessions");
    assert_eq!(sessions.len(), 2, "both machines are still here");
    let through = sessions.iter().find(|entry| entry.name == "through").expect("through");
    assert_eq!(through.jump.len(), 1, "the hop that was there travelled");
    assert!(sessions.iter().any(|entry| entry.id == through.jump[0]));
}

#[test]
fn a_session_naming_a_key_the_file_did_not_carry_is_counted() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    let imported = source
        .import_key(KEY_CONTENTS.to_owned(), None, key_info("id_ed25519"))
        .expect("key");
    source
        .save_session(
            Session {
                auth: SessionAuth {
                    method: AuthMethod::PublicKey,
                    key: Some(imported),
                    has_secret: false,
                },
                ..session("keyed", "keyed")
            },
            None,
        )
        .expect("session");

    // No credentials: the key stays behind, and the session that names it says so.
    let text = snapshot(&source, &SyncSelection::default(), None);
    let target = vault(&area, "target");
    let summary = restore(&text, None, &target, false);

    assert_eq!(summary.missing_keys, 1);
    assert_eq!(summary.contents.keys, 0);
    assert_eq!(target.sessions().expect("sessions").len(), 1, "it is still imported");
}

#[test]
fn a_session_pointing_at_a_key_that_travelled_points_at_it_here() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    let imported = source
        .import_key(KEY_CONTENTS.to_owned(), None, key_info("id_ed25519"))
        .expect("key");
    source
        .save_session(
            Session {
                auth: SessionAuth {
                    method: AuthMethod::PublicKey,
                    key: Some(imported),
                    has_secret: false,
                },
                ..session("keyed", "keyed")
            },
            None,
        )
        .expect("session");

    let selection = SyncSelection {
        credentials: true,
        ..SyncSelection::default()
    };
    let text = snapshot(&source, &selection, Some("correct horse"));

    let target = vault(&area, "target");
    let summary = restore(&text, Some("correct horse"), &target, false);

    assert_eq!(summary.missing_keys, 0, "the key came with the file");
    assert_eq!(summary.contents.keys, 1);
    let keys = target.keys().expect("keys");
    let name = target.sessions().expect("sessions")[0]
        .auth
        .key
        .as_ref()
        .and_then(|key| key.key_ref.clone())
        .expect("a handle");
    assert_eq!(
        Some(&name),
        keys[0].key_ref.as_ref(),
        "the session points at the key that arrived"
    );
}

#[test]
fn a_key_this_machine_already_holds_is_recognised_rather_than_imported_twice() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    let target = vault(&area, "target");

    // The same key on both sides, under different handles — which is what importing the same file
    // by hand on two machines looks like.
    for vault in [&source, &target] {
        vault
            .import_key(KEY_CONTENTS.to_owned(), None, key_info("id_ed25519"))
            .expect("key");
    }

    let selection = SyncSelection {
        credentials: true,
        ..SyncSelection::default()
    };
    let summary = restore(
        &snapshot(&source, &selection, Some("correct horse")),
        Some("correct horse"),
        &target,
        false,
    );

    assert_eq!(summary.contents.keys, 0, "there was nothing to import");
    assert_eq!(target.keys().expect("keys").len(), 1, "and no second copy");
}

#[test]
fn restoring_a_history_adds_what_is_missing_and_keeps_what_is_here() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    source.remember_command("from the file", 100).expect("history");
    let target = vault(&area, "target");
    target.remember_command("from this machine", 200).expect("history");

    let selection = SyncSelection {
        history: true,
        ..SyncSelection::default()
    };
    let text = snapshot(&source, &selection, None);
    let first = restore(&text, None, &target, false);

    assert_eq!(first.contents.history, 1, "the one that was missing");
    assert_eq!(target.command_history().expect("history").len(), 2);

    // And again: the same file adds nothing the second time.
    let second = restore(&text, None, &target, false);
    assert_eq!(second.contents.history, 1);
    assert_eq!(target.command_history().expect("history").len(), 2);
}

#[test]
fn a_diary_and_a_trust_list_are_not_carried_unless_they_were_asked_for() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);

    let text = snapshot(&source, &SyncSelection::default(), None);
    let OpenedSnapshot::Plain(snapshot) = open(&text).expect("open") else {
        panic!("plain");
    };

    assert!(snapshot.history.is_none(), "a diary is not a default");
    assert!(snapshot.known_hosts.is_none(), "nor is what a machine trusts");
    assert_eq!(snapshot.contents().history, 0);
}

#[test]
fn the_commands_land_in_the_group_they_were_in() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);
    let text = snapshot(&source, &SyncSelection::default(), None);

    let target = vault(&area, "target");
    restore(&text, None, &target, false);

    let groups = target.snippet_groups().expect("snippets");
    let ops = groups.iter().find(|group| group.name == "Ops").expect("Ops");
    assert_eq!(ops.commands.len(), 1);
    assert_eq!(
        ops.commands[0].group_id, ops.id,
        "the command sits where it says it sits"
    );
}

#[test]
fn a_snapshot_that_could_not_be_written_is_refused_by_its_size() {
    let area = TempDir::new().expect("temp dir");
    let path = area.path().join("too-big.json");
    let huge = "x".repeat(sevenshell_sync::MAX_SNAPSHOT_BYTES as usize + 1);

    let error = write(&path, &huge).expect_err("too large");
    assert_eq!(error.code(), ErrorCode::SyncIo);
    assert!(!path.exists(), "and nothing was written");
}

#[test]
fn a_backup_file_survives_a_round_trip_through_the_disk() {
    let area = TempDir::new().expect("temp dir");
    let source = vault(&area, "source");
    library(&source);
    let text = snapshot(&source, &SyncSelection::default(), None);

    let path = area.path().join("library.backup.json");
    write(&path, &text).expect("write");

    let read_back = read(&path).expect("read");
    assert_eq!(read_back, text);

    let OpenedSnapshot::Plain(snapshot) = open(&read_back).expect("open") else {
        panic!("plain");
    };
    assert_eq!(
        snapshot.contents().sessions,
        source.sessions().expect("sessions").len() as u32
    );
}

//! The wire shapes the interface receives.
//!
//! Two things are checked here, and both have gone wrong at least once:
//!
//! 1. **Every field is camelCase.** `rename_all` on an enum renames the *variants*, not the fields
//!    inside them: `Opened { session_id }` went out as `session_id` while the interface read
//!    `sessionId`, and an interface reading `undefined` does not complain — it does something
//!    worse, like iterating nothing and reporting that the whole operation failed.
//! 2. **A wire type can actually be serialized.** One that cannot fails at the last moment, where
//!    the only thing the interface sees is "something went wrong inside the application".

use std::collections::BTreeMap;

use serde::Serialize;
use sevenshell_types::{
    AppLayout, AppSettings, AuthMethod, BackgroundDisplayMode, BackgroundImage, BatchHostOutcome, BatchHostRef,
    BatchHostResult, BatchOutcome, BatchRequest, CommandHistoryEntry, CompletionWord, CornerStyle, CpuCounters,
    CpuModel, CpuStates, CreatedKey, CustomColors, DiskUsage, DownloadFolderView, EnvironmentEntry, FilesLayout,
    ForwardMode, HomeLayout, HostFailure, HostMetrics, HostSystem, KeyAlgorithm, KeyInfo, KnownHostEntry, LoadAverage,
    LocalForward, MemoryUsage, NetworkCounters, OpenOutcome, OpenTunnel, PresentedHostKey, ProcessDetail, ProcessPage,
    ProcessSample, Protocol, RiskAssessment, RiskLevel, RiskReason, SendFailure, SendOutcome, SendReport, SendTarget,
    Session, SessionAuth, SessionExportSummary, SessionGroup, SessionImportOutcome, SessionImportSummary,
    SessionLayout, Snippet, SnippetGroup, SnippetProvider, SnippetStatus, SnippetUploadReceipt, SocketRow,
    SyncApplySummary, SyncCatalog, SyncContents, SyncDownloadOutcome, SyncInspectOutcome, SyncReceipt, SyncSelection,
    SyncServiceStatus, SyncUploadReceipt, TabGroupMode, TabLayout, TabMaxWidth, TabPlacement, TabSortMode,
    TerminalSettings, ThemeMode, ThemePalette, TunnelFailure, UploadAnswer, UploadOutcome, VaultState,
};

/// Fails on any key that is not camelCase, however deep it sits.
fn assert_camel_case_keys(value: &serde_json::Value, path: &str) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, nested) in fields {
                assert!(!key.contains('_'), "{path}.{key} is snake_case on the wire");
                assert!(
                    !key.chars().next().is_some_and(char::is_uppercase),
                    "{path}.{key} starts with an upper-case letter"
                );
                assert_camel_case_keys(nested, &format!("{path}.{key}"));
            }
        }
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                assert_camel_case_keys(item, &format!("{path}[{index}]"));
            }
        }
        _ => {}
    }
}

fn check<T: Serialize>(value: &T, path: &str) -> serde_json::Value {
    let json = serde_json::to_value(value).unwrap_or_else(|error| panic!("{path} does not serialize: {error}"));
    assert_camel_case_keys(&json, path);
    json
}

fn presented() -> PresentedHostKey {
    PresentedHostKey {
        host: "host.example".to_owned(),
        port: 22,
        key_type: "ssh-ed25519".to_owned(),
        public_key: "AAAA".to_owned(),
        fingerprint: "SHA256:AAAA".to_owned(),
    }
}

fn trusted() -> KnownHostEntry {
    KnownHostEntry {
        host: "host.example".to_owned(),
        port: 22,
        key_type: "ssh-ed25519".to_owned(),
        public_key: "AAAA".to_owned(),
        fingerprint: "SHA256:AAAA".to_owned(),
        added_at: 1,
    }
}

#[test]
fn every_outcome_serializes_with_camel_case_fields() {
    let unknown = check(&OpenOutcome::HostKeyUnknown { key: presented() }, "HostKeyUnknown");
    assert_eq!(unknown["kind"], serde_json::json!("hostKeyUnknown"));

    let mismatch = check(
        &OpenOutcome::HostKeyMismatch {
            key: presented(),
            trusted: trusted(),
        },
        "HostKeyMismatch",
    );
    assert_eq!(mismatch["kind"], serde_json::json!("hostKeyMismatch"));
    assert_eq!(mismatch["trusted"]["addedAt"], serde_json::json!(1));

    let opened = check(
        &OpenOutcome::Opened {
            handle: "t1".to_owned(),
            session_id: "session-1".to_owned(),
            tunnels: vec![OpenTunnel {
                id: "t1-f1".to_owned(),
                handle: "t1".to_owned(),
                session_id: "session-1".to_owned(),
                bind_host: "127.0.0.1".to_owned(),
                bind_port: 18080,
                mode: ForwardMode::Local,
                target_host: "db.internal".to_owned(),
                target_port: 5432,
                connections: 0,
            }],
            tunnel_failures: vec![TunnelFailure {
                forward_id: "f2".to_owned(),
                bind_host: "127.0.0.1".to_owned(),
                bind_port: 18081,
                detail: "the port is already in use".to_owned(),
            }],
        },
        "Opened",
    );

    // The names the interface reads, spelled exactly as it reads them.
    assert_eq!(opened["kind"], serde_json::json!("opened"));
    assert_eq!(opened["sessionId"], serde_json::json!("session-1"));
    assert_eq!(opened["tunnels"][0]["bindPort"], serde_json::json!(18080));
    assert_eq!(opened["tunnelFailures"][0]["forwardId"], serde_json::json!("f2"));
}

#[test]
fn an_upload_that_found_a_name_serializes_with_camel_case_fields() {
    let started = check(
        &UploadOutcome::Started {
            ids: vec!["t1".to_owned()],
        },
        "Started",
    );
    assert_eq!(started["kind"], serde_json::json!("started"));
    assert_eq!(started["ids"][0], serde_json::json!("t1"));

    // The question carries the names it is about, and whether skipping is a real answer for it.
    let conflict = check(
        &UploadOutcome::Conflict {
            token: "upload-1".to_owned(),
            names: vec!["notes.txt".to_owned()],
            skippable: true,
        },
        "Conflict",
    );
    assert_eq!(conflict["kind"], serde_json::json!("conflict"));
    assert_eq!(conflict["token"], serde_json::json!("upload-1"));
    assert_eq!(conflict["names"][0], serde_json::json!("notes.txt"));
    assert_eq!(conflict["skippable"], serde_json::json!(true));

    let cancelled = check(&UploadOutcome::Cancelled, "Cancelled");
    assert_eq!(cancelled["kind"], serde_json::json!("cancelled"));

    // The answer travels as the interface spells it, one word per variant.
    assert_eq!(
        serde_json::to_value(UploadAnswer::Overwrite).expect("the answer serializes"),
        serde_json::json!("overwrite")
    );
    assert_eq!(
        serde_json::to_value(UploadAnswer::Skip).expect("the answer serializes"),
        serde_json::json!("skip")
    );
    assert_eq!(
        serde_json::to_value(UploadAnswer::Cancel).expect("the answer serializes"),
        serde_json::json!("cancel")
    );
}

#[test]
fn an_import_that_stopped_to_ask_serializes_with_camel_case_fields() {
    // The shape the first half of this file is about: an enum whose variants carry fields, where
    // `rename_all` alone would put `file_name` and `missing_keys` on the wire and the interface
    // would read `undefined` for both.
    let asked = check(
        &SessionImportOutcome::NeedsPassphrase {
            file_name: "library.json".to_owned(),
        },
        "NeedsPassphrase",
    );
    assert_eq!(asked["kind"], serde_json::json!("needsPassphrase"));
    assert_eq!(asked["fileName"], serde_json::json!("library.json"));

    let imported = check(
        &SessionImportOutcome::Imported {
            summary: SessionImportSummary {
                sessions: 1,
                groups: 2,
                secrets: 3,
                missing_keys: 4,
                dropped_hops: 5,
            },
        },
        "Imported",
    );
    assert_eq!(imported["kind"], serde_json::json!("imported"));
    assert_eq!(imported["summary"]["missingKeys"], serde_json::json!(4));
    assert_eq!(imported["summary"]["droppedHops"], serde_json::json!(5));
}

#[test]
fn a_backup_catalog_serializes_with_camel_case_fields() {
    let contents = SyncContents {
        sessions: 1,
        groups: 2,
        snippets: 3,
        words: 4,
        history: 5,
        known_hosts: 6,
        settings: 7,
        secrets: 8,
        keys: 9,
    };

    let catalog = check(
        &SyncCatalog {
            contents,
            sealed: true,
            exported_at: 1_700_000_000,
            app_version: "0.1.0".to_owned(),
        },
        "SyncCatalog",
    );
    assert_eq!(catalog["contents"]["knownHosts"], serde_json::json!(6));
    assert_eq!(catalog["appVersion"], serde_json::json!("0.1.0"));

    let receipt = check(
        &SyncReceipt {
            contents,
            sealed: false,
            file_name: "library.json".to_owned(),
        },
        "SyncReceipt",
    );
    assert_eq!(receipt["fileName"], serde_json::json!("library.json"));
    assert_eq!(receipt["sealed"], serde_json::json!(false));
}

#[test]
fn reading_a_backup_that_stopped_to_ask_serializes_with_camel_case_fields() {
    let asked = check(
        &SyncInspectOutcome::NeedsPassphrase {
            file_name: "library.json".to_owned(),
        },
        "NeedsPassphrase",
    );
    assert_eq!(asked["kind"], serde_json::json!("needsPassphrase"));
    assert_eq!(asked["fileName"], serde_json::json!("library.json"));

    let read = check(
        &SyncInspectOutcome::Read {
            catalog: SyncCatalog {
                contents: SyncContents::default(),
                sealed: true,
                exported_at: 1,
                app_version: "0.1.0".to_owned(),
            },
            file_name: "library.json".to_owned(),
        },
        "Read",
    );
    assert_eq!(read["kind"], serde_json::json!("read"));
    assert_eq!(read["fileName"], serde_json::json!("library.json"));
    assert_eq!(read["catalog"]["contents"]["knownHosts"], serde_json::json!(0));
    assert_eq!(read["catalog"]["sealed"], serde_json::json!(true));
}

#[test]
fn a_restore_summary_serializes_with_camel_case_fields() {
    let applied = check(
        &SyncApplySummary {
            contents: SyncContents::default(),
            replaced: 1,
            copied: 2,
            missing_keys: 4,
            dropped_hops: 5,
        },
        "SyncApplySummary",
    );

    assert_eq!(applied["missingKeys"], serde_json::json!(4));
    assert_eq!(applied["droppedHops"], serde_json::json!(5));
    assert_eq!(applied["contents"]["knownHosts"], serde_json::json!(0));
}

#[test]
fn a_sync_service_status_and_an_upload_receipt_serialize_with_camel_case_fields() {
    let status = check(
        &SyncServiceStatus {
            has_password: true,
            has_passphrase: false,
        },
        "SyncServiceStatus",
    );
    assert_eq!(status["hasPassword"], serde_json::json!(true));
    assert_eq!(status["hasPassphrase"], serde_json::json!(false));

    let receipt = check(
        &SyncUploadReceipt {
            contents: SyncContents::default(),
            sealed: true,
            remote_path: "7shell/sync/backup.json".to_owned(),
        },
        "SyncUploadReceipt",
    );
    assert_eq!(receipt["remotePath"], serde_json::json!("7shell/sync/backup.json"));
    assert_eq!(receipt["contents"]["knownHosts"], serde_json::json!(0));
}

#[test]
fn a_snippet_account_and_its_receipt_speak_the_interface_s_words() {
    // The provider is the one wire value here that is not camelCase, and it is deliberate: these are
    // the services' own names, lower case as they write them, and `gitHub` would be neither.
    for provider in SnippetProvider::ALL {
        let json = check(provider, "SnippetProvider");
        assert_eq!(
            json,
            serde_json::json!(provider.as_str()),
            "the wire name and the stored name are one name"
        );
    }
    assert_eq!(
        SnippetProvider::default(),
        SnippetProvider::ALL[0],
        "the service nobody chose yet is the first one the list offers"
    );

    let status = check(
        &SnippetStatus {
            account: "gitlab:https://gitlab.example.com".to_owned(),
            has_token: true,
        },
        "SnippetStatus",
    );
    assert_eq!(status["hasToken"], serde_json::json!(true));
    assert_eq!(
        status["account"],
        serde_json::json!("gitlab:https://gitlab.example.com")
    );

    let uploaded = check(
        &SnippetUploadReceipt {
            contents: SyncContents::default(),
            sealed: true,
            provider: SnippetProvider::GitLab,
            snippet_id: "42".to_owned(),
        },
        "SnippetUploadReceipt",
    );
    assert_eq!(uploaded["provider"], serde_json::json!("gitlab"));
    assert_eq!(uploaded["snippetId"], serde_json::json!("42"));
    assert_eq!(
        uploaded["sealed"],
        serde_json::json!(true),
        "a snippet is always sealed, and the receipt says so rather than implying it"
    );
}

#[test]
fn a_download_folder_view_carries_names_and_no_path_to_anything() {
    let view = check(
        &DownloadFolderView {
            current: Some("fsdownload".to_owned()),
            history: vec!["fsdownload".to_owned(), "Desktop".to_owned()],
        },
        "DownloadFolderView",
    );

    assert_eq!(
        view["history"],
        serde_json::json!(["fsdownload", "Desktop"]),
        "what the interface is offered is the folders' own names, in the order they were chosen"
    );

    // Nothing chosen means every download asks, and that reads as an absent folder rather than as an
    // empty name somebody might try to look up.
    let asking = check(
        &DownloadFolderView {
            current: None,
            history: Vec::new(),
        },
        "DownloadFolderView",
    );
    assert_eq!(asking["current"], serde_json::Value::Null);
}

#[test]
fn asking_a_server_for_its_file_answers_with_camel_case_fields() {
    let nothing = check(&SyncDownloadOutcome::NothingYet, "NothingYet");
    assert_eq!(nothing["kind"], serde_json::json!("nothingYet"));

    let asked = check(&SyncDownloadOutcome::NeedsPassphrase, "NeedsPassphrase");
    assert_eq!(asked["kind"], serde_json::json!("needsPassphrase"));

    let read = check(
        &SyncDownloadOutcome::Read {
            catalog: SyncCatalog {
                contents: SyncContents::default(),
                sealed: false,
                exported_at: 1,
                app_version: "0.1.0".to_owned(),
            },
        },
        "Read",
    );
    assert_eq!(read["kind"], serde_json::json!("read"));
    assert_eq!(read["catalog"]["exportedAt"], serde_json::json!(1));
}

#[test]
fn a_selection_carries_its_switches_and_omits_the_lists_nobody_narrowed() {
    // The lists are absent rather than empty, because the two mean different things: absent is
    // "every one there is", and empty would be "none of them".
    let standard = check(&SyncSelection::default(), "SyncSelection");

    assert_eq!(standard["credentials"], serde_json::json!(false));
    assert_eq!(standard["history"], serde_json::json!(false));
    assert_eq!(standard["settings"], serde_json::json!(true));
    assert!(
        standard.get("sessions").is_none(),
        "a list nobody narrowed is not written"
    );

    let narrowed = check(
        &SyncSelection {
            sessions: Some(vec!["session-1".to_owned()]),
            ..SyncSelection::default()
        },
        "SyncSelection",
    );
    assert_eq!(narrowed["sessions"][0], serde_json::json!("session-1"));
}

#[test]
fn an_export_summary_says_whether_a_passphrase_stands_in_front_of_the_file() {
    let written = check(
        &SessionExportSummary {
            sessions: 3,
            groups: 1,
            secrets: 3,
            encrypted: true,
            file_name: "library.json".to_owned(),
        },
        "SessionExportSummary",
    );

    assert_eq!(written["fileName"], serde_json::json!("library.json"));
    assert_eq!(written["encrypted"], serde_json::json!(true));
}

#[test]
fn a_session_with_a_route_and_forwards_serializes_with_camel_case_fields() {
    let session = Session {
        id: "session-1".to_owned(),
        name: "through the hop".to_owned(),
        group_id: Some("group-1".to_owned()),
        protocol: Protocol::Ssh,
        host: "host.example".to_owned(),
        port: 22,
        user: "root".to_owned(),
        auth: SessionAuth {
            method: AuthMethod::PublicKey,
            key: Some(KeyInfo {
                key_ref: Some("key-1".to_owned()),
                display_name: "id_ed25519".to_owned(),
                algorithm: "ssh-ed25519".to_owned(),
                fingerprint: "SHA256:AAAA".to_owned(),
                has_passphrase: true,
            }),
            has_secret: true,
        },
        terminal: TerminalSettings::default(),
        jump: vec!["hop-1".to_owned()],
        forwards: vec![LocalForward {
            id: "f1".to_owned(),
            bind_host: "127.0.0.1".to_owned(),
            bind_port: 0,
            mode: ForwardMode::Local,
            target_host: "db.internal".to_owned(),
            target_port: 5432,
        }],
        updated_at: 1,
    };

    let json = check(&session, "Session");
    assert_eq!(json["jump"], serde_json::json!(["hop-1"]));
    assert_eq!(json["forwards"][0]["targetPort"], serde_json::json!(5432));
    assert_eq!(json["auth"]["key"]["hasPassphrase"], serde_json::json!(true));
}

#[test]
fn a_created_key_carries_its_fingerprint_and_one_public_line() {
    // The one key-shaped thing that crosses this boundary, and only the half that is meant to be
    // copied onto machines. The kind is a short lower-case word — `ed25519`, not `ssh-ed25519` —
    // while the algorithm next to it is the key type OpenSSH spells, because that is what was read
    // out of the key that was made.
    for algorithm in KeyAlgorithm::ALL {
        let json = check(algorithm, "KeyAlgorithm");
        assert_eq!(json, serde_json::json!(algorithm.as_str()));
        assert!(
            json.as_str().is_some_and(|name| !name.contains('-')),
            "the short name is what a dropdown stores: {json}"
        );
    }
    assert_eq!(
        KeyAlgorithm::default(),
        KeyAlgorithm::ALL[0],
        "the first kind is the one to pick"
    );

    let created = check(
        &CreatedKey {
            info: KeyInfo {
                key_ref: Some("key-1".to_owned()),
                display_name: "id_ed25519".to_owned(),
                algorithm: "ssh-ed25519".to_owned(),
                fingerprint: "SHA256:AAAA".to_owned(),
                has_passphrase: false,
            },
            public_key: "ssh-ed25519 AAAA someone@example".to_owned(),
        },
        "CreatedKey",
    );

    assert_eq!(
        created["publicKey"],
        serde_json::json!("ssh-ed25519 AAAA someone@example")
    );
    assert_eq!(created["info"]["displayName"], serde_json::json!("id_ed25519"));
    assert_eq!(created["info"]["hasPassphrase"], serde_json::json!(false));
}

#[test]
fn the_remaining_wire_types_serialize_with_camel_case_fields() {
    check(
        &AppSettings {
            theme_mode: ThemeMode::Dark,
            terminal_theme: ThemeMode::System,
            files_layout: FilesLayout::List,
            session_layout: SessionLayout::Terminal,
            terminal_panel_height: Some(320),
            sidebar_width: Some(300),
            files_tree_width: Some(224),
            commands_library_width: Some(256),
            sidebar_open: false,
            monitor_enabled: true,
            show_tray_icon: false,
            quit_on_close: true,
            background_image: BackgroundImage {
                file_name: Some("wallpaper.png".to_owned()),
                display_mode: BackgroundDisplayMode::Tile,
                transparency: 40,
                blur: 8,
            },
            packed_transfer: true,
            command_completion: false,
            keybindings: BTreeMap::from([("commands.completionAccept".to_owned(), "Mod+Enter".to_owned())]),
            theme_palette: ThemePalette::Sage,
            ui_scale: 125,
            corner_style: CornerStyle::Small,
            home_layout: HomeLayout::Intro,
            app_layout: AppLayout::Classic,
            tab_layout: TabLayout::Scroll,
            tab_placement: TabPlacement::Left,
            tab_group_mode: TabGroupMode::Session,
            tab_sort_mode: TabSortMode::TitleAsc,
            tab_max_width: TabMaxWidth::Compact,
            ui_font: Some("Inter, sans-serif".to_owned()),
            terminal_font: Some("'Fira Code', monospace".to_owned()),
            terminal_font_size: 16,
            custom_colors: Some(CustomColors::light_default()),
            custom_colors_dark: Some(CustomColors::dark_default()),
            locale: Some("zh-CN".to_owned()),
            sync_endpoint: Some("https://dav.example.com/dav/".to_owned()),
            sync_username: Some("alice".to_owned()),
            sync_remote_path: Some("7shell/sync/backup.json".to_owned()),
            sync_include_secrets: true,
            sync_auto_upload: true,
            sync_auto_upload_minutes: 45,
            sync_snippet_provider: Some(SnippetProvider::GitLab),
            sync_snippet_instance: Some("https://gitlab.example.com".to_owned()),
            sync_snippet_ids: BTreeMap::from([("gitlab:https://gitlab.example.com".to_owned(), "42".to_owned())]),
        },
        "AppSettings",
    );
    check(&CustomColors::light_default(), "CustomColors");
    // The corner style is a word on the wire, like the theme: a variant that serialized as `large`
    // when the interface expects `"large"` is what this file is for.
    check(&CornerStyle::Large, "CornerStyle");
    check(&HomeLayout::Workspace, "HomeLayout");
    check(&AppLayout::Classic, "AppLayout");
    check(&TabLayout::Wrap, "TabLayout");
    check(&TabPlacement::Bottom, "TabPlacement");
    check(&TabGroupMode::Session, "TabGroupMode");
    check(&TabSortMode::CreatedAsc, "TabSortMode");
    check(&TabMaxWidth::Wide, "TabMaxWidth");
    check(&BackgroundDisplayMode::Center, "BackgroundDisplayMode");
    check(&BackgroundImage::default(), "BackgroundImage");
    check(&ThemeMode::System, "ThemeMode");
    // Both of the settings the interface offers as words rather than as numbers: a variant that
    // serialized as `default` when the interface expects `"default"` is the bug this file is for.
    check(&ThemePalette::Claude, "ThemePalette");
    check(&ThemePalette::Default, "ThemePalette");
    check(&VaultState::Unlocked, "VaultState");
    check(
        &SessionGroup {
            id: "g1".to_owned(),
            name: "work".to_owned(),
            parent_id: Some("g0".to_owned()),
            order: 1,
            pinned: true,
        },
        "SessionGroup",
    );
    check(&ForwardMode::Dynamic, "ForwardMode");
    check(&trusted(), "KnownHostEntry");
    check(&presented(), "PresentedHostKey");
    check(&TerminalSettings::default(), "TerminalSettings");
}

/// A batch result is the one the interface reads row by row, so its shape is checked field by
/// field rather than only for camelCase: a row missing its exit code reads as still running, and a
/// row that lost its failure reason reads as a command that worked.
#[test]
fn a_batch_result_serializes_with_camel_case_fields() {
    let request = check(
        &BatchRequest {
            session_ids: vec!["session-1".to_owned(), "session-2".to_owned()],
            command: "uptime".to_owned(),
            timeout_ms: 30_000,
            parallel: 4,
            acknowledged: true,
        },
        "BatchRequest",
    );
    assert_eq!(request["sessionIds"][1], serde_json::json!("session-2"));
    assert_eq!(request["timeoutMs"], serde_json::json!(30_000));

    let approval = check(
        &BatchOutcome::ApprovalRequired {
            risk: RiskAssessment::from_reasons(vec![RiskReason::new(
                "recursive-force-delete",
                "risk.recursiveForceDelete",
                RiskLevel::Destructive,
            )]),
        },
        "ApprovalRequired",
    );
    assert_eq!(approval["kind"], serde_json::json!("approvalRequired"));
    assert_eq!(approval["risk"]["level"], serde_json::json!("destructive"));
    assert_eq!(
        approval["risk"]["reasons"][0]["ruleId"],
        serde_json::json!("recursive-force-delete")
    );
    assert_eq!(
        approval["risk"]["reasons"][0]["labelKey"],
        serde_json::json!("risk.recursiveForceDelete")
    );

    let unknown = check(
        &BatchOutcome::HostKeyUnknown {
            host: BatchHostRef {
                session_id: "session-2".to_owned(),
                session_name: "web-2".to_owned(),
            },
            key: presented(),
        },
        "HostKeyUnknown",
    );
    assert_eq!(unknown["kind"], serde_json::json!("hostKeyUnknown"));
    assert_eq!(
        unknown["host"]["sessionName"],
        serde_json::json!("web-2"),
        "the decision says which host raised it"
    );

    let finished = check(
        &BatchOutcome::Finished {
            command: "uptime".to_owned(),
            risk: RiskAssessment::safe(),
            results: vec![
                BatchHostResult {
                    session_id: "session-1".to_owned(),
                    session_name: "web-1".to_owned(),
                    outcome: BatchHostOutcome::Ran {
                        exit_code: Some(0),
                        stdout: "up 3 days\n".to_owned(),
                        stderr: String::new(),
                    },
                    duration_ms: 120,
                },
                BatchHostResult {
                    session_id: "session-2".to_owned(),
                    session_name: "web-2".to_owned(),
                    outcome: BatchHostOutcome::Failed {
                        error: HostFailure {
                            code: "SSH-TIMEOUT".to_owned(),
                            message_key: "errors.ssh.timeout".to_owned(),
                            message_params: BTreeMap::from([("seconds".to_owned(), "30".to_owned())]),
                            detail: None,
                        },
                    },
                    duration_ms: 30_000,
                },
            ],
            duration_ms: 30_100,
        },
        "Finished",
    );

    assert_eq!(finished["kind"], serde_json::json!("finished"));
    assert_eq!(finished["results"][0]["sessionName"], serde_json::json!("web-1"));
    assert_eq!(finished["results"][0]["outcome"]["kind"], serde_json::json!("ran"));
    assert_eq!(finished["results"][0]["outcome"]["exitCode"], serde_json::json!(0));
    assert_eq!(finished["results"][1]["outcome"]["kind"], serde_json::json!("failed"));
    assert_eq!(
        finished["results"][1]["outcome"]["error"]["code"],
        serde_json::json!("SSH-TIMEOUT")
    );
    assert_eq!(
        finished["results"][1]["outcome"]["error"]["messageParams"]["seconds"],
        serde_json::json!("30")
    );
    assert_eq!(finished["durationMs"], serde_json::json!(30_100));
}

/// A host sample is read section by section, and the sections that a host did not answer are
/// `null` rather than absent: the panel draws every row it has a place for, so the shape of a
/// sample has to be the same whether the host is a Linux box or one that answered nothing at all.
#[test]
fn a_host_sample_serializes_with_camel_case_fields_and_keeps_its_empty_sections() {
    let sample = check(
        &HostMetrics {
            sampled_at: 1_700_000_000_000,
            round_trip_ms: 37,
            hostname: Some("web-1".to_owned()),
            os: Some("Ubuntu 22.04.3 LTS".to_owned()),
            kernel: Some("5.15.0-91-generic".to_owned()),
            arch: Some("x86_64".to_owned()),
            cores: Some(4),
            cpu_model: Some(CpuModel {
                name: "Intel(R) Core(TM) i9-14900HX".to_owned(),
                frequency_mhz: Some(2419.198),
                cache_kb: Some(36_864),
                bogomips: Some(4838.39),
            }),
            uptime_seconds: Some(123_456.78),
            load: Some(LoadAverage {
                one: 0.02,
                five: 0.01,
                fifteen: 0.0,
            }),
            cpu: Some(CpuCounters {
                total_ticks: 42_210,
                idle_ticks: 40_500,
                states: CpuStates {
                    user_ticks: 1_000,
                    nice_ticks: 200,
                    system_ticks: 300,
                    idle_ticks: 40_000,
                    io_wait_ticks: 500,
                    interrupt_ticks: 60,
                    soft_interrupt_ticks: 70,
                    steal_ticks: 80,
                },
            }),
            memory: Some(MemoryUsage {
                total_bytes: 8_009_984 * 1024,
                available_bytes: 6_543_210 * 1024,
                swap_total_bytes: 4_194_304 * 1024,
                swap_free_bytes: 4_194_304 * 1024,
            }),
            network: vec![NetworkCounters {
                name: "eth0".to_owned(),
                received_bytes: 12_345_678,
                transmitted_bytes: 2_345_678,
            }],
            disks: vec![DiskUsage {
                mount_point: "/".to_owned(),
                filesystem: "overlay".to_owned(),
                size_bytes: 97_552_384 * 1024,
                available_bytes: 57_770_936 * 1024,
            }],
            processes: vec![ProcessSample {
                pid: 1,
                user: "root".to_owned(),
                name: "systemd".to_owned(),
                command: "/sbin/init splash".to_owned(),
                executable: None,
                cpu_percent: 0.7,
                memory_percent: 1.2,
                resident_bytes: 11_878 * 1024,
                cpu_ticks: None,
            }],
        },
        "HostMetrics",
    );

    // The names the interface reads, spelled exactly as it reads them.
    assert_eq!(sample["sampledAt"], serde_json::json!(1_700_000_000_000_u64));
    assert_eq!(sample["roundTripMs"], serde_json::json!(37));
    assert_eq!(sample["uptimeSeconds"], serde_json::json!(123_456.78));
    assert_eq!(sample["load"]["one"], serde_json::json!(0.02));
    assert_eq!(sample["cpu"]["totalTicks"], serde_json::json!(42_210));
    assert_eq!(sample["cpu"]["idleTicks"], serde_json::json!(40_500));
    assert_eq!(
        sample["memory"]["availableBytes"],
        serde_json::json!(6_543_210_u64 * 1024)
    );
    assert_eq!(
        sample["memory"]["swapFreeBytes"],
        serde_json::json!(4_194_304_u64 * 1024)
    );
    assert_eq!(sample["network"][0]["receivedBytes"], serde_json::json!(12_345_678));
    assert_eq!(sample["network"][0]["transmittedBytes"], serde_json::json!(2_345_678));
    assert_eq!(sample["disks"][0]["mountPoint"], serde_json::json!("/"));
    assert_eq!(
        sample["disks"][0]["sizeBytes"],
        serde_json::json!(97_552_384_u64 * 1024)
    );
    assert_eq!(sample["processes"][0]["cpuPercent"], serde_json::json!(0.7));
    assert_eq!(sample["processes"][0]["memoryPercent"], serde_json::json!(1.2));
    assert_eq!(
        sample["processes"][0]["residentBytes"],
        serde_json::json!(11_878 * 1024)
    );
    assert_eq!(sample["processes"][0]["user"], serde_json::json!("root"));
    assert_eq!(
        sample["processes"][0]["command"],
        serde_json::json!("/sbin/init splash")
    );

    // The CPU page reads the breakdown; the panel reads the two numbers above it.
    assert_eq!(
        sample["cpuModel"]["name"],
        serde_json::json!("Intel(R) Core(TM) i9-14900HX")
    );
    assert_eq!(sample["cpuModel"]["frequencyMhz"], serde_json::json!(2419.198));
    assert_eq!(sample["cpuModel"]["cacheKb"], serde_json::json!(36_864));
    assert_eq!(sample["cpu"]["states"]["ioWaitTicks"], serde_json::json!(500));
    assert_eq!(sample["cpu"]["states"]["softInterruptTicks"], serde_json::json!(70));
    assert_eq!(sample["cpu"]["states"]["stealTicks"], serde_json::json!(80));

    let nothing = check(
        &HostMetrics {
            sampled_at: 0,
            round_trip_ms: 0,
            hostname: None,
            os: None,
            kernel: None,
            arch: None,
            cores: None,
            cpu_model: None,
            uptime_seconds: None,
            load: None,
            cpu: None,
            memory: None,
            network: Vec::new(),
            disks: Vec::new(),
            processes: Vec::new(),
        },
        "HostMetrics (nothing read)",
    );

    assert!(
        nothing["load"].is_null(),
        "an unread section is null, not a missing key"
    );
    assert!(nothing["memory"].is_null());
    assert!(nothing["cpuModel"].is_null());
    assert_eq!(nothing["network"], serde_json::json!([]));
    assert_eq!(nothing["disks"], serde_json::json!([]));
}

/// A host's own account of itself, in both the shapes the interface has to tell apart: a machine
/// that named its distribution, and one that answered nothing at all. A mark in the session list is
/// drawn from this, so a silent host arrives as empty fields — never as a missing object, and never
/// as a value somebody downstream has to guess at.
#[test]
fn a_host_identity_serializes_with_camel_case_fields_and_keeps_silence_as_empty_fields() {
    let system = check(
        &HostSystem {
            name: Some("Ubuntu 24.04.1 LTS".to_owned()),
            kernel: Some("Linux".to_owned()),
            distro: Some("ubuntu".to_owned()),
            distro_like: vec!["debian".to_owned()],
        },
        "HostSystem",
    );

    assert_eq!(system["distro"], serde_json::json!("ubuntu"));
    assert_eq!(system["distroLike"], serde_json::json!(["debian"]));
    assert_eq!(system["kernel"], serde_json::json!("Linux"));

    let silent = check(&HostSystem::default(), "HostSystem (nothing answered)");

    assert!(
        silent["name"].is_null(),
        "a host that did not say is null, not a missing key"
    );
    assert!(silent["distro"].is_null());
    assert_eq!(silent["distroLike"], serde_json::json!([]));
}

/// The two shapes the process page reads: a row with everything a read asked for, and the whole of
/// one process — including the environment, which is the only place in this build that carries one.
#[test]
fn a_process_page_and_a_process_detail_serialize_with_camel_case_fields() {
    let page = check(
        &ProcessPage {
            ticks_per_second: 100,
            processes: vec![ProcessSample {
                pid: 42,
                user: "root".to_owned(),
                name: "top".to_owned(),
                command: "top -b -d 1".to_owned(),
                executable: Some("/usr/bin/top".to_owned()),
                cpu_percent: 3.5,
                memory_percent: 0.6,
                resident_bytes: 6_656_000,
                cpu_ticks: Some(1_000),
            }],
        },
        "ProcessPage",
    );

    assert_eq!(page["ticksPerSecond"], serde_json::json!(100));
    assert_eq!(page["processes"][0]["cpuTicks"], serde_json::json!(1_000));
    assert_eq!(page["processes"][0]["executable"], serde_json::json!("/usr/bin/top"));

    let detail = check(
        &ProcessDetail {
            pid: 42,
            name: "top".to_owned(),
            command_line: "top -b -d 1".to_owned(),
            executable: "/usr/bin/top".to_owned(),
            working_directory: "/root".to_owned(),
            environment: vec![EnvironmentEntry {
                name: "PATH".to_owned(),
                value: "/usr/bin:/bin".to_owned(),
            }],
        },
        "ProcessDetail",
    );

    assert_eq!(detail["commandLine"], serde_json::json!("top -b -d 1"));
    assert_eq!(detail["workingDirectory"], serde_json::json!("/root"));
    assert_eq!(detail["environment"][0]["name"], serde_json::json!("PATH"));
}

/// A socket row is read column by column on the network page, and two of its facts are optional in
/// a way that matters: a socket nobody will attribute has no process, and a tool that reports no
/// byte counters has no bytes — which the page draws as a dash rather than as a zero.
#[test]
fn a_socket_row_serializes_with_camel_case_fields() {
    let listener = check(
        &SocketRow {
            protocol: "tcp".to_owned(),
            state: "LISTEN".to_owned(),
            address: "0.0.0.0".to_owned(),
            port: 22,
            pid: 1741,
            process: "sshd".to_owned(),
            peer_address: String::new(),
            peer_port: 0,
            sent_bytes: Some(1400),
            received_bytes: Some(560),
        },
        "SocketRow",
    );

    assert_eq!(listener["state"], serde_json::json!("LISTEN"));
    assert_eq!(listener["peerAddress"], serde_json::json!(""));
    assert_eq!(listener["sentBytes"], serde_json::json!(1400));

    let unattributed = check(
        &SocketRow {
            protocol: "udp".to_owned(),
            state: "UNCONN".to_owned(),
            address: "::".to_owned(),
            port: 43449,
            pid: 0,
            process: String::new(),
            peer_address: "192.168.254.1".to_owned(),
            peer_port: 52344,
            sent_bytes: None,
            received_bytes: None,
        },
        "SocketRow (nothing attributed)",
    );

    assert!(
        unattributed["sentBytes"].is_null(),
        "a host that reports no counters is null, not zero"
    );
    assert_eq!(unattributed["peerPort"], serde_json::json!(52344));
}

/// What a saved command looks like once it has been through JSON, and what a send reports.
///
/// The slot history is checked field by field rather than only for camelCase: it is keyed by
/// position, and a shape that flattened it would still serialize — into something the interface
/// could not put a value back into.
#[test]
fn a_saved_command_serializes_with_camel_case_fields() {
    let snippet = check(
        &Snippet {
            id: "snippet-1".to_owned(),
            group_id: "default".to_owned(),
            name: "restart nginx".to_owned(),
            prefix: "res".to_owned(),
            command: "systemctl restart #1".to_owned(),
            append_cr: true,
            enabled: false,
            order: 3,
            slot_history: vec![vec!["nginx".to_owned(), "apache2".to_owned()], Vec::new()],
        },
        "Snippet",
    );
    assert_eq!(snippet["groupId"], serde_json::json!("default"));
    assert_eq!(snippet["appendCr"], serde_json::json!(true));
    assert_eq!(snippet["slotHistory"][0][1], serde_json::json!("apache2"));
    assert_eq!(snippet["slotHistory"][1], serde_json::json!([]));

    let group = check(
        &SnippetGroup {
            id: "default".to_owned(),
            name: "default".to_owned(),
            order: 0,
            commands: Vec::new(),
        },
        "SnippetGroup",
    );
    assert_eq!(group["order"], serde_json::json!(0));

    check(
        &CommandHistoryEntry {
            command: "uptime".to_owned(),
            at: 1_700_000_000,
        },
        "CommandHistoryEntry",
    );

    let sent = check(
        &SendOutcome::Sent {
            report: SendReport {
                sent: vec!["t1".to_owned()],
                failures: vec![SendFailure {
                    handle: "t2".to_owned(),
                    code: "SESSION-NOT-FOUND".to_owned(),
                    message_key: "errors.session.notFound".to_owned(),
                    message_params: BTreeMap::from([("sessionId".to_owned(), "t2".to_owned())]),
                    detail: None,
                }],
            },
        },
        "Sent",
    );
    assert_eq!(sent["kind"], serde_json::json!("sent"));
    assert_eq!(sent["report"]["sent"][0], serde_json::json!("t1"));
    assert_eq!(
        sent["report"]["failures"][0]["messageKey"],
        serde_json::json!("errors.session.notFound")
    );

    let target = check(
        &SendTarget::Terminals {
            handles: vec!["t1".to_owned(), "t2".to_owned()],
        },
        "SendTarget",
    );
    assert_eq!(target["kind"], serde_json::json!("terminals"));
    assert_eq!(target["handles"][1], serde_json::json!("t2"));
}

/// One word somebody added, once it has been through JSON.
///
/// The note is checked as text rather than only for its key: it is the one field here somebody
/// writes in their own words, and a shape that renamed it would be a row whose explanation had
/// gone missing in the interface and nowhere else.
#[test]
fn a_completion_word_serializes_with_camel_case_fields() {
    let word = check(
        &CompletionWord {
            id: "word-1".to_owned(),
            command: "kubectl".to_owned(),
            word: "apply".to_owned(),
            note: "put the manifest on the cluster".to_owned(),
        },
        "CompletionWord",
    );

    assert_eq!(word["id"], serde_json::json!("word-1"));
    assert_eq!(word["note"], serde_json::json!("put the manifest on the cluster"));
    assert_eq!(word.as_object().expect("an object").len(), 4);
}

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

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sevenshell_types::{
    AgentOrigin, AiContextSummary, AiProviderConfig, AiProviderKind, AiProviderView, AiSessionDetail, AiSessionSummary,
    AiToolOutcome, AppLayout, AppSettings, AuthMethod, BackgroundDisplayMode, BackgroundImage, BatchHostOutcome,
    BatchHostRef, BatchHostResult, BatchOutcome, BatchRequest, CommandHistoryEntry, CompletionExecuteResult,
    CompletionWord, ContainerDetail, ContainerMount, ContainerPort, ContainerRuntime, ContainerSettings,
    ContainerSummary, ContainerTransport, CornerStyle, CpuCounters, CpuModel, CpuStates, CreatedKey, CustomColors,
    DiskUsage, DownloadFolderView, EnvironmentEntry, FilesLayout, ForwardMode, HomeLayout, HostFailure, HostMetrics,
    HostSystem, K8sMethod, K8sRequest, K8sResponse, KeyAlgorithm, KeyInfo, KnownHostEntry, KubeContextInfo,
    KubeconfigSource, KubernetesSettings, LoadAverage, LocalForward, McpAccessPolicy, McpPrompt, McpPromptArgument,
    McpPromptMessage, McpPromptRole, McpPromptText, McpResource, McpResourceContent, McpResourceRead, McpScope,
    McpServerConfig, McpServerContent, McpServerState, McpServerStatus, McpServerTransport, McpToolPolicy, MemoryUsage,
    NetworkCounters, OpenOutcome, OpenTunnel, PolicyAction, PolicyMatrix, PolicyRule, PresentedHostKey, ProcessDetail,
    ProcessPage, ProcessSample, Protocol, RiskAssessment, RiskLevel, RiskReason, SendFailure, SendOutcome, SendReport,
    SendTarget, Session, SessionAuth, SessionExportSummary, SessionGroup, SessionImportOutcome, SessionImportSummary,
    SessionLayout, SessionReopen, SftpPasteItem, SftpPasteOutcome, Snippet, SnippetGroup, SnippetProvider,
    SnippetStatus, SnippetUploadReceipt, SocketRow, SyncApplySummary, SyncCatalog, SyncContents, SyncDownloadOutcome,
    SyncInspectOutcome, SyncReceipt, SyncSelection, SyncServiceStatus, SyncUploadReceipt, TabGroupMode, TabLayout,
    TabMaxWidth, TabPlacement, TabSortMode, TerminalCursorInactiveStyle, TerminalCursorStyle, TerminalCustomScheme,
    TerminalHighlightMode, TerminalHighlightRule, TerminalSchemeBackground, TerminalSchemeProfile,
    TerminalSchemeSource, TerminalSettings, ThemeMode, ThemePalette, TunnelFailure, UpdateCheck, UpdateProgress,
    UploadAnswer, UploadOutcome, VaultArchive, VaultState,
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
        desktop: sevenshell_types::DesktopSettings::default(),
        telnet: sevenshell_types::TelnetSettings::default(),
        serial: sevenshell_types::SerialSettings::default(),
        local: sevenshell_types::LocalSettings::default(),
        wsl: sevenshell_types::WslSettings::default(),
        container: sevenshell_types::ContainerSettings::default(),
        kubernetes: sevenshell_types::KubernetesSettings::default(),
        webdav: sevenshell_types::WebDavSettings {
            url: "https://dav.example/folder/".to_owned(),
        },
        ftp: sevenshell_types::FtpSettings { secure: true },
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
    assert_eq!(json["desktop"]["width"], serde_json::json!(1280));
    assert_eq!(json["desktop"]["redirectDrives"], serde_json::json!(true));
    assert_eq!(json["webdav"]["url"], serde_json::json!("https://dav.example/folder/"));
    assert_eq!(json["ftp"]["secure"], serde_json::json!(true));
}

#[test]
fn desktop_settings_serialize_with_camel_case_fields() {
    let json = check(
        &sevenshell_types::DesktopSettings {
            width: 1920,
            height: 1080,
            fullscreen: true,
            redirect_drives: false,
        },
        "DesktopSettings",
    );

    assert_eq!(json["width"], serde_json::json!(1920));
    assert_eq!(json["height"], serde_json::json!(1080));
    assert_eq!(json["fullscreen"], serde_json::json!(true));
    assert_eq!(json["redirectDrives"], serde_json::json!(false));
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
    let settings = check(
        &AppSettings {
            theme_mode: ThemeMode::Dark,
            terminal_theme: ThemeMode::System,
            files_layout: FilesLayout::List,
            session_layout: SessionLayout::Terminal,
            session_reopen: SessionReopen::NewTab,
            terminal_panel_height: Some(320),
            sidebar_width: Some(300),
            files_tree_width: Some(224),
            commands_library_width: Some(256),
            sidebar_open: false,
            monitor_enabled: true,
            show_tray_icon: false,
            quit_on_close: true,
            update_check_on_startup: false,
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
            ignored_update_version: Some("0.9.9".to_owned()),
            terminal_font: Some("'Fira Code', monospace".to_owned()),
            terminal_font_size: 16,
            terminal_font_weight: 500,
            terminal_bold_font_weight: 800,
            terminal_line_height: 1.25,
            terminal_letter_spacing: 1,
            terminal_padding_x: 14,
            terminal_padding_y: 9,
            terminal_cursor_style: TerminalCursorStyle::Bar,
            terminal_cursor_blink: false,
            terminal_cursor_inactive_style: TerminalCursorInactiveStyle::None,
            terminal_bold_bright: false,
            terminal_min_contrast: 4.5,
            terminal_scheme_source: TerminalSchemeSource::Custom,
            terminal_scheme_dark: Some("dracula".to_owned()),
            terminal_scheme_light: Some("solarized-light".to_owned()),
            terminal_scheme_background: TerminalSchemeBackground::Host,
            terminal_custom_schemes: vec![TerminalCustomScheme {
                id: "imported".to_owned(),
                name: "Imported".to_owned(),
                foreground: "#ffffff".to_owned(),
                background: "#000000".to_owned(),
                cursor: "#ffffff".to_owned(),
                colors: (0..16).map(|_| "#123456".to_owned()).collect(),
                selection_background: Some("#654321".to_owned()),
            }],
            terminal_scheme_profiles: vec![TerminalSchemeProfile {
                id: "mine".to_owned(),
                name: "Mine".to_owned(),
                terminal_theme: ThemeMode::Dark,
                scheme_source: TerminalSchemeSource::Custom,
                scheme_dark: Some("dracula".to_owned()),
                scheme_light: Some("solarized-light".to_owned()),
                scheme_background: TerminalSchemeBackground::Scheme,
            }],
            terminal_highlight_rules: vec![TerminalHighlightRule {
                id: "timeout".to_owned(),
                pattern: "timeout".to_owned(),
                color: "#f59e0b".to_owned(),
                is_regex: false,
                case_sensitive: false,
            }],
            terminal_highlight_mode: TerminalHighlightMode::Text,
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
            mcp_enabled: true,
            mcp_port: 4711,
            mcp_scope: McpScope::SafeWrite,
            mcp_approval_timeout_seconds: 90,
            mcp_audit_retention_days: 14,
            mcp_max_result_bytes: 131_072,
            mcp_tool_policy: McpToolPolicy {
                disabled_tools: BTreeSet::from(["sshell_exec_run".to_owned()]),
            },
            mcp_access_policy: McpAccessPolicy {
                visible_session_ids: Some(BTreeSet::from(["s1".to_owned()])),
                session_scopes: BTreeMap::from([("s1".to_owned(), McpScope::HighRisk)]),
                group_scopes: BTreeMap::from([("g1".to_owned(), McpScope::ReadOnly)]),
            },
            policy_matrix: PolicyMatrix {
                global: PolicyRule {
                    safe: PolicyAction::Allow,
                    caution: PolicyAction::Ask,
                    destructive: PolicyAction::Deny,
                    catastrophic: PolicyAction::Deny,
                },
                groups: BTreeMap::from([("g1".to_owned(), PolicyRule::default())]),
                hosts: BTreeMap::new(),
                directories: BTreeMap::new(),
            },
            ai_providers: vec![AiProviderConfig {
                id: "local".to_owned(),
                name: "Ollama".to_owned(),
                kind: AiProviderKind::Local,
                base_url: "http://127.0.0.1:11434/v1".to_owned(),
                model: "llama3.1".to_owned(),
                max_output_tokens: 2_048,
            }],
            ai_default_provider_id: Some("local".to_owned()),
            ai_context_enabled: false,
            ai_context_lines: 40,
            ai_session_retention_days: 45,
            mcp_client_servers: vec![McpServerConfig {
                id: "files".to_owned(),
                name: "Files".to_owned(),
                transport: McpServerTransport::Stdio,
                command: Some("npx".to_owned()),
                args: vec!["-y".to_owned(), "@modelcontextprotocol/server-filesystem".to_owned()],
                url: None,
                enabled: true,
                tool_timeout_seconds: 60,
            }],
        },
        "AppSettings",
    );
    assert_eq!(
        settings["aiSessionRetentionDays"],
        serde_json::json!(45),
        "the retention is a page field like the two above it, and the window reads this name"
    );
    // The terminal's typography travels under the names the page reads, and the two cursor shapes
    // are the lower-case words the menu matches.
    assert_eq!(settings["terminalLineHeight"], serde_json::json!(1.25));
    assert_eq!(settings["terminalCursorStyle"], serde_json::json!("bar"));
    assert_eq!(settings["terminalCursorInactiveStyle"], serde_json::json!("none"));
    assert_eq!(settings["terminalFontWeight"], serde_json::json!(500));
    assert_eq!(settings["terminalSchemeSource"], serde_json::json!("custom"));
    assert_eq!(settings["terminalSchemeDark"], serde_json::json!("dracula"));
    assert_eq!(settings["terminalSchemeBackground"], serde_json::json!("host"));
    assert_eq!(
        settings["terminalCustomSchemes"][0]["selectionBackground"],
        serde_json::json!("#654321"),
        "an imported scheme carries its surfaces under the names the catalogue uses"
    );
    assert_eq!(
        settings["terminalSchemeProfiles"][0]["terminalTheme"],
        serde_json::json!("dark")
    );
    assert_eq!(
        settings["terminalSchemeProfiles"][0]["schemeDark"],
        serde_json::json!("dracula")
    );
    assert_eq!(
        settings["terminalHighlightRules"][0]["isRegex"],
        serde_json::json!(false)
    );
    assert_eq!(settings["terminalHighlightMode"], serde_json::json!("text"));
    // An external server travels as configuration only: the transport is the lower-case word the
    // page matches, and the timeout is the camelCase name the row reads.
    let external = &settings["mcpClientServers"][0];
    assert_eq!(external["transport"], serde_json::json!("stdio"));
    assert_eq!(external["toolTimeoutSeconds"], serde_json::json!(60));
    assert_eq!(external["args"][0], serde_json::json!("-y"));
    assert!(
        external.get("url").is_none(),
        "an address nobody set is absent rather than null"
    );
    for forbidden in ["env", "headers", "token", "secret", "password"] {
        assert!(
            external.get(forbidden).is_none(),
            "`{forbidden}` would put a credential in a file anything can read: {external}"
        );
    }
    // The scope is a word on the wire, and it is the camelCase one the interface's union spells:
    // `read_only` here would be a value the settings page cannot match against anything.
    let scope = check(&McpScope::HighRisk, "McpScope");
    assert_eq!(scope, serde_json::json!("highRisk"));

    // The assistant: who is asking, and a provider that carries no key of its own.
    let origin = check(&AgentOrigin::Ai, "AgentOrigin");
    assert_eq!(origin, serde_json::json!("ai"));
    let kind = check(&AiProviderKind::OpenAi, "AiProviderKind");
    assert_eq!(kind, serde_json::json!("openai"));
    // The one kind whose wire name is not lower case: the settings page matches this value, so a
    // variant that serialized as `openairesponses` would draw as a protocol nobody chose.
    let responses = check(&AiProviderKind::OpenAiResponses, "AiProviderKind");
    assert_eq!(responses, serde_json::json!("openaiResponses"));
    let provider = AiProviderConfig::new(
        "hosted",
        "Hosted",
        AiProviderKind::Anthropic,
        "https://api.anthropic.com",
        "claude-sonnet-4-5",
    );
    let config = check(&provider, "AiProviderConfig");
    assert_eq!(config["baseUrl"], serde_json::json!("https://api.anthropic.com"));
    assert_eq!(config["maxOutputTokens"], serde_json::json!(4_096));
    assert!(
        config.get("hasKey").is_none(),
        "a configuration says nothing about a key, and the view below is where that question is asked"
    );
    let view = check(&AiProviderView::new(provider, None), "AiProviderView");
    assert_eq!(
        view["id"],
        serde_json::json!("hosted"),
        "the view is the config plus one fact"
    );
    assert!(
        view["hasKey"].is_null(),
        "a locked vault could not ask, which is null rather than false"
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
    // The remembered answer about a session that is already open: `new-tab` and not `newTab`, which
    // is the shape the interface compares against.
    check(&SessionReopen::NewTab, "SessionReopen");
    check(&TabMaxWidth::Wide, "TabMaxWidth");
    check(&BackgroundDisplayMode::Center, "BackgroundDisplayMode");
    check(&BackgroundImage::default(), "BackgroundImage");
    check(&ThemeMode::System, "ThemeMode");
    // Both of the settings the interface offers as words rather than as numbers: a variant that
    // serialized as `default` when the interface expects `"default"` is the bug this file is for.
    check(&ThemePalette::Claude, "ThemePalette");
    check(&ThemePalette::Default, "ThemePalette");
    check(&VaultState::Unlocked, "VaultState");
    let archived = check(
        &VaultArchive {
            name: "vault.json.old".to_owned(),
            modified_at: 1,
            size_bytes: 2048,
        },
        "VaultArchive",
    );
    // The two fields the archive list reads by: snake_case here draws an empty column, not an error.
    assert_eq!(archived["modifiedAt"], serde_json::json!(1));
    assert_eq!(archived["sizeBytes"], serde_json::json!(2048));
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
            source: "host:web".to_owned(),
        },
        "ApprovalRequired",
    );
    assert_eq!(approval["kind"], serde_json::json!("approvalRequired"));
    assert_eq!(approval["risk"]["level"], serde_json::json!("destructive"));
    assert_eq!(approval["source"], serde_json::json!("host:web"));
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

/// What the update channel answered, in each of its three shapes.
///
/// The optional fields are checked absent as well as present: the interface draws a release note
/// only when there is one, and a serialized `null` would be a note saying nothing where the answer
/// was "the channel did not say".
#[test]
fn an_update_answer_serializes_with_camel_case_fields() {
    let up_to_date = check(
        &UpdateCheck::UpToDate {
            current_version: "0.1.0".to_owned(),
        },
        "UpdateCheck::UpToDate",
    );
    assert_eq!(up_to_date["kind"], serde_json::json!("upToDate"));
    assert_eq!(up_to_date["currentVersion"], serde_json::json!("0.1.0"));

    let available = check(
        &UpdateCheck::Available {
            current_version: "0.1.0".to_owned(),
            release: sevenshell_types::AvailableRelease {
                version: "0.1.0-rc.2".to_owned(),
                notes: Some("the second candidate".to_owned()),
                published_at: Some(1_780_000_000),
            },
        },
        "UpdateCheck::Available",
    );
    assert_eq!(available["kind"], serde_json::json!("available"));
    assert_eq!(available["release"]["version"], serde_json::json!("0.1.0-rc.2"));
    assert_eq!(available["release"]["publishedAt"], serde_json::json!(1_780_000_000));

    let quiet = check(
        &UpdateCheck::Available {
            current_version: "0.1.0".to_owned(),
            release: sevenshell_types::AvailableRelease {
                version: "0.1.0-rc.2".to_owned(),
                notes: None,
                published_at: None,
            },
        },
        "UpdateCheck::Available (no notes)",
    );
    assert!(
        quiet["release"].get("notes").is_none(),
        "a release the channel did not describe carries no notes key at all"
    );

    let progress = check(
        &UpdateProgress::Downloading {
            downloaded: 1024,
            total: None,
        },
        "UpdateProgress::Downloading",
    );
    assert_eq!(progress["kind"], serde_json::json!("downloading"));
    assert_eq!(progress["downloaded"], serde_json::json!(1024));
    assert!(progress["total"].is_null(), "no announced size is a null, not a guess");

    let installing = check(&UpdateProgress::Installing, "UpdateProgress::Installing");
    assert_eq!(installing["kind"], serde_json::json!("installing"));
}

#[test]
fn completion_results_serialize_with_camel_case_fields() {
    let result = check(
        &CompletionExecuteResult {
            exit_code: Some(0),
            stdout: "main\n".to_owned(),
            stderr: String::new(),
            truncated: false,
            timed_out: false,
        },
        "CompletionExecuteResult",
    );
    assert_eq!(
        result,
        serde_json::json!({
            "exitCode": 0,
            "stdout": "main\n",
            "stderr": "",
            "truncated": false,
            "timedOut": false,
        })
    );

    let silent = check(
        &CompletionExecuteResult {
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            truncated: true,
            timed_out: true,
        },
        "CompletionExecuteResult::timedOut",
    );
    assert!(
        silent["exitCode"].is_null(),
        "a generator that ran out of time has no exit status"
    );
}

#[test]
fn a_serial_session_names_every_line_parameter_on_the_wire() {
    let json = check(
        &sevenshell_types::SerialSettings {
            port: "COM3".to_owned(),
            baud_rate: 9_600,
            data_bits: 7,
            parity: sevenshell_types::SerialParity::Even,
            stop_bits: sevenshell_types::SerialStopBits::Two,
            flow_control: sevenshell_types::SerialFlowControl::RtsCts,
            backspace: sevenshell_types::BackspaceMode::CtrlH,
        },
        "SerialSettings",
    );
    assert_eq!(json["port"], serde_json::json!("COM3"));
    assert_eq!(json["baudRate"], serde_json::json!(9600));
    assert_eq!(json["dataBits"], serde_json::json!(7));
    assert_eq!(json["parity"], serde_json::json!("even"));
    assert_eq!(json["stopBits"], serde_json::json!("2"));
    assert_eq!(json["flowControl"], serde_json::json!("rtsCts"));
    assert_eq!(json["backspace"], serde_json::json!("ctrlH"));
}

#[test]
fn telnet_and_local_settings_serialize_as_short_stable_words() {
    let telnet = check(
        &sevenshell_types::TelnetSettings {
            enter: sevenshell_types::EnterMode::Lf,
            backspace: sevenshell_types::BackspaceMode::CtrlH,
        },
        "TelnetSettings",
    );
    assert_eq!(telnet["enter"], serde_json::json!("lf"));
    assert_eq!(telnet["backspace"], serde_json::json!("ctrlH"));

    let local = check(
        &sevenshell_types::LocalSettings {
            shell: "git-bash".to_owned(),
            args: "--noprofile".to_owned(),
        },
        "LocalSettings",
    );
    assert_eq!(local["shell"], serde_json::json!("git-bash"));
    assert_eq!(local["args"], serde_json::json!("--noprofile"));

    let wsl = check(
        &sevenshell_types::WslSettings {
            distro: "Ubuntu".to_owned(),
            args: String::new(),
        },
        "WslSettings",
    );
    assert_eq!(wsl["distro"], serde_json::json!("Ubuntu"));
}

#[test]
fn a_stored_conversation_serializes_with_camel_case_fields() {
    let summary = check(
        &AiSessionSummary {
            id: "session-1".to_owned(),
            name: "why is nginx 502".to_owned(),
            created_at: 1_700_000_000,
            updated_at: 1_700_000_100,
            message_count: 4,
            has_summary: true,
        },
        "AiSessionSummary",
    );
    assert_eq!(summary["messageCount"], serde_json::json!(4));
    assert_eq!(summary["hasSummary"], serde_json::json!(true));

    let detail = check(
        &AiSessionDetail {
            summary: AiSessionSummary {
                id: "session-1".to_owned(),
                name: "why is nginx 502".to_owned(),
                created_at: 1,
                updated_at: 2,
                message_count: 1,
                has_summary: true,
            },
            messages: vec![serde_json::json!({
                "role": "assistant",
                "blocks": [{ "type": "toolUse", "id": "call-1", "name": "sshell_exec_run", "input": {} }],
            })],
            outcomes: BTreeMap::from([(
                "call-1".to_owned(),
                AiToolOutcome {
                    call_id: "call-1".to_owned(),
                    ok: true,
                    code: None,
                    truncated: false,
                    duration_ms: 12,
                },
            )]),
            context_summary: Some(AiContextSummary {
                version: 1,
                content: "the older half".to_owned(),
                through_message_id: "message-1".to_owned(),
                created_at: 3,
            }),
        },
        "AiSessionDetail",
    );

    // The tool card reads three of these by name: an `undefined` code reads as a failure this build
    // cannot describe, and a missing duration draws as a call that took no time.
    assert_eq!(detail["outcomes"]["call-1"]["callId"], serde_json::json!("call-1"));
    assert_eq!(detail["outcomes"]["call-1"]["durationMs"], serde_json::json!(12));
    assert_eq!(
        detail["contextSummary"]["throughMessageId"],
        serde_json::json!("message-1")
    );
    assert_eq!(detail["summary"]["id"], serde_json::json!("session-1"));
    assert_eq!(
        detail["messages"][0]["blocks"][0]["type"],
        serde_json::json!("toolUse"),
        "a stored message travels as it was written, block types included"
    );
}

#[test]
fn a_session_saved_before_local_terminals_existed_still_reads() {
    let old = serde_json::json!({
        "id": "legacy",
        "name": "legacy",
        "protocol": "ssh",
        "host": "host.example",
        "port": 22,
        "user": "root",
        "auth": { "method": "password", "hasSecret": false },
        "terminal": { "terminalType": "xterm-256color", "keepaliveSeconds": 30 },
        "updatedAt": 1
    });
    let session: Session = serde_json::from_value(old).expect("an older document still reads");
    assert_eq!(session.serial.baud_rate, 115_200);
    assert_eq!(session.serial.data_bits, 8);
    assert_eq!(session.telnet.enter, sevenshell_types::EnterMode::Crlf);
    assert!(session.local.shell.is_empty());
    assert!(session.wsl.distro.is_empty());
}

#[test]
fn a_paste_outcome_serializes_with_camel_case_fields() {
    let asked = check(
        &SftpPasteOutcome {
            conflicts: vec!["/home/user/logs".to_owned()],
            results: Vec::new(),
        },
        "SftpPasteOutcome::conflicts",
    );
    assert_eq!(asked["conflicts"], serde_json::json!(["/home/user/logs"]));

    let done = check(
        &SftpPasteOutcome {
            conflicts: Vec::new(),
            results: vec![SftpPasteItem {
                source: "/home/user/notes.txt".to_owned(),
                target: "/home/user/archive/notes.txt".to_owned(),
                ok: false,
                error: Some("cp: permission denied".to_owned()),
            }],
        },
        "SftpPasteOutcome::results",
    );
    assert_eq!(
        done["results"][0]["target"],
        serde_json::json!("/home/user/archive/notes.txt")
    );
    assert_eq!(done["results"][0]["error"], serde_json::json!("cp: permission denied"));
    assert_eq!(done["results"][0]["ok"], serde_json::json!(false));
}

#[test]
fn container_and_cluster_wire_types_keep_their_camel_case_fields() {
    let summary = check(
        &ContainerSummary {
            id: "abc".to_owned(),
            name: "web".to_owned(),
            image: "nginx:latest".to_owned(),
            state: "running".to_owned(),
            status: "Up 2 hours".to_owned(),
            ports: "0.0.0.0:8080->80/tcp".to_owned(),
            created_at: "2026-11-10 10:00:00".to_owned(),
        },
        "ContainerSummary",
    );
    assert_eq!(summary["createdAt"], serde_json::json!("2026-11-10 10:00:00"));

    let detail = check(
        &ContainerDetail {
            id: "abc".to_owned(),
            name: "web".to_owned(),
            image: "nginx:latest".to_owned(),
            state: "running".to_owned(),
            status: "Up".to_owned(),
            started_at: "2026-11-10".to_owned(),
            finished_at: String::new(),
            exit_code: None,
            oom_killed: false,
            pid: 12,
            restart_policy: "no".to_owned(),
            entrypoint: String::new(),
            command: "nginx".to_owned(),
            work_dir: "/".to_owned(),
            user: String::new(),
            network_mode: "bridge".to_owned(),
            ip: "172.17.0.2".to_owned(),
            gateway: "172.17.0.1".to_owned(),
            ports: vec![ContainerPort {
                host_ip: "0.0.0.0".to_owned(),
                host_port: "8080".to_owned(),
                container_port: "80".to_owned(),
                protocol: "tcp".to_owned(),
            }],
            mounts: vec![ContainerMount {
                source: "/data".to_owned(),
                destination: "/usr/share/nginx/html".to_owned(),
                read_only: true,
            }],
            env: vec!["A=B".to_owned()],
            raw: "{}".to_owned(),
        },
        "ContainerDetail",
    );
    assert_eq!(detail["oomKilled"], serde_json::json!(false));
    assert_eq!(detail["mounts"][0]["readOnly"], serde_json::json!(true));

    let settings = check(
        &ContainerSettings {
            runtime: ContainerRuntime::Podman,
            transport: ContainerTransport::Ssh,
            ssh_session: "lab".to_owned(),
            namespace: "default".to_owned(),
        },
        "ContainerSettings",
    );
    assert_eq!(settings["sshSession"], serde_json::json!("lab"));
    assert_eq!(settings["runtime"], serde_json::json!("podman"));

    let cluster = check(
        &KubernetesSettings {
            source: KubeconfigSource::Imported,
            context: Some("lab".to_owned()),
            namespace: "default".to_owned(),
            via_ssh: Some("gateway".to_owned()),
        },
        "KubernetesSettings",
    );
    assert_eq!(cluster["viaSsh"], serde_json::json!("gateway"));

    let context = check(
        &KubeContextInfo {
            name: "lab".to_owned(),
            cluster: "lab-cluster".to_owned(),
            user: "admin".to_owned(),
            namespace: "default".to_owned(),
            current: true,
        },
        "KubeContextInfo",
    );
    assert_eq!(context["current"], serde_json::json!(true));

    let request = check(
        &K8sRequest {
            method: K8sMethod::Patch,
            path: "/api/v1/nodes/lab".to_owned(),
            body: Some("{}".to_owned()),
            content_type: Some("application/merge-patch+json".to_owned()),
            acknowledged: true,
        },
        "K8sRequest",
    );
    assert_eq!(
        request["contentType"],
        serde_json::json!("application/merge-patch+json")
    );
    assert_eq!(request["method"], serde_json::json!("patch"));

    let response = check(
        &K8sResponse {
            status: 200,
            body: "{}".to_owned(),
        },
        "K8sResponse",
    );
    assert_eq!(response["status"], serde_json::json!(200));
}

/// An external MCP server's two shapes: the configuration the settings file holds, and the state
/// the page draws for it.
///
/// The words are checked rather than only their casing: `stdio`, `ready` and `serverId` are what the
/// page matches on, and a state that arrived as `Ready` would draw as an unknown row.
#[test]
fn an_external_server_and_its_status_speak_the_interface_s_words() {
    let transport = check(&McpServerTransport::Sse, "McpServerTransport");
    assert_eq!(transport, serde_json::json!("sse"));

    let state = check(&McpServerState::Ready, "McpServerState");
    assert_eq!(state, serde_json::json!("ready"));

    let status = check(
        &McpServerStatus::new("files", McpServerState::Connecting)
            .with_failure("MCP-SERVER-CONNECT-FAILED", "the server did not answer"),
        "McpServerStatus",
    );
    assert_eq!(status["serverId"], serde_json::json!("files"));
    assert_eq!(status["state"], serde_json::json!("failed"));
    assert_eq!(status["tools"], serde_json::json!(0));
    assert_eq!(status["code"], serde_json::json!("MCP-SERVER-CONNECT-FAILED"));
}

/// What the resource and prompt picker reads: the catalogue, one resource read and one prompt.
///
/// Every optional field is omitted rather than sent as null, and the prompt's role is the lower-case
/// word the interface matches on. The blob is carried as base64 exactly as the server sent it.
#[test]
fn server_content_speaks_the_interface_s_words() {
    let catalogue = check(
        &McpServerContent {
            server_id: "files".to_owned(),
            name: "Files".to_owned(),
            resources: vec![McpResource {
                uri: "file:///notes.txt".to_owned(),
                name: "notes.txt".to_owned(),
                title: Some("Notes".to_owned()),
                description: None,
                mime_type: Some("text/plain".to_owned()),
                size: Some(42),
            }],
            prompts: vec![McpPrompt {
                name: "summarize".to_owned(),
                title: None,
                description: Some("Summarize a file".to_owned()),
                arguments: vec![McpPromptArgument {
                    name: "path".to_owned(),
                    title: None,
                    description: None,
                    required: true,
                }],
            }],
        },
        "McpServerContent",
    );
    assert_eq!(catalogue["serverId"], serde_json::json!("files"));
    assert_eq!(catalogue["resources"][0]["uri"], serde_json::json!("file:///notes.txt"));
    assert_eq!(catalogue["resources"][0]["mimeType"], serde_json::json!("text/plain"));
    assert!(catalogue["resources"][0].get("description").is_none());
    assert_eq!(
        catalogue["prompts"][0]["arguments"][0]["required"],
        serde_json::json!(true)
    );
    assert!(catalogue["prompts"][0].get("title").is_none());

    let read = check(
        &McpResourceRead {
            server_id: "files".to_owned(),
            uri: "file:///notes.txt".to_owned(),
            contents: vec![McpResourceContent {
                uri: "file:///notes.txt".to_owned(),
                mime_type: Some("text/plain".to_owned()),
                text: Some("hello".to_owned()),
                blob: None,
                truncated: false,
            }],
            truncated: false,
        },
        "McpResourceRead",
    );
    assert_eq!(read["contents"][0]["text"], serde_json::json!("hello"));
    assert!(read["contents"][0].get("blob").is_none());

    let prompt = check(
        &McpPromptText {
            server_id: "files".to_owned(),
            name: "summarize".to_owned(),
            description: None,
            messages: vec![McpPromptMessage {
                role: McpPromptRole::Assistant,
                text: "Summarize it".to_owned(),
            }],
            truncated: false,
        },
        "McpPromptText",
    );
    assert_eq!(prompt["messages"][0]["role"], serde_json::json!("assistant"));
    assert!(prompt.get("description").is_none());
}

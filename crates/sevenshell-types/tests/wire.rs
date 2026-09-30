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
    AppSettings, AuthMethod, BatchHostOutcome, BatchHostResult, BatchOutcome, BatchRequest, FilesLayout, HostFailure,
    KeyInfo, KnownHostEntry, LocalForward, OpenOutcome, OpenTunnel, PresentedHostKey, Protocol, RiskAssessment,
    RiskLevel, RiskReason, Session, SessionAuth, SessionGroup, TerminalSettings, ThemeMode, TunnelFailure, VaultState,
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
fn the_remaining_wire_types_serialize_with_camel_case_fields() {
    check(
        &AppSettings {
            theme_mode: ThemeMode::Dark,
            terminal_theme: ThemeMode::System,
            files_layout: FilesLayout::List,
            locale: Some("zh-CN".to_owned()),
        },
        "AppSettings",
    );
    check(&ThemeMode::System, "ThemeMode");
    check(&VaultState::Unlocked, "VaultState");
    check(
        &SessionGroup {
            id: "g1".to_owned(),
            name: "work".to_owned(),
            parent_id: Some("g0".to_owned()),
            order: 1,
        },
        "SessionGroup",
    );
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

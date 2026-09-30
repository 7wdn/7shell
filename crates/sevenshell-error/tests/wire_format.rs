//! The wire shape of the error envelope.
//!
//! These assertions are the contract the frontend mirrors; if the JSON changes,
//! `apps/desktop/src/lib/backend/errors.ts` and the contract test must change with it.

use serde_json::json;
use sevenshell_error::{ErrorCode, OperationOutcome, ShellError};

#[test]
fn host_key_mismatch_serializes_to_the_documented_shape() {
    let error = ShellError::ssh_host_key_mismatch("host.example", 22, "ssh-ed25519")
        .with_diagnostic("presented", "SHA256:aaa")
        .with_diagnostic("trusted", "SHA256:bbb");

    let value = serde_json::to_value(&error).expect("envelope serializes");

    assert_eq!(
        value,
        json!({
            "version": 1,
            "code": "SSH-HOSTKEY-MISMATCH",
            "messageKey": "errors.ssh.hostKeyMismatch",
            "messageParams": { "host": "host.example", "port": "22" },
            "origin": { "subsystem": "ssh", "adapter": "russh", "peer": "host.example:22" },
            "outcome": "notSent",
            "diagnostics": { "keyType": "ssh-ed25519", "presented": "SHA256:aaa", "trusted": "SHA256:bbb" }
        })
    );
}

#[test]
fn empty_fields_are_omitted_rather_than_sent_as_null() {
    let error = ShellError::vault_locked();
    let value = serde_json::to_value(&error).expect("envelope serializes");

    assert!(value.get("detail").is_none());
    assert!(value.get("diagnostics").is_none());
    assert_eq!(value["outcome"], json!("rejected"));
    assert_eq!(value["code"], json!("VAULT-LOCKED"));
}

#[test]
fn the_envelope_round_trips() {
    let error = ShellError::sftp_transfer_failed("/remote/report.csv", "connection reset")
        .with_outcome(OperationOutcome::Sent)
        .with_diagnostic("bytesTransferred", "4096");

    let json = serde_json::to_string(&error).expect("envelope serializes");
    let restored: ShellError = serde_json::from_str(&json).expect("envelope deserializes");

    assert_eq!(restored, error);
    assert_eq!(restored.code(), ErrorCode::SftpTransferFailed);
    assert_eq!(restored.message_key(), "errors.sftp.transferFailed");
}

#[test]
fn a_frontend_built_envelope_is_accepted() {
    // Mirrors what the interface raises when it has no backend transport available.
    let raw = json!({
        "version": 1,
        "code": "APP-NO-TRANSPORT",
        "messageKey": "errors.app.noTransport",
        "messageParams": { "operation": "app_info" },
        "origin": { "subsystem": "app" },
        "outcome": "rejected"
    });

    let error: ShellError = serde_json::from_value(raw).expect("client envelopes deserialize");
    assert_eq!(error.code(), ErrorCode::AppNoTransport);
    assert_eq!(error.detail(), None);
}

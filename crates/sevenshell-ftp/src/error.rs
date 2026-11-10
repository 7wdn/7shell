//! Turning a protocol failure into the one error shape the rest of the application speaks.
//!
//! Two rules shape the detail:
//!
//! * **No server-authored text.** An FTP error reply is whatever the server chose to say, and it
//!   may quote the command that caused it — including `PASS`. The detail carries the numeric status
//!   and the protocol's own description of it, both of which come from this side of the wire.
//! * **No address.** A peer is `subsystem` information elsewhere; here the detail says what failed,
//!   not where, so a message that reaches a log or a screenshot names nothing.

use sevenshell_error::{OperationOutcome, ShellError};
use suppaftp::FtpError;

use crate::path::PathProblem;

/// A command or a transfer failed.
pub(crate) fn failure(action: &str, error: &FtpError, outcome: OperationOutcome) -> ShellError {
    let detail = format!("FTP {action} failed: {}", describe(error));
    let envelope = match error {
        FtpError::ConnectionError(_) => ShellError::ftp_unreachable(detail),
        FtpError::SecureError(_) => ShellError::ftp_tls_failed(detail),
        FtpError::UnexpectedResponse(response) if matches!(response.status.code(), 530 | 532) => {
            ShellError::ftp_auth_failed(detail)
        }
        FtpError::UnexpectedResponse(response) if matches!(response.status.code(), 550 | 553) => {
            ShellError::ftp_permission_denied(detail)
        }
        _ => ShellError::ftp_operation_failed(detail),
    };
    let envelope = envelope.with_outcome(outcome);
    match error {
        FtpError::UnexpectedResponse(response) => {
            envelope.with_diagnostic("ftpStatus", response.status.code().to_string())
        }
        _ => envelope,
    }
}

/// A path that was refused before it could become a command argument.
pub(crate) fn invalid_path(problem: PathProblem) -> ShellError {
    ShellError::ftp_invalid_path(format!("FTP path refused: {}", problem.reason()))
        .with_outcome(OperationOutcome::NotSent)
}

/// The server answered a listing command with lines in no format this crate reads.
pub(crate) fn unreadable_listing(command: &str) -> ShellError {
    ShellError::ftp_operation_failed(format!(
        "FTP {command} returned a listing in a format this client does not understand"
    ))
}

fn describe(error: &FtpError) -> String {
    match error {
        // OS messages name the reason ("connection refused", "no route to host"). They describe
        // this end's socket, not the peer, so they are safe to carry.
        FtpError::ConnectionError(io) => format!("connection error ({io})"),
        // The TLS library's text can name the host and reads like a stack trace. What a person
        // needs from it is whether the certificate is the problem, so only that distinction is
        // repeated.
        FtpError::SecureError(message) if message.to_ascii_lowercase().contains("certificate") => {
            "TLS certificate verification failed".to_owned()
        }
        FtpError::SecureError(_) => "TLS negotiation failed".to_owned(),
        FtpError::UnexpectedResponse(response) => {
            format!("the server replied {} ({})", response.status.code(), response.status)
        }
        FtpError::BadResponse => "the server reply could not be read".to_owned(),
        FtpError::InvalidAddress(_) => "the address was not valid".to_owned(),
        FtpError::DataConnectionAlreadyOpen => "another transfer is still open".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suppaftp::Status;
    use suppaftp::types::Response;

    #[test]
    fn a_server_reply_contributes_its_status_and_not_its_text() {
        let error = FtpError::UnexpectedResponse(Response::new(
            Status::NotLoggedIn,
            b"530 PASS hunter2 is wrong".to_vec(),
        ));
        let envelope = failure("login", &error, OperationOutcome::Sent);
        let detail = envelope.detail().unwrap_or_default();
        assert!(detail.contains("530"), "detail lost the status: {detail}");
        assert!(
            !detail.contains("hunter2"),
            "server text reached the envelope: {detail}"
        );
        assert_eq!(envelope.diagnostics().get("ftpStatus").map(String::as_str), Some("530"));
        assert_eq!(envelope.outcome(), OperationOutcome::Sent);
    }

    #[test]
    fn a_certificate_failure_is_named_without_repeating_the_tls_library_text() {
        let error = FtpError::SecureError("certificate verify failed: hostname mismatch".to_owned());
        let detail = failure("FTPS negotiation", &error, OperationOutcome::NotSent)
            .detail()
            .unwrap_or_default()
            .to_owned();
        assert!(detail.contains("certificate"), "the reason was lost: {detail}");
        assert!(!detail.contains("hostname mismatch"), "TLS text was repeated: {detail}");
    }

    #[test]
    fn a_refused_path_says_which_rule_it_broke() {
        let detail = invalid_path(PathProblem::OutsideRoot)
            .detail()
            .unwrap_or_default()
            .to_owned();
        assert!(
            detail.contains("session root"),
            "detail did not name the rule: {detail}"
        );
    }
}

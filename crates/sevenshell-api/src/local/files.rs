//! Reading and writing a remote file through an agent.
//!
//! Three tools, and the interesting decisions are about what is *not* here:
//!
//! * **No local path.** Nothing in this module names a file on the machine the application runs on.
//!   A path chosen in a native dialog stays on the other side of the window; an agent asks about a
//!   *remote* path, and gets text back.
//! * **No truncation of a file's text.** A file that is too large to return is refused with its size,
//!   because half a configuration file read as if it were whole is worse than a refusal.
//! * **A read is a read.** Listing and reading have no side effects, and the one write says what it
//!   replaced by how many bytes it wrote.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use sevenshell_types::{RiskAssessment, RiskLevel};

use crate::context::ToolContext;
use crate::registry::Tool;
use crate::tool::{ToolAccess, ToolAudit, ToolDescriptor, ToolError, ToolInputSchema, ToolResult, parse_arguments};

use super::gate;
use super::session_id;

pub const SFTP_LIST: &str = "sshell_sftp_list";
pub const SFTP_READ_TEXT: &str = "sshell_sftp_read_text";
pub const SFTP_WRITE_TEXT: &str = "sshell_sftp_write_text";

// -- Inputs -------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SftpListInput {
    pub session_id: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SftpReadTextInput {
    pub session_id: String,
    pub path: String,
}

/// No `Debug`, deliberately: the text is somebody's file, and a struct that prints it is a struct
/// that puts it in a log the first time somebody derives one more trait.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SftpWriteTextInput {
    pub session_id: String,
    pub path: String,
    pub text: String,
}

// -- Descriptors --------------------------------------------------------

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn session_id_property() -> Value {
    string("The id of a saved session, from sshell_sessions_list.")
}

pub fn sftp_list_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: SFTP_LIST.to_owned(),
        title: "List a remote directory".to_owned(),
        description: "Reads one directory of a saved session's file system and answers with its \
                      entries: name, full path, kind, size and the permission bits the server sent."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "path": string("The remote directory to read. Absent or empty asks for the session's home directory.")
            }),
            &["sessionId"],
        ),
        access: ToolAccess::Read,
    }
}

pub fn sftp_read_text_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: SFTP_READ_TEXT.to_owned(),
        title: "Read a remote file".to_owned(),
        description: "Reads one remote file as UTF-8 text. A file larger than the editor's limit or \
                      not valid UTF-8 is refused rather than truncated."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "path": string("The remote file to read.")
            }),
            &["sessionId", "path"],
        ),
        access: ToolAccess::Read,
    }
}

pub fn sftp_write_text_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: SFTP_WRITE_TEXT.to_owned(),
        title: "Write a remote file".to_owned(),
        description: "Writes one remote file as UTF-8 text, replacing what is there. There is no \
                      upload of a local file: this interface never names a path on the machine it \
                      runs on."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "path": string("The remote file to write."),
                "text": string("The whole new contents, as UTF-8 text.")
            }),
            &["sessionId", "path", "text"],
        ),
        access: ToolAccess::Write,
    }
}

// -- The tools ----------------------------------------------------------

pub struct SftpListTool;

#[async_trait]
impl Tool for SftpListTool {
    fn descriptor(&self) -> ToolDescriptor {
        sftp_list_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: SftpListInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        let asked = input
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(remote_path)
            .transpose()?;
        // A listing is a read, and the session's own tier decides whether even that is offered.
        // Before the cache: opening a channel is the work a refusal exists to avoid.
        context.guard_session(session_id, ToolAccess::Read)?;

        let listing = gate::bounded(context, gate::READ_TIMEOUT, async {
            context
                .cache()
                .files(session_id, asked.as_deref())
                .await
                .map_err(ToolError::from)
        })
        .await?;

        let (entries, total, truncated) = gate::rows(&listing.entries);
        let subject = gate::subject(asked.as_deref().unwrap_or(&listing.path));

        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "path": listing.path,
            "entries": entries,
            "total": total,
            "truncated": truncated,
        }))
        .with_audit(ToolAudit::for_session(session_id).with_subject(subject)))
    }
}

pub struct SftpReadTextTool;

#[async_trait]
impl Tool for SftpReadTextTool {
    fn descriptor(&self) -> ToolDescriptor {
        sftp_read_text_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: SftpReadTextInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        let path = remote_path(&input.path)?;
        context.guard_session(session_id, ToolAccess::Read)?;
        let subject = gate::subject(&path);
        let audit = ToolAudit::for_session(session_id).with_subject(subject);

        let text = gate::bounded(context, gate::READ_TIMEOUT, async {
            context
                .cache()
                .read_text(session_id, &path)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        // Refused rather than cut: a file returned half-way looks exactly like a file that is that
        // long, and an agent that is building on it would never know.
        if text.len() > gate::TEXT_LIMIT {
            return Err(ToolError::new(gate::TOO_LARGE_CODE)
                .with_message(format!(
                    "the file at {path} is {} bytes; at most {} bytes of text are returned",
                    text.len(),
                    gate::TEXT_LIMIT
                ))
                .with_audit(audit));
        }

        let bytes = text.len();
        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "path": path,
            "text": text,
            "bytes": bytes,
        }))
        .with_audit(audit))
    }
}

pub struct SftpWriteTextTool;

#[async_trait]
impl Tool for SftpWriteTextTool {
    fn descriptor(&self) -> ToolDescriptor {
        sftp_write_text_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: SftpWriteTextInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        let path = remote_path(&input.path)?;
        // A write, so the session's tier is asked before a channel is opened and before the text
        // spends a round trip being refused.
        context.guard_session(session_id, ToolAccess::Write)?;
        let subject = gate::subject(&path);
        let bytes = input.text.len();

        gate::bounded(context, gate::READ_TIMEOUT, async {
            context
                .cache()
                .write_text(session_id, &path, &input.text)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        // The verdict is this tool's own: no classifier rule grades "write this file", and inventing
        // a label key here would need a translation that does not exist. What is recorded is the
        // level a person would want to see in the trail — the file changed — with no rule claimed.
        let risk = RiskAssessment {
            level: RiskLevel::Caution,
            reasons: Vec::new(),
        };

        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "path": path,
            "bytes": bytes,
        }))
        .with_audit(ToolAudit::for_session(session_id).with_subject(subject).with_risk(risk)))
    }
}

/// A remote path, checked for the two things a path must never be.
///
/// Blank is not a path. A control character is not one either — a newline in a path is how one path
/// becomes two operations on the far side.
fn remote_path(path: &str) -> Result<String, ToolError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ToolError::invalid_arguments("path is empty"));
    }
    if trimmed.chars().any(|character| character.is_control()) {
        return Err(ToolError::invalid_arguments("path contains a control character"));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_path_refuses_blank_and_control_characters() {
        assert!(remote_path("  ").is_err());
        assert!(remote_path("/tmp/a\nb").is_err());
        assert_eq!(remote_path(" /srv/app ").expect("a path"), "/srv/app");
    }
}

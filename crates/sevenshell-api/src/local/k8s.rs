//! One Kubernetes API path, read.
//!
//! There is deliberately no method argument: this tool can only GET, so "an agent deleted a
//! namespace" is not a sentence this interface can produce. The path is the API server's own, and it
//! is checked before it is sent — the connection belongs to a saved session, not to an address an
//! agent typed.
//!
//! The body comes back as the server sent it, cut only at the interface's result limit and flagged
//! when it was.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::context::ToolContext;
use crate::registry::Tool;
use crate::tool::{ToolAccess, ToolAudit, ToolDescriptor, ToolError, ToolInputSchema, ToolResult, parse_arguments};

use super::gate;
use super::session_id;

pub const K8S_GET: &str = "sshell_k8s_get";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct K8sGetInput {
    pub session_id: String,
    pub path: String,
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

pub fn k8s_get_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: K8S_GET.to_owned(),
        title: "Read a Kubernetes resource".to_owned(),
        description: "Reads one Kubernetes API path with GET and answers with the status and the \
                      body exactly as the API server sent them. Only reads: there is no method \
                      argument, so this tool cannot change a cluster."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": string("The id of a saved session, from sshell_sessions_list."),
                "path": string("The API path, for example /api/v1/namespaces/default/pods.")
            }),
            &["sessionId", "path"],
        ),
        access: ToolAccess::Read,
    }
}

pub struct K8sGetTool;

#[async_trait]
impl Tool for K8sGetTool {
    fn descriptor(&self) -> ToolDescriptor {
        k8s_get_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: K8sGetInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        let path = api_path(&input.path)?;
        let subject = gate::subject(&path);

        let response = gate::bounded(context, gate::READ_TIMEOUT, async {
            context
                .cache()
                .k8s_get(session_id, &path)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        let (body, truncated) = gate::clip(&response.body, gate::TEXT_LIMIT);

        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "path": path,
            "status": response.status,
            "body": body,
            "truncated": truncated,
        }))
        .with_audit(ToolAudit::for_session(session_id).with_subject(subject)))
    }
}

/// A path the API server can be asked about.
///
/// Three refusals, and each of them is a way one request stops being about one cluster path: an
/// absolute URL would replace the session's cluster with an address an agent chose, a blank path has
/// no meaning, and a control character is a header injection waiting for a client that joins
/// strings.
fn api_path(path: &str) -> Result<String, ToolError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ToolError::invalid_arguments("path is empty"));
    }
    if !trimmed.starts_with('/') {
        return Err(ToolError::invalid_arguments(
            "path must be an API path beginning with `/`",
        ));
    }
    if trimmed.contains("://") || trimmed.contains("..") {
        return Err(ToolError::invalid_arguments(
            "path must be a path on the cluster's API server, not an address",
        ));
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
    fn a_path_that_is_an_address_is_refused() {
        assert!(api_path("https://evil.example/api/v1/pods").is_err());
        assert!(api_path("/api/v1/namespaces/../secrets").is_err());
        assert!(api_path("api/v1/pods").is_err());
        assert!(
            api_path(" /api/v1/pods\n").is_ok(),
            "a trailing newline is trimmed, not kept"
        );
        assert_eq!(api_path("/api/v1/pods").expect("a path"), "/api/v1/pods");
    }
}

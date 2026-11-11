//! Turning one external tool into the tool contract this build already has.
//!
//! Everything an agent sees about an external tool is a [`ToolDescriptor`]: the name it calls, the
//! JSON Schema it fills, and how much it is allowed to change. The first two come from the server;
//! the third is this build's judgement, and the only signal it has is the server's own
//! `annotations.readOnlyHint`. A tool the server did not mark read-only is treated as a write — the
//! conservative answer, because a server that cannot be checked must not be trusted to have said
//! nothing dangerous.
//!
//! What the server may hand over is bounded before it reaches a prompt:
//!
//! * the schema has to *be* an object schema, is stripped of `$schema` (a dialect declaration this
//!   build does not read), and may not exceed [`MAX_INPUT_SCHEMA_BYTES`];
//! * the description may not exceed [`MAX_DESCRIPTION_BYTES`], and a longer one is cut with a
//!   marker rather than refused: a verbose description is not a reason to withhold a tool.
//!
//! A write's every call stops for approval. Nothing about an external write can be a policy
//! decision: this build did not write the tool, cannot see what it does with the arguments, and has
//! only the server's word that it is read-only. See [`sevenshell_api::ApprovalReason::External`].

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use sevenshell_api::{
    ApprovalDecision, ApprovalRequest, Tool, ToolAccess, ToolAudit, ToolContext, ToolDescriptor, ToolError,
    ToolInputSchema, ToolResult,
};
use sevenshell_error::ErrorCode;

use crate::McpClientError;
use crate::config::{split_tool_name, tool_name};
use crate::session::McpSession;

/// The largest input schema this build puts in front of an agent.
///
/// Deep and wide schemas use tokens in every request they are offered in. Thirty-two KiB is far
/// past any hand-written tool and still small enough that a server cannot spend a conversation on
/// one schema.
pub const MAX_INPUT_SCHEMA_BYTES: usize = 32 * 1024;

/// How much of a tool's description an agent is given.
pub const MAX_DESCRIPTION_BYTES: usize = 4 * 1024;

/// How long a derived title may be. A title is one line in a list; anything longer is a description
/// the server put in the wrong field.
const MAX_TITLE_BYTES: usize = 512;

/// The protocol's own ceiling for a tool name, in characters.
///
/// The full name an agent calls is this plus `mcp__`, the server id and one separator, so a name
/// past this is one no tool listing was meant to carry.
const MAX_TOOL_NAME_CHARS: usize = 128;

/// How an external tool is gated, from what its server said about it.
///
/// `Some(true)` — the only signal that makes a tool read-only — is the server's assertion. `None`
/// means it said nothing, which is not the same as "safe": the answer is [`ToolAccess::Write`].
pub fn access_for_read_only_hint(read_only_hint: Option<bool>) -> ToolAccess {
    if read_only_hint == Some(true) {
        ToolAccess::Read
    } else {
        ToolAccess::Write
    }
}

/// Reads one `tools/list` entry into the descriptor this build serves.
///
/// The name is built, not copied: `mcp__{server}__{tool}` is the one spelling that says which
/// server answers. Everything else is the server's, with the two bounds this build imposes.
pub fn descriptor_for(server_id: &str, listing: &Value) -> Result<ToolDescriptor, McpClientError> {
    let Some(object) = listing.as_object() else {
        return Err(invalid_tool(server_id, "", "the tool listing is not an object"));
    };

    // The name is used exactly as the server spelled it: trimming it would build a tool name whose
    // call reaches a different tool than the one the server listed. A name with whitespace in it is
    // one no client can call back, so it is refused rather than repaired.
    let Some(tool) = object.get("name").and_then(Value::as_str).filter(|name| {
        !name.is_empty() && name.chars().count() <= MAX_TOOL_NAME_CHARS && !name.chars().any(char::is_whitespace)
    }) else {
        return Err(invalid_tool(
            server_id,
            "",
            "a tool's name has to be a non-empty word of at most 128 characters",
        ));
    };

    let Some(schema) = object.get("inputSchema").and_then(Value::as_object) else {
        return Err(invalid_tool(server_id, tool, "the input schema is not an object"));
    };

    let mut schema = schema.clone();
    // `$schema` names the JSON Schema dialect the server wrote. This build does not read dialects;
    // leaving it in would hand an agent a field that says nothing about the arguments.
    schema.remove("$schema");

    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(invalid_tool(
            server_id,
            tool,
            "the input schema is not an object schema",
        ));
    }

    let schema = Value::Object(schema);
    let size = serde_json::to_vec(&schema)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if size > MAX_INPUT_SCHEMA_BYTES {
        return Err(invalid_tool(
            server_id,
            tool,
            &format!("the input schema is {size} bytes, over this build's {MAX_INPUT_SCHEMA_BYTES}-byte limit"),
        ));
    }
    let input_schema: ToolInputSchema = serde_json::from_value(schema)
        .map_err(|_| invalid_tool(server_id, tool, "the input schema could not be read as one schema"))?;

    let description = bounded_text(
        object.get("description").and_then(Value::as_str).unwrap_or_default(),
        MAX_DESCRIPTION_BYTES,
    );
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(tool);
    let title = bounded_text(title, MAX_TITLE_BYTES);

    let read_only_hint = object
        .get("annotations")
        .and_then(|annotations| annotations.get("readOnlyHint"))
        .and_then(Value::as_bool);

    Ok(ToolDescriptor {
        name: tool_name(server_id, tool),
        title,
        description,
        input_schema,
        access: access_for_read_only_hint(read_only_hint),
    })
}

fn invalid_tool(server: &str, tool: &str, detail: &str) -> McpClientError {
    McpClientError::InvalidTool {
        server: server.to_owned(),
        tool: tool.to_owned(),
        detail: detail.to_owned(),
    }
}

/// Cuts text to a byte bound on a character boundary, admitting the cut.
fn bounded_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    const MARKER: &str = " [truncated]";
    let room = max_bytes.saturating_sub(MARKER.len());
    let mut end = room.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARKER}", &text[..end])
}

/// One tool from one external server, as this build's registry sees it.
pub struct ExternalTool {
    session: Arc<McpSession>,
    server: String,
    tool: String,
    descriptor: ToolDescriptor,
    max_result_bytes: usize,
}

impl ExternalTool {
    /// Wraps one descriptor the session's server announced.
    ///
    /// `server` and `tool` are the two halves the name was built from; they are passed rather than
    /// parsed here so a caller that already split the name once does not do it twice.
    pub fn new(
        session: Arc<McpSession>,
        server: impl Into<String>,
        tool: impl Into<String>,
        descriptor: ToolDescriptor,
    ) -> Self {
        Self {
            session,
            server: server.into(),
            tool: tool.into(),
            descriptor,
            max_result_bytes: usize::try_from(sevenshell_api::DEFAULT_MAX_RESULT_BYTES).unwrap_or(usize::MAX),
        }
    }

    /// Wraps one `tools/list` entry. The name halves are read back out of the descriptor, so this
    /// cannot disagree with what an agent calls.
    pub fn from_listing(
        session: Arc<McpSession>,
        server: &str,
        listing: &Value,
        max_result_bytes: usize,
    ) -> Result<Self, McpClientError> {
        let descriptor = descriptor_for(server, listing)?;
        let tool = split_tool_name(&descriptor.name)
            .map(|(_, tool)| tool.to_owned())
            .ok_or_else(|| invalid_tool(server, "", "the tool's name could not be read back"))?;
        Ok(Self {
            session,
            server: server.to_owned(),
            tool,
            descriptor,
            max_result_bytes: max_result_bytes.max(1),
        })
    }

    /// The most this tool's answer may be, from the settings page.
    pub fn with_result_limit(mut self, max_result_bytes: usize) -> Self {
        self.max_result_bytes = max_result_bytes.max(1);
        self
    }

    /// The server this tool belongs to, as the approval question names it.
    pub fn server(&self) -> &str {
        &self.server
    }

    /// The tool's own name, without the server's half.
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The session a call would go through.
    pub fn session(&self) -> &Arc<McpSession> {
        &self.session
    }

    /// What the audit line says the call was about: the server and the tool, never the arguments.
    ///
    /// The arguments are the agent's, and an agent's argument can be a password or a file's text.
    /// The server and the tool are what an investigation needs to find the call again.
    fn subject(&self) -> String {
        format!("{}/{}", self.server, self.tool)
    }
}

#[async_trait]
impl Tool for ExternalTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let subject = self.subject();
        let mut audit = ToolAudit::default().with_subject(subject.clone());

        // A call somebody already stopped asks nothing and sends nothing.
        context
            .cancel()
            .check()
            .map_err(|error| error.with_audit(audit.clone()))?;

        if self.descriptor.access == ToolAccess::Write {
            // Every call, every time. There is no "remember" for a tool this build cannot judge.
            let request = ApprovalRequest::external(&self.server, &self.tool);
            let record = context
                .cancel()
                .guard(context.request_approval(request))
                .await
                .map_err(|error| error.with_audit(audit.clone()))?;

            if !record.decision.is_approved() {
                let refused = match record.decision {
                    ApprovalDecision::Denied => "the person at the desktop denied this external tool call",
                    ApprovalDecision::TimedOut => "nobody answered the approval request before it expired",
                    ApprovalDecision::Approved => unreachable!("the approved case returned above"),
                };
                return Err(ToolError::denied(&subject)
                    .with_message(refused)
                    .with_audit(audit.with_approval(record)));
            }
            audit = audit.with_approval(record);
        }

        let call = async {
            self.session
                .call_tool(&self.tool, arguments, self.max_result_bytes)
                .await
                .map_err(|error| call_error(&error))
        };
        let outcome = context
            .cancel()
            .guard(call)
            .await
            .map_err(|error| error.with_audit(audit.clone()))?;

        if outcome.is_error {
            let message = match &outcome.value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            return Err(ToolError::new(ErrorCode::McpExternalFailed.as_str())
                .with_message(message)
                .with_audit(audit));
        }

        Ok(ToolResult::new(outcome.value).with_audit(audit))
    }
}

/// A transport failure as the tool contract reports it.
///
/// The code is the client's own (the catalog's, where it has one); the sentence is the scrubbed
/// detail, because an agent's next step depends on whether the server refused, timed out or went
/// away. No audit here: the caller owns one line's worth of facts and attaches them once.
fn call_error(error: &McpClientError) -> ToolError {
    let message = error.detail().map(str::to_owned).unwrap_or_else(|| error.to_string());
    ToolError::new(error.code()).with_message(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    use sevenshell_api::{ApprovalReason, ToolAccess};
    use sevenshell_types::{McpScope, McpServerConfig, McpServerTransport};

    use crate::session::LATEST_PROTOCOL_VERSION;
    use crate::session::McpSession;
    use crate::testing::{
        FakeTransport, RecordingBroker, context, context_denying, initialize_reply, read_tool_listing, text_result,
        tool_listing, tools_reply,
    };

    fn server() -> McpServerConfig {
        let mut server = McpServerConfig::new("files", "Files", McpServerTransport::Stdio);
        server.command = Some("unused".to_owned());
        server
    }

    fn scripted(result: Value) -> Arc<FakeTransport> {
        Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("read", "read a file")]))
                .on("tools/call", result),
        )
    }

    async fn session(fake: Arc<FakeTransport>) -> Arc<McpSession> {
        Arc::new(McpSession::open(server(), Box::new(fake)).await.expect("a session"))
    }

    #[test]
    fn only_an_explicit_read_only_hint_makes_a_tool_read_only() {
        assert_eq!(access_for_read_only_hint(Some(true)), ToolAccess::Read);
        assert_eq!(
            access_for_read_only_hint(Some(false)),
            ToolAccess::Write,
            "a server that says it writes is taken at its word"
        );
        assert_eq!(
            access_for_read_only_hint(None),
            ToolAccess::Write,
            "silence is not a promise: a tool nothing is known about is treated as a write"
        );
    }

    #[test]
    fn a_descriptor_carries_the_built_name_the_schema_and_the_hint() {
        let listing = json!({
            "name": "read_file",
            "title": "Read a file",
            "description": "reads one file",
            "inputSchema": {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true },
        });

        let descriptor = descriptor_for("files", &listing).expect("a descriptor");
        assert_eq!(descriptor.name, "mcp__files__read_file");
        assert_eq!(descriptor.title, "Read a file");
        assert_eq!(descriptor.description, "reads one file");
        assert_eq!(descriptor.access, ToolAccess::Read);
        assert!(
            descriptor.input_schema.as_value().get("$schema").is_none(),
            "the dialect declaration is stripped: {}",
            descriptor.input_schema.as_value()
        );
        assert_eq!(descriptor.input_schema.as_value()["type"], json!("object"));
    }

    #[test]
    fn a_tool_without_a_title_is_named_by_its_own_name_and_silence_is_a_write() {
        let descriptor = descriptor_for("files", &tool_listing("write_file", "")).expect("a descriptor");
        assert_eq!(descriptor.title, "write_file");
        assert_eq!(descriptor.access, ToolAccess::Write);
        assert_eq!(descriptor.description, "");
    }

    #[test]
    fn a_description_past_the_limit_is_cut_with_a_marker_rather_than_refused() {
        let long = "d".repeat(MAX_DESCRIPTION_BYTES + 500);
        let listing = json!({
            "name": "read",
            "description": long,
            "inputSchema": { "type": "object" },
        });
        let descriptor = descriptor_for("files", &listing).expect("a descriptor");
        assert!(
            descriptor.description.len() <= MAX_DESCRIPTION_BYTES,
            "the description is bounded: {} bytes",
            descriptor.description.len()
        );
        assert!(descriptor.description.ends_with("[truncated]"));
    }

    #[test]
    fn a_schema_that_is_not_an_object_schema_is_refused() {
        let cases = [
            json!({ "name": "read", "inputSchema": "object" }),
            json!({ "name": "read", "inputSchema": { "type": "string" } }),
            json!({ "name": "read", "inputSchema": { "properties": {} } }),
            json!({ "name": "read" }),
            json!({ "name": "", "inputSchema": { "type": "object" } }),
            json!({ "name": " read ", "inputSchema": { "type": "object" } }),
            json!({ "name": "a".repeat(MAX_TOOL_NAME_CHARS + 1), "inputSchema": { "type": "object" } }),
        ];

        for listing in cases {
            let error = descriptor_for("files", &listing).expect_err("this tool cannot be served");
            assert_eq!(error.code(), "MCP-INVALID-TOOL", "{listing}");
        }
    }

    #[test]
    fn a_schema_past_the_byte_limit_is_refused_and_one_under_it_is_not() {
        let roomy = json!({
            "name": "read",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "p".repeat(MAX_INPUT_SCHEMA_BYTES) } },
            },
        });
        let error = descriptor_for("files", &roomy).expect_err("past the limit");
        assert_eq!(error.code(), "MCP-INVALID-TOOL");
        assert!(
            error
                .detail()
                .is_some_and(|detail| detail.contains(&MAX_INPUT_SCHEMA_BYTES.to_string())),
            "the refusal says what the limit is: {error:?}"
        );

        let fits = json!({
            "name": "read",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "p".repeat(MAX_INPUT_SCHEMA_BYTES - 1_000) } },
            },
        });
        assert!(descriptor_for("files", &fits).is_ok());
    }

    #[tokio::test]
    async fn a_read_tool_runs_without_asking_anybody() {
        let broker = RecordingBroker::asking(&[ApprovalDecision::Denied]);
        let (_directory, context) = context(McpScope::ReadOnly, broker.clone());
        let fake = scripted(text_result("the file's text"));

        let descriptor = descriptor_for("files", &read_tool_listing("read", "")).expect("a descriptor");
        let tool = ExternalTool::new(session(fake).await, "files", "read", descriptor);

        let result = tool
            .call(&context, json!({ "path": "/etc/hosts" }))
            .await
            .expect("a result");
        assert_eq!(result.value, json!("the file's text"));
        assert!(
            broker.seen().is_empty(),
            "a tool the server marked read-only runs without a question"
        );

        let audit = result.audit.expect("the call is audited");
        assert_eq!(audit.subject.as_deref(), Some("files/read"));
        assert!(
            audit.session_id.is_none(),
            "an external call is not about an open terminal"
        );
    }

    #[tokio::test]
    async fn a_write_tool_asks_with_the_server_and_the_tool_and_runs_when_approved() {
        let broker = RecordingBroker::asking(&[ApprovalDecision::Approved]);
        let (_directory, context) = context(McpScope::SafeWrite, broker.clone());
        let fake = scripted(text_result("written"));

        let descriptor = descriptor_for("files", &tool_listing("write", "")).expect("a descriptor");
        let tool = ExternalTool::new(session(fake).await, "files", "write", descriptor);

        let result = tool
            .call(&context, json!({ "path": "/tmp/x" }))
            .await
            .expect("a result");
        assert_eq!(result.value, json!("written"));

        let asked = broker.seen();
        assert_eq!(asked.len(), 1, "one question for one write");
        assert_eq!(
            asked[0].reason,
            ApprovalReason::External {
                server: "files".to_owned(),
                tool: "write".to_owned(),
            }
        );

        let audit = result.audit.expect("the call is audited");
        assert_eq!(audit.subject.as_deref(), Some("files/write"));
        assert_eq!(
            audit.approval.as_ref().map(|record| record.decision),
            Some(ApprovalDecision::Approved),
            "the audit line carries the answer the person gave"
        );
    }

    #[tokio::test]
    async fn a_refused_write_sends_nothing_and_is_a_policy_denial() {
        let broker = RecordingBroker::asking(&[ApprovalDecision::Denied]);
        let (_directory, context) = context(McpScope::SafeWrite, broker.clone());
        let fake = scripted(text_result("this must not be reached"));

        let descriptor = descriptor_for("files", &tool_listing("write", "")).expect("a descriptor");
        let tool = ExternalTool::new(session(Arc::clone(&fake)).await, "files", "write", descriptor);

        let error = tool
            .call(&context, json!({ "path": "/tmp/x" }))
            .await
            .expect_err("a denied write does not run");
        assert_eq!(error.code(), "POLICY-DENIED");
        assert!(fake.calls("tools/call").is_empty(), "nothing was sent to the server");
        assert_eq!(
            error
                .audit()
                .and_then(|audit| audit.approval.as_ref())
                .map(|record| record.decision),
            Some(ApprovalDecision::Denied)
        );
    }

    #[tokio::test]
    async fn a_write_nobody_answered_is_a_refusal_not_a_default_yes() {
        let broker = RecordingBroker::asking(&[ApprovalDecision::TimedOut]);
        let (_directory, context) = context(McpScope::HighRisk, broker);
        let fake = scripted(text_result("unreachable"));

        let descriptor = descriptor_for("files", &tool_listing("write", "")).expect("a descriptor");
        let tool = ExternalTool::new(session(fake).await, "files", "write", descriptor);

        let error = tool.call(&context, json!({})).await.expect_err("a silence is a no");
        assert_eq!(error.code(), "POLICY-DENIED");
    }

    #[tokio::test]
    async fn a_call_stopped_before_it_started_sends_nothing() {
        let (_directory, context) = context_denying(McpScope::HighRisk);
        let cancelled = sevenshell_api::Cancel::new();
        cancelled.cancel();
        let context = context.with_cancel(cancelled);
        let fake = scripted(text_result("unreachable"));

        let descriptor = descriptor_for("files", &read_tool_listing("read", "")).expect("a descriptor");
        let tool = ExternalTool::new(session(Arc::clone(&fake)).await, "files", "read", descriptor);

        let error = tool.call(&context, json!({})).await.expect_err("nothing runs");
        assert_eq!(error.code(), "MCP-CANCELLED");
        assert!(fake.calls("tools/call").is_empty());
    }

    #[tokio::test]
    async fn a_tool_that_ran_and_failed_is_an_external_failure_with_the_server_s_words() {
        let (_directory, context) = context_denying(McpScope::ReadOnly);
        let fake = scripted(json!({
            "content": [{ "type": "text", "text": "the path does not exist" }],
            "isError": true,
        }));

        let descriptor = descriptor_for("files", &read_tool_listing("read", "")).expect("a descriptor");
        let tool = ExternalTool::new(session(fake).await, "files", "read", descriptor);

        let error = tool.call(&context, json!({})).await.expect_err("the tool failed");
        assert_eq!(error.code(), "MCP-EXTERNAL-FAILED");
        assert_eq!(
            error.message(),
            Some("the path does not exist"),
            "an agent gets the server's own words"
        );
        assert_eq!(
            error.audit().and_then(|audit| audit.subject.clone()).as_deref(),
            Some("files/read")
        );
    }

    #[tokio::test]
    async fn a_call_past_its_budget_is_a_timeout_with_its_own_code() {
        let broker = RecordingBroker::asking(&[ApprovalDecision::Approved]);
        let (_directory, context) = context(McpScope::SafeWrite, broker);
        let fake = Arc::new(
            FakeTransport::new()
                .on("initialize", initialize_reply(LATEST_PROTOCOL_VERSION))
                .on("tools/list", tools_reply(&[tool_listing("write", "")]))
                .on_delay("tools/call", Duration::from_secs(30), text_result("too late")),
        );

        let descriptor = descriptor_for("files", &tool_listing("write", "")).expect("a descriptor");
        let session = McpSession::open(server(), Box::new(fake))
            .await
            .expect("a session")
            .with_call_budget(Duration::from_millis(20));
        let tool = ExternalTool::new(Arc::new(session), "files", "write", descriptor);

        let error = tool.call(&context, json!({})).await.expect_err("too slow");
        assert_eq!(error.code(), "MCP-EXTERNAL-TIMEOUT");
    }

    #[tokio::test]
    async fn a_tool_built_from_a_listing_reads_its_two_name_halves_back() {
        let fake = scripted(text_result("ok"));
        let tool = ExternalTool::from_listing(session(fake).await, "files", &tool_listing("read__all", ""), 1024)
            .expect("a tool");

        assert_eq!(tool.descriptor().name, "mcp__files__read__all");
        assert_eq!(tool.server(), "files");
        assert_eq!(tool.tool(), "read__all", "only the first separator splits");
    }
}

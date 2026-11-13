//! The tool contract, held still.
//!
//! What is frozen here is what a later change is not allowed to move without saying so:
//!
//! * the list — ten tools, `sshell_`-prefixed;
//! * the argument schemas — closed objects whose required fields are the ones the input types
//!   refuse to live without;
//! * the refusal of `acknowledged` — an agent cannot agree on a person's behalf;
//! * the error crossing — a catalog failure keeps its code, key, parameters and detail;
//! * the audit line — every call leaves one, including the ones that never ran.
//!
//! The schema-versus-input test is deliberately bidirectional: for every declared property it
//! removes the property and checks that the *input type* agrees about whether it was required. A
//! schema that says "optional" over a field the tool cannot do without is exactly the drift this
//! file exists to catch.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use sevenshell_api::{
    ApprovalBroker, ApprovalDecision, ApprovalReason, ApprovalRequest, AuditRecord, AuditSink, AuditStatus,
    EndpointFile, ToolAccess, ToolCall, ToolContext, ToolError, ToolOutcome, local, scope_allows,
};
use sevenshell_core::Core;
use sevenshell_error::ShellError;
use sevenshell_types::{
    AgentOrigin, AuthMethod, ContainerRuntime, ContainerSettings, DesktopSettings, FtpSettings, KubernetesSettings,
    LocalSettings, McpScope, Protocol, RiskAssessment, RiskLevel, RiskReason, SerialSettings, Session, SessionAuth,
    TelnetSettings, TerminalSettings, WebDavSettings, WslSettings,
};
use sevenshell_vault::Vault;
use tempfile::{TempDir, tempdir};

// -- Fixtures -----------------------------------------------------------

/// One way to call every tool with the fewest arguments its contract allows.
fn minimal_arguments() -> Vec<(&'static str, Value)> {
    vec![
        ("sshell_sessions_list", json!({})),
        ("sshell_session_open", json!({ "sessionId": "lab" })),
        ("sshell_exec_run", json!({ "sessionId": "lab", "command": "uptime" })),
        ("sshell_sftp_list", json!({ "sessionId": "lab" })),
        (
            "sshell_sftp_read_text",
            json!({ "sessionId": "lab", "path": "/etc/hostname" }),
        ),
        (
            "sshell_sftp_write_text",
            json!({ "sessionId": "lab", "path": "/tmp/note", "text": "hello" }),
        ),
        ("sshell_containers_list", json!({ "sessionId": "lab" })),
        (
            "sshell_container_action",
            json!({ "sessionId": "lab", "containerId": "abc", "action": "start" }),
        ),
        (
            "sshell_k8s_get",
            json!({ "sessionId": "lab", "path": "/api/v1/namespaces/default/pods" }),
        ),
        (
            "sshell_batch_exec",
            json!({ "sessionIds": ["lab"], "command": "uptime" }),
        ),
    ]
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        name: name.to_owned(),
        arguments,
    }
}

fn sample_session(id: &str, name: &str) -> Session {
    Session {
        id: id.to_owned(),
        name: name.to_owned(),
        group_id: None,
        protocol: Protocol::Ssh,
        host: "127.0.0.1".to_owned(),
        port: 2222,
        user: "labuser".to_owned(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        desktop: DesktopSettings::default(),
        telnet: TelnetSettings::default(),
        serial: SerialSettings::default(),
        local: LocalSettings::default(),
        wsl: WslSettings::default(),
        container: ContainerSettings::default(),
        kubernetes: KubernetesSettings::default(),
        webdav: WebDavSettings::default(),
        ftp: FtpSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    }
}

/// A context around a vault that exists but was never unlocked. Cheap: no key derivation runs.
fn locked_context(scope: McpScope) -> (TempDir, Arc<Vault>, ToolContext) {
    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    let core = Core::new(Arc::clone(&vault)).0;
    (directory, vault, ToolContext::new(core, scope))
}

/// A context around an unlocked vault, for the one tool that reads it.
fn unlocked_context(scope: McpScope) -> (TempDir, Arc<Vault>, ToolContext) {
    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("correct horse battery staple").expect("create");
    let core = Core::new(Arc::clone(&vault)).0;
    (directory, vault, ToolContext::new(core, scope))
}

#[derive(Default)]
struct MemoryAudit {
    records: Mutex<Vec<AuditRecord>>,
}

impl AuditSink for MemoryAudit {
    fn record(&self, record: AuditRecord) {
        self.records.lock().expect("not poisoned").push(record);
    }
}

impl MemoryAudit {
    fn records(&self) -> Vec<AuditRecord> {
        self.records.lock().expect("not poisoned").clone()
    }
}

/// A broker that records the questions and refuses them, so a test can see what was asked.
#[derive(Default)]
struct RecordingBroker {
    asked: Mutex<Vec<ApprovalRequest>>,
}

#[async_trait]
impl ApprovalBroker for RecordingBroker {
    async fn request(&self, request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        self.asked.lock().expect("not poisoned").push(request);
        Ok(ApprovalDecision::Denied)
    }
}

impl RecordingBroker {
    fn asked(&self) -> Vec<ApprovalRequest> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

fn error_of(outcome: ToolOutcome) -> ToolError {
    match outcome {
        ToolOutcome::Failed { error } => error,
        ToolOutcome::Succeeded { result } => panic!("expected a failure, got {}", result.value),
    }
}

// -- The list -----------------------------------------------------------

#[test]
fn the_registry_lists_exactly_the_ten_declared_tools() {
    let registry = local::default_registry();
    let descriptors = registry.descriptors();

    assert_eq!(descriptors.len(), 10, "the first wave is ten tools");
    assert_eq!(registry.len(), 10);

    for descriptor in &descriptors {
        assert!(
            descriptor.name.starts_with("sshell_"),
            "`{}` is outside the prefix every tool shares",
            descriptor.name
        );
        assert!(
            !descriptor.description.trim().is_empty(),
            "{} has nothing to tell an agent",
            descriptor.name
        );
        assert!(!descriptor.title.trim().is_empty());
    }

    let mut names: Vec<&str> = descriptors.iter().map(|descriptor| descriptor.name.as_str()).collect();
    names.sort_unstable();
    let unique = names.len();
    names.dedup();
    assert_eq!(
        names.len(),
        unique,
        "two tools share a name, so one of them is unreachable"
    );
}

#[test]
fn every_schema_is_a_closed_camel_case_object() {
    for descriptor in local::default_registry().descriptors() {
        let schema = descriptor.input_schema.as_value();
        assert_eq!(
            schema["type"],
            json!("object"),
            "{} is not an object schema",
            descriptor.name
        );
        assert_eq!(
            schema["additionalProperties"],
            json!(false),
            "{} would accept arguments it never declared",
            descriptor.name
        );

        let properties: Vec<&str> = descriptor.input_schema.property_names().collect();
        assert_eq!(
            properties.len(),
            schema["properties"].as_object().map_or(0, serde_json::Map::len),
            "{} has a property list that does not read back",
            descriptor.name
        );

        for property in &properties {
            assert!(
                !property.contains('_') && !property.chars().next().is_some_and(char::is_uppercase),
                "{} declares `{property}`, which is not the camelCase name an agent sends",
                descriptor.name
            );
        }
        for required in descriptor.input_schema.required() {
            assert!(
                properties.contains(&required),
                "{} requires `{required}`, which it never declares",
                descriptor.name
            );
        }
    }
}

#[test]
fn no_tool_declares_an_acknowledgement_it_could_receive() {
    for descriptor in local::default_registry().descriptors() {
        assert!(
            descriptor.input_schema.property("acknowledged").is_none(),
            "{} would let an agent acknowledge its own destructive call",
            descriptor.name
        );
    }
}

// -- The arguments ------------------------------------------------------

#[tokio::test]
async fn an_agent_cannot_acknowledge_anything() {
    let (_directory, _vault, context) = locked_context(McpScope::HighRisk);
    let registry = local::default_registry();

    for (name, arguments) in minimal_arguments() {
        let mut arguments = arguments.as_object().cloned().unwrap_or_default();
        arguments.insert("acknowledged".to_owned(), json!(true));

        let error = error_of(registry.call(&context, call(name, Value::Object(arguments))).await);

        assert_eq!(
            error.code(),
            "MCP-INVALID-ARGUMENTS",
            "{name} answered `{}` instead of refusing an argument it never declared",
            error.code()
        );
    }
}

#[tokio::test]
async fn the_schema_and_the_input_agree_about_every_field() {
    let (_directory, _vault, context) = unlocked_context(McpScope::HighRisk);
    let registry = local::default_registry();
    let minimal = minimal_arguments();

    for descriptor in registry.descriptors() {
        let base = minimal
            .iter()
            .find(|(name, _)| *name == descriptor.name)
            .map(|(_, arguments)| arguments.clone())
            .unwrap_or_else(|| panic!("no fixture for {}", descriptor.name));

        let required: Vec<String> = descriptor.input_schema.required().map(str::to_owned).collect();
        let properties: Vec<String> = descriptor.input_schema.property_names().map(str::to_owned).collect();

        for property in properties {
            let mut arguments = base.as_object().cloned().expect("an object fixture");
            arguments.remove(&property);

            let outcome = registry
                .call(&context, call(&descriptor.name, Value::Object(arguments)))
                .await;

            if required.contains(&property) {
                let error = error_of(outcome);
                assert_eq!(
                    error.code(),
                    "MCP-INVALID-ARGUMENTS",
                    "{} accepted a call without its required `{property}` (`{}`)",
                    descriptor.name,
                    error.code()
                );
            } else {
                // Optional means the call gets past parsing: whatever the tool then answers — a
                // missing session, a locked vault — it is not a complaint about the arguments.
                let code = outcome.error().map_or("ok", |error| error.code()).to_owned();
                assert!(
                    code != "MCP-INVALID-ARGUMENTS",
                    "{} refused its own optional `{property}` as unknown (`{code}`)",
                    descriptor.name
                );
            }
        }
    }
}

#[tokio::test]
async fn every_declared_tool_is_implemented() {
    let (_directory, _vault, context) = unlocked_context(McpScope::HighRisk);
    let registry = local::default_registry();

    for (name, arguments) in minimal_arguments() {
        let code = registry
            .call(&context, call(name, arguments))
            .await
            .error()
            .map_or("ok", |error| error.code())
            .to_owned();

        assert_ne!(
            code, "MCP-NOT-IMPLEMENTED",
            "{name} is registered but has nothing behind it"
        );
    }
}

#[tokio::test]
async fn missing_arguments_read_as_an_empty_object() {
    let (_directory, _vault, context) = unlocked_context(McpScope::ReadOnly);

    let outcome = local::default_registry()
        .call(&context, call("sshell_sessions_list", Value::Null))
        .await;

    assert!(outcome.is_succeeded(), "{outcome:?}");
}

// -- What the one implemented tool answers ------------------------------

#[tokio::test]
async fn listing_saved_sessions_answers_without_a_secret_or_a_handle() {
    let (_directory, vault, context) = unlocked_context(McpScope::ReadOnly);
    vault
        .save_session(
            sample_session("lab", "The lab"),
            Some(sevenshell_vault::SessionSecret::Password("hunter2".to_owned())),
        )
        .expect("save");

    let outcome = local::default_registry()
        .call(&context, call("sshell_sessions_list", json!({ "kind": "saved" })))
        .await;

    let ToolOutcome::Succeeded { result } = outcome else {
        panic!("a saved session should be listable: {outcome:?}");
    };

    assert_eq!(result.value["kind"], json!("saved"));
    assert_eq!(result.value["sessions"][0]["id"], json!("lab"));
    assert_eq!(result.value["sessions"][0]["name"], json!("The lab"));
    assert_eq!(result.value["sessions"][0]["protocol"], json!("ssh"));

    let printed = result.value.to_string();
    assert!(!printed.contains("hunter2"), "a password reached an agent: {printed}");
    assert!(
        !printed.contains("password"),
        "the answer names things an agent has no business with"
    );
    assert!(!printed.contains("keyRef"), "a vault key reference reached an agent");
}

#[tokio::test]
async fn a_container_session_names_its_engine_and_other_protocols_do_not() {
    let (_directory, vault, context) = unlocked_context(McpScope::ReadOnly);
    let mut boxes = sample_session("boxes", "The boxes");
    boxes.protocol = Protocol::Container;
    boxes.container = ContainerSettings {
        runtime: ContainerRuntime::Podman,
        ..ContainerSettings::default()
    };
    vault.save_session(boxes, None).expect("save the container session");
    vault
        .save_session(sample_session("lab", "The lab"), None)
        .expect("save the SSH session");

    let outcome = local::default_registry()
        .call(&context, call("sshell_sessions_list", json!({ "kind": "saved" })))
        .await;

    let ToolOutcome::Succeeded { result } = outcome else {
        panic!("saved sessions should be listable: {outcome:?}");
    };
    let rows = result.value["sessions"].as_array().expect("an array of rows");
    let container = rows
        .iter()
        .find(|row| row["id"] == json!("boxes"))
        .expect("the container session is listed");
    assert_eq!(
        container["containerRuntime"],
        json!("podman"),
        "an agent asked for the Podman connections has to be able to tell them apart"
    );
    let ssh = rows
        .iter()
        .find(|row| row["id"] == json!("lab"))
        .expect("the SSH session is listed");
    assert!(
        ssh.get("containerRuntime").is_none(),
        "a runtime on an SSH row would be an invented fact: {ssh}"
    );
}

#[tokio::test]
async fn listing_open_sessions_is_empty_until_something_is_open() {
    let (_directory, _vault, context) = unlocked_context(McpScope::ReadOnly);

    let outcome = local::default_registry()
        .call(&context, call("sshell_sessions_list", json!({ "kind": "open" })))
        .await;

    let ToolOutcome::Succeeded { result } = outcome else {
        panic!("nothing open is an empty list, not a failure: {outcome:?}");
    };
    assert_eq!(result.value["kind"], json!("open"));
    assert_eq!(result.value["sessions"], json!([]));
}

#[tokio::test]
async fn a_locked_vault_is_reported_rather_than_read_as_no_sessions() {
    let (_directory, _vault, context) = locked_context(McpScope::ReadOnly);

    let error = error_of(
        local::default_registry()
            .call(&context, call("sshell_sessions_list", json!({})))
            .await,
    );

    assert_eq!(error.code(), "VAULT-LOCKED");
    assert_eq!(error.message_key(), Some("errors.vault.locked"));
}

// -- The gate -----------------------------------------------------------

#[tokio::test]
async fn a_read_only_scope_refuses_a_write_tool_and_a_wider_one_lets_it_through_the_gate() {
    let audit = Arc::new(MemoryAudit::default());
    let (_directory, _vault, context) = locked_context(McpScope::ReadOnly);
    let context = context.with_audit(audit.clone());
    let registry = local::default_registry();
    let arguments = json!({ "sessionId": "lab", "containerId": "abc", "action": "start" });

    let error = error_of(
        registry
            .call(&context, call("sshell_container_action", arguments.clone()))
            .await,
    );
    assert_eq!(error.code(), "POLICY-DENIED");
    assert_eq!(error.message_key(), Some("errors.policy.denied"));

    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, AuditStatus::Denied);
    assert_eq!(records[0].tool, "sshell_container_action");

    // The same call under a tier that offers writes reaches the tool, which is what proves the
    // refusal above was the scope and not the tool: an unlocked vault with no such session is the
    // answer, not a missing implementation.
    let (_directory, _vault, wider) = locked_context(McpScope::SafeWrite);
    let error = error_of(registry.call(&wider, call("sshell_container_action", arguments)).await);
    assert_eq!(error.code(), "VAULT-LOCKED");
}

#[test]
fn the_scope_table_is_the_one_the_settings_page_offers() {
    assert!(scope_allows(McpScope::ReadOnly, ToolAccess::Read));
    assert!(!scope_allows(McpScope::ReadOnly, ToolAccess::Write));
    assert!(scope_allows(McpScope::SafeWrite, ToolAccess::Write));
    assert!(scope_allows(McpScope::HighRisk, ToolAccess::Write));
}

// -- The error crossing -------------------------------------------------

#[test]
fn a_shell_error_crosses_with_its_code_key_parameters_and_detail() {
    let error =
        ToolError::from(ShellError::ssh_timeout("10.0.0.7", 2222, 2).with_detail("the command was still running"));

    assert_eq!(error.code(), "SSH-TIMEOUT");
    assert_eq!(error.message_key(), Some("errors.ssh.timeout"));
    assert_eq!(error.message_params().get("seconds").map(String::as_str), Some("2"));
    assert_eq!(error.message_params().get("host").map(String::as_str), Some("10.0.0.7"));
    assert_eq!(error.detail(), Some("the command was still running"));
    assert!(
        error.message().is_some_and(|message| message.contains("SSH-TIMEOUT")),
        "the agent is given the stable code it can key off"
    );
}

// -- The audit trail ----------------------------------------------------

#[tokio::test]
async fn every_call_leaves_one_audit_line_including_the_ones_that_never_ran() {
    let audit = Arc::new(MemoryAudit::default());
    let (_directory, _vault, context) = unlocked_context(McpScope::ReadOnly);
    let context = context.with_audit(audit.clone());
    let registry = local::default_registry();

    let _ = registry.call(&context, call("sshell_sessions_list", json!({}))).await;
    let _ = registry.call(&context, call("sshell_never_existed", json!({}))).await;
    let _ = registry
        .call(
            &context,
            call("sshell_exec_run", json!({ "sessionId": "lab", "command": "uptime" })),
        )
        .await;

    let records = audit.records();
    assert_eq!(records.len(), 3, "one line per call, however the call ended");

    assert_eq!(records[0].tool, "sshell_sessions_list");
    assert_eq!(records[0].status, AuditStatus::Succeeded);
    assert_eq!(records[0].scope, McpScope::ReadOnly);

    assert_eq!(records[1].tool, "sshell_never_existed");
    assert_eq!(records[1].status, AuditStatus::Failed);
    assert_eq!(records[1].code.as_deref(), Some("MCP-UNKNOWN-TOOL"));

    assert_eq!(records[2].tool, "sshell_exec_run");
    assert_eq!(records[2].status, AuditStatus::Denied);
    assert_eq!(records[2].code.as_deref(), Some("POLICY-DENIED"));
    assert!(
        records.iter().all(|record| record.origin == AgentOrigin::Mcp),
        "a context that never said where it came from is an MCP one"
    );
}

#[tokio::test]
async fn a_call_from_the_assistant_is_audited_as_one() {
    let audit = Arc::new(MemoryAudit::default());
    let (_directory, _vault, context) = unlocked_context(McpScope::ReadOnly);
    let context = context.with_audit(audit.clone()).with_origin(AgentOrigin::Ai);

    let outcome = local::default_registry()
        .call(&context, call("sshell_sessions_list", json!({})))
        .await;

    assert!(outcome.is_succeeded(), "{outcome:?}");
    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool, "sshell_sessions_list");
    assert_eq!(
        records[0].origin,
        AgentOrigin::Ai,
        "the audit trail has to say who asked, not only what was asked"
    );
}

// -- Approval -----------------------------------------------------------

#[tokio::test]
async fn a_context_with_no_window_refuses_an_approval_instead_of_running() {
    let (_directory, _vault, context) = locked_context(McpScope::HighRisk);
    let risk = RiskAssessment::from_reasons(vec![RiskReason::new(
        "recursive-force-delete",
        "risk.command.recursiveForceDelete",
        RiskLevel::Destructive,
    )]);

    let record = context
        .request_approval(ApprovalRequest::risk("sshell_exec_run", risk.clone()).with_subject("rm -rf /srv/app"))
        .await
        .expect("a broker that says no is not an error");

    assert_eq!(record.decision, ApprovalDecision::Denied);
    assert!(matches!(record.reason, ApprovalReason::Risk { .. }));
    assert!(!record.decision.is_approved());
}

#[tokio::test]
async fn an_approval_question_names_the_caller_the_context_belongs_to() {
    let broker = Arc::new(RecordingBroker::default());
    let risk = RiskAssessment::from_reasons(vec![RiskReason::new(
        "recursive-force-delete",
        "risk.command.recursiveForceDelete",
        RiskLevel::Destructive,
    )]);

    // The assistant's context: the request a tool builds says nothing about a caller, so the context
    // supplies it.
    let (_directory, _vault, assistant) = locked_context(McpScope::HighRisk);
    let assistant = assistant.with_approvals(broker.clone()).with_origin(AgentOrigin::Ai);
    assistant
        .request_approval(ApprovalRequest::risk("sshell_exec_run", risk.clone()))
        .await
        .expect("a broker that says no is not an error");

    // The MCP context keeps the default it always had.
    let (_directory, _vault, mcp) = locked_context(McpScope::HighRisk);
    let mcp = mcp.with_approvals(broker.clone());
    mcp.request_approval(ApprovalRequest::risk("sshell_exec_run", risk))
        .await
        .expect("a broker that says no is not an error");

    let asked = broker.asked();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0].origin, AgentOrigin::Ai, "the window must say who is asking");
    assert_eq!(asked[1].origin, AgentOrigin::Mcp);
}

#[test]
fn anything_written_before_origin_existed_reads_as_mcp() {
    // An audit file is appended to across versions: a line with no origin is a line this build wrote
    // yesterday, and it must not be read as unknown.
    let record: AuditRecord = serde_json::from_value(json!({
        "at": 1,
        "tool": "sshell_sessions_list",
        "scope": "readOnly",
        "status": "succeeded",
        "durationMs": 2
    }))
    .expect("an older audit line still reads");
    assert_eq!(record.origin, AgentOrigin::Mcp);

    // The same for a request that was parked or logged before the field existed.
    let mut request = serde_json::to_value(ApprovalRequest::risk("sshell_exec_run", RiskAssessment::safe()))
        .expect("a request is JSON");
    assert_eq!(request["origin"], json!("mcp"), "the constructor names MCP");
    request.as_object_mut().expect("an object").remove("origin");
    let parsed: ApprovalRequest = serde_json::from_value(request).expect("an older request still reads");
    assert_eq!(parsed.origin, AgentOrigin::Mcp);
}

#[test]
fn a_policy_question_carries_the_layer_that_asked() {
    // The window draws a policy question differently from a risk one: the verdict may be safe, and
    // the source is the whole of what makes it a question at all.
    let request =
        ApprovalRequest::policy("sshell_batch_exec", RiskAssessment::safe(), "host:web:2222").with_subject("uptime");
    let value = serde_json::to_value(&request).expect("a request is JSON");
    assert_eq!(value["reason"]["kind"], json!("policy"));
    assert_eq!(value["reason"]["source"], json!("host:web:2222"));
    assert_eq!(value["reason"]["assessment"]["level"], json!("safe"));

    let parsed: ApprovalRequest = serde_json::from_value(value).expect("the window's shape reads back");
    assert_eq!(parsed.reason, request.reason);
}

// -- The endpoint file --------------------------------------------------

#[test]
fn an_endpoint_file_never_prints_its_token() {
    let endpoint = EndpointFile::new(4711, "s3cret-token", 42, 1_700_000_000);

    let printed = format!("{endpoint:?}");

    assert!(
        !printed.contains("s3cret-token"),
        "a debug line leaked the bearer token"
    );
    assert!(
        printed.contains("4711"),
        "the port is not a secret and is what a person looks for"
    );
    assert_eq!(endpoint.base_url(), "http://127.0.0.1:4711");
}

#[test]
fn an_endpoint_file_carries_the_configured_result_limit_and_an_old_file_still_reads() {
    let endpoint = EndpointFile::new(4711, "a-token", 42, 1_700_000_000).with_max_result_bytes(512 * 1024);
    let encoded = serde_json::to_value(&endpoint).expect("an endpoint is JSON");

    assert_eq!(
        encoded["maxResultBytes"],
        json!(512 * 1024),
        "the bridge reads the ceiling from the file, not from a compiled constant"
    );

    // A file written before the field existed reads as the compiled default, never as zero: zero
    // would truncate every answer to an empty string.
    let old: EndpointFile = serde_json::from_str(r#"{"port":4711,"token":"old","pid":1,"startedAt":2}"#)
        .expect("an old endpoint file still parses");
    assert_eq!(old.max_result_bytes, sevenshell_api::DEFAULT_MAX_RESULT_BYTES);
}

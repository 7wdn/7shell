//! What each tool answers, and what it asks before it answers.
//!
//! Everything here runs without a network and without a runtime: the fixtures are a vault with no
//! sessions in it, a broker that answers from a script, and an audit sink that keeps the lines in
//! memory. That is enough to pin the four things that are easy to get subtly wrong:
//!
//! * **The scope gate**, which is the registry's and must stay the registry's.
//! * **The risk gate**, which is the policy crate's own classifier and no second set of rules.
//! * **The approval**, which only a broker can produce — never an argument.
//! * **The audit line**, which has to explain a refused call as well as a successful one.
//!
//! The live half — a real host, a real key, a real file — is `live_tools.rs` and only runs when the
//! environment names a server.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use sevenshell_api::local;
use sevenshell_api::{
    ApprovalBroker, ApprovalDecision, ApprovalReason, ApprovalRequest, AuditRecord, AuditSink, AuditStatus, ToolCall,
    ToolContext, ToolError, ToolOutcome,
};
use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, ContainerSettings, DesktopSettings, FtpSettings, KubernetesSettings, LocalSettings, McpScope, Protocol,
    RiskLevel, SerialSettings, Session, SessionAuth, TelnetSettings, TerminalSettings, WebDavSettings, WslSettings,
};
use sevenshell_vault::Vault;
use tempfile::{TempDir, tempdir};

// -- Fixtures -----------------------------------------------------------

/// A call with the fewest arguments the contract allows, for each tool.
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

/// A broker that answers from a script and remembers every question it was asked.
///
/// The queue running out answers "denied": a test that expected a question and did not get one
/// should fail on the approval, not on a panic.
#[derive(Default)]
struct ScriptedBroker {
    decisions: Mutex<VecDeque<ApprovalDecision>>,
    asked: Mutex<Vec<ApprovalRequest>>,
}

impl ScriptedBroker {
    fn new(decisions: &[ApprovalDecision]) -> Arc<Self> {
        Arc::new(Self {
            decisions: Mutex::new(decisions.iter().copied().collect()),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<ApprovalRequest> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl ApprovalBroker for ScriptedBroker {
    async fn request(&self, request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        self.asked.lock().expect("not poisoned").push(request);
        Ok(self
            .decisions
            .lock()
            .expect("not poisoned")
            .pop_front()
            .unwrap_or(ApprovalDecision::Denied))
    }
}

/// A context around a vault that exists but holds no sessions, with a script, a trail and a cache of
/// its own.
fn fixture(scope: McpScope, broker: &Arc<ScriptedBroker>) -> (TempDir, Arc<Vault>, Arc<MemoryAudit>, ToolContext) {
    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("correct horse battery staple").expect("create");
    let core = Core::new(Arc::clone(&vault)).0;

    let audit = Arc::new(MemoryAudit::default());
    let context = ToolContext::new(Arc::clone(&core), scope)
        .with_approvals(Arc::clone(broker) as Arc<dyn ApprovalBroker>)
        .with_audit(Arc::clone(&audit) as Arc<dyn AuditSink>);
    (directory, vault, audit, context)
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        name: name.to_owned(),
        arguments,
    }
}

fn error_of(outcome: ToolOutcome) -> ToolError {
    match outcome {
        ToolOutcome::Failed { error } => error,
        ToolOutcome::Succeeded { result } => panic!("expected a failure, got {}", result.value),
    }
}

fn session(id: &str, protocol: Protocol) -> Session {
    Session {
        id: id.to_owned(),
        name: id.to_owned(),
        group_id: None,
        protocol,
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

// -- The scope gate -----------------------------------------------------

#[tokio::test]
async fn a_read_only_scope_refuses_every_write_tool_before_it_is_reached() {
    let broker = ScriptedBroker::new(&[]);
    let (_directory, _vault, audit, context) = fixture(McpScope::ReadOnly, &broker);
    let registry = local::default_registry();

    let writes = [
        ("sshell_session_open", json!({ "sessionId": "lab" })),
        ("sshell_exec_run", json!({ "sessionId": "lab", "command": "uptime" })),
        (
            "sshell_sftp_write_text",
            json!({ "sessionId": "lab", "path": "/tmp/note", "text": "hello" }),
        ),
        (
            "sshell_container_action",
            json!({ "sessionId": "lab", "containerId": "abc", "action": "start" }),
        ),
        (
            "sshell_batch_exec",
            json!({ "sessionIds": ["lab"], "command": "uptime" }),
        ),
    ];

    for (name, arguments) in &writes {
        let error = error_of(registry.call(&context, call(name, arguments.clone())).await);
        assert_eq!(error.code(), "POLICY-DENIED", "{name} ran under a read-only scope");
    }

    assert!(broker.asked().is_empty(), "a refused tool must not reach a person");
    let records = audit.records();
    assert_eq!(records.len(), writes.len());
    assert!(records.iter().all(|record| record.status == AuditStatus::Denied));
}

// -- The risk gate ------------------------------------------------------

#[tokio::test]
async fn a_denied_approval_refuses_the_command_and_the_audit_says_what_was_asked() {
    let broker = ScriptedBroker::new(&[ApprovalDecision::Denied]);
    let (_directory, _vault, audit, context) = fixture(McpScope::SafeWrite, &broker);

    let error = error_of(
        local::default_registry()
            .call(
                &context,
                call(
                    "sshell_exec_run",
                    json!({ "sessionId": "lab", "command": "rm -rf /srv/app" }),
                ),
            )
            .await,
    );

    assert_eq!(error.code(), "POLICY-DENIED");
    assert!(
        error.message().is_some_and(|message| message.contains("denied")),
        "the agent is told which answer it was: {:?}",
        error.message()
    );

    let asked = broker.asked();
    assert_eq!(asked.len(), 1, "one command, one question");
    assert_eq!(asked[0].tool, "sshell_exec_run");
    assert_eq!(asked[0].session_id.as_deref(), Some("lab"));
    assert_eq!(asked[0].subject.as_deref(), Some("rm -rf /srv/app"));
    assert!(matches!(
        &asked[0].reason,
        ApprovalReason::Risk { assessment } if assessment.level == RiskLevel::Destructive
    ));

    let records = audit.records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.status, AuditStatus::Denied);
    assert_eq!(record.code.as_deref(), Some("POLICY-DENIED"));
    assert_eq!(record.subject.as_deref(), Some("rm -rf /srv/app"));
    assert_eq!(
        record.risk.as_ref().map(|risk| risk.level),
        Some(RiskLevel::Destructive)
    );
    assert_eq!(
        record.approval.as_ref().map(|approval| approval.decision),
        Some(ApprovalDecision::Denied)
    );
}

#[tokio::test]
async fn an_unanswered_approval_is_a_refusal_rather_than_a_yes() {
    let broker = ScriptedBroker::new(&[ApprovalDecision::TimedOut]);
    let (_directory, _vault, audit, context) = fixture(McpScope::SafeWrite, &broker);

    let error = error_of(
        local::default_registry()
            .call(
                &context,
                call(
                    "sshell_batch_exec",
                    json!({ "sessionIds": ["lab"], "command": "rm -rf /srv/app" }),
                ),
            )
            .await,
    );

    assert_eq!(error.code(), "POLICY-DENIED");
    let records = audit.records();
    assert_eq!(
        records[0].approval.as_ref().map(|approval| approval.decision),
        Some(ApprovalDecision::TimedOut),
        "silence is recorded as silence, not as a no"
    );
}

#[tokio::test]
async fn an_approved_destructive_command_carries_the_acknowledgement_a_person_gave() {
    let broker = ScriptedBroker::new(&[ApprovalDecision::Approved]);
    let (_directory, _vault, audit, context) = fixture(McpScope::SafeWrite, &broker);

    // There is no such session, so the run is answered host-by-host: what this proves is that the
    // gate opened and the command was attempted, not that anything ran on a machine.
    let error = error_of(
        local::default_registry()
            .call(
                &context,
                call(
                    "sshell_exec_run",
                    json!({ "sessionId": "lab", "command": "rm -rf /srv/app" }),
                ),
            )
            .await,
    );

    assert_eq!(
        error.code(),
        "SESSION-NOT-FOUND",
        "an approved command has to reach the vault, not stop at the gate"
    );
    assert_eq!(broker.asked().len(), 1);

    let records = audit.records();
    assert_eq!(
        records[0].approval.as_ref().map(|approval| approval.decision),
        Some(ApprovalDecision::Approved)
    );
}

#[tokio::test]
async fn a_command_under_the_destructive_line_is_not_a_question() {
    let broker = ScriptedBroker::new(&[]);
    let (_directory, _vault, _audit, context) = fixture(McpScope::SafeWrite, &broker);

    let error = error_of(
        local::default_registry()
            .call(
                &context,
                call("sshell_exec_run", json!({ "sessionId": "lab", "command": "uptime" })),
            )
            .await,
    );

    assert_eq!(error.code(), "SESSION-NOT-FOUND");
    assert!(
        broker.asked().is_empty(),
        "a harmless command must not cost somebody a prompt"
    );
}

#[tokio::test]
async fn a_destructive_container_verb_asks_and_a_harmless_one_does_not() {
    let broker = ScriptedBroker::new(&[ApprovalDecision::Denied]);
    let (_directory, _vault, _audit, context) = fixture(McpScope::SafeWrite, &broker);

    let registry = local::default_registry();

    let error = error_of(
        registry
            .call(
                &context,
                call(
                    "sshell_container_action",
                    json!({ "sessionId": "lab", "containerId": "web", "action": "remove" }),
                ),
            )
            .await,
    );
    assert_eq!(error.code(), "POLICY-DENIED");
    assert_eq!(broker.asked().len(), 1);

    // The same tool, a verb below the line: the gate lets it through to the session lookup.
    let broker = ScriptedBroker::new(&[]);
    let (_directory, _vault, _audit, quiet) = fixture(McpScope::SafeWrite, &broker);
    let error = error_of(
        registry
            .call(
                &quiet,
                call(
                    "sshell_container_action",
                    json!({ "sessionId": "lab", "containerId": "web", "action": "start" }),
                ),
            )
            .await,
    );
    assert_eq!(error.code(), "SESSION-NOT-FOUND");
    assert!(broker.asked().is_empty());
}

// -- The arguments nobody may send --------------------------------------

#[tokio::test]
async fn an_agent_cannot_acknowledge_a_destructive_call() {
    let broker = ScriptedBroker::new(&[ApprovalDecision::Approved]);
    let (_directory, _vault, _audit, context) = fixture(McpScope::HighRisk, &broker);
    let registry = local::default_registry();

    for (name, arguments) in minimal_arguments() {
        let mut arguments = arguments.as_object().cloned().unwrap_or_default();
        arguments.insert("acknowledged".to_owned(), json!(true));

        let error = error_of(registry.call(&context, call(name, Value::Object(arguments))).await);

        assert_eq!(
            error.code(),
            "MCP-INVALID-ARGUMENTS",
            "{name} accepted an acknowledgement it never declared"
        );
    }

    assert!(
        broker.asked().is_empty(),
        "a refused argument must never reach a person as a question"
    );
}

#[tokio::test]
async fn a_blank_command_is_refused_before_anything_is_asked() {
    let broker = ScriptedBroker::new(&[]);
    let (_directory, _vault, _audit, context) = fixture(McpScope::SafeWrite, &broker);

    let error = error_of(
        local::default_registry()
            .call(
                &context,
                call("sshell_exec_run", json!({ "sessionId": "lab", "command": "   " })),
            )
            .await,
    );

    assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
    assert!(broker.asked().is_empty());
}

#[tokio::test]
async fn a_batch_is_capped_at_the_number_the_contract_states() {
    let broker = ScriptedBroker::new(&[]);
    let (_directory, _vault, _audit, context) = fixture(McpScope::SafeWrite, &broker);
    let registry = local::default_registry();

    let at_the_cap: Vec<String> = (0..32).map(|index| format!("s{index}")).collect();
    let outcome = registry
        .call(
            &context,
            call(
                "sshell_batch_exec",
                json!({ "sessionIds": at_the_cap, "command": "uptime" }),
            ),
        )
        .await;

    let ToolOutcome::Succeeded { result } = outcome else {
        panic!("thirty-two sessions is the ceiling, not a refusal: {outcome:?}");
    };
    let rows = result.value["results"].as_array().expect("one row per session");
    assert_eq!(rows.len(), 32, "every session has a row, however it turned out");
    assert!(
        rows.iter()
            .all(|row| row["ok"] == json!(false) && row["error"]["code"] == json!("SESSION-NOT-FOUND")),
        "a session that is not saved is that row's answer"
    );

    let over_the_cap: Vec<String> = (0..33).map(|index| format!("s{index}")).collect();
    let error = error_of(
        registry
            .call(
                &context,
                call(
                    "sshell_batch_exec",
                    json!({ "sessionIds": over_the_cap, "command": "uptime" }),
                ),
            )
            .await,
    );
    assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
}

#[tokio::test]
async fn a_batch_refuses_a_repeated_or_empty_session() {
    let broker = ScriptedBroker::new(&[]);
    let (_directory, _vault, _audit, context) = fixture(McpScope::SafeWrite, &broker);
    let registry = local::default_registry();

    for arguments in [
        json!({ "sessionIds": [], "command": "uptime" }),
        json!({ "sessionIds": ["lab", "lab"], "command": "uptime" }),
        json!({ "sessionIds": [" "], "command": "uptime" }),
    ] {
        let error = error_of(registry.call(&context, call("sshell_batch_exec", arguments)).await);
        assert_eq!(
            error.code(),
            "MCP-INVALID-ARGUMENTS",
            "{}",
            error.message().unwrap_or("")
        );
    }
}

#[tokio::test]
async fn a_kubernetes_path_that_is_an_address_is_refused() {
    let broker = ScriptedBroker::new(&[]);
    let (_directory, _vault, _audit, context) = fixture(McpScope::ReadOnly, &broker);

    for path in ["https://evil.example/api/v1/pods", "/api/v1/../secrets", "api/v1/pods"] {
        let error = error_of(
            local::default_registry()
                .call(
                    &context,
                    call("sshell_k8s_get", json!({ "sessionId": "lab", "path": path })),
                )
                .await,
        );
        assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS", "`{path}` was accepted");
    }
}

// -- The cache ----------------------------------------------------------

#[tokio::test]
async fn a_shared_cache_never_invents_a_session() {
    // The three families a cache holds all answer the same way when the session is not saved: the
    // vault is asked first, so nothing is cached for a session that does not exist.
    let broker = ScriptedBroker::new(&[]);
    let (_directory, vault, _audit, context) = fixture(McpScope::ReadOnly, &broker);
    vault
        .save_session(session("only-saved", Protocol::Ssh), None)
        .expect("save");
    let cache = Arc::clone(context.cache());

    for error in [
        cache.files("lab", None).await.expect_err("no such session"),
        cache.containers("lab").await.expect_err("no such session"),
        cache.k8s_get("lab", "/api/v1/pods").await.expect_err("no such session"),
    ] {
        assert_eq!(error.code(), sevenshell_error::ErrorCode::SessionNotFound, "{error}");
    }

    // Closing an empty cache is what the host does on a vault lock: it must be a no-op, not a hang.
    cache.close_all().await;
}

#[tokio::test]
async fn a_blank_path_is_refused_by_the_file_tools() {
    let broker = ScriptedBroker::new(&[]);
    let (_directory, _vault, _audit, context) = fixture(McpScope::ReadOnly, &broker);

    let error = error_of(
        local::default_registry()
            .call(
                &context,
                call("sshell_sftp_read_text", json!({ "sessionId": "lab", "path": "  " })),
            )
            .await,
    );
    assert_eq!(error.code(), "MCP-INVALID-ARGUMENTS");
}

#[test]
fn the_fixture_table_names_every_tool() {
    // A tool added later without a fixture here would make the acknowledgement test silently skip
    // it, which is exactly the kind of hole this file exists to close.
    let mut names: Vec<&str> = minimal_arguments().iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    let mut declared: Vec<String> = local::default_registry()
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect();
    declared.sort();
    assert_eq!(names, declared);
}

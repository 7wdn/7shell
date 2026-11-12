//! The access policy, as the built-in tools enforce it.
//!
//! Three rules are held still here, and each of them is a way a policy could become ornamental:
//!
//! * **A hidden session is not advertised.** `sshell_sessions_list` filters before a row is built,
//!   so neither the id nor the name of a session the policy hides leaves the process.
//! * **A hidden session is refused before any work happens.** Every tool that names a session
//!   answers `MCP-SESSION-HIDDEN` before a vault lookup, a connection or an approval. The broker is
//!   the evidence: a question it was never asked proves nothing was put to a person.
//! * **A session's own tier is enforced by the tool.** The registry only sees the widest scope the
//!   policy allows, so a session narrowed to read-only is caught by the tool's own guard — a write
//!   there is refused while a read on the same session reaches its own work.
//!
//! Nothing here needs a network: a session that was never saved fails at the vault lookup, which is
//! exactly the point a guarded call must not get past when the policy already refused it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use sevenshell_api::local;
use sevenshell_api::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, AuditRecord, AuditSink, McpAccess, ToolCall, ToolContext,
    ToolError, ToolOutcome,
};
use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, ContainerSettings, DesktopSettings, FtpSettings, KubernetesSettings, LocalSettings, McpScope, Protocol,
    SerialSettings, Session, SessionAuth, TelnetSettings, TerminalSettings, WebDavSettings, WslSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};

// -- Fixtures -----------------------------------------------------------

#[derive(Default)]
struct MemoryAudit {
    records: Mutex<Vec<AuditRecord>>,
}

impl MemoryAudit {
    fn records(&self) -> Vec<AuditRecord> {
        self.records.lock().expect("not poisoned").clone()
    }
}

impl AuditSink for MemoryAudit {
    fn record(&self, record: AuditRecord) {
        self.records.lock().expect("not poisoned").push(record);
    }
}

/// A broker that refuses everything and remembers what it was asked.
#[derive(Default)]
struct RecordingBroker {
    asked: Mutex<Vec<ApprovalRequest>>,
}

impl RecordingBroker {
    fn asked(&self) -> Vec<ApprovalRequest> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl ApprovalBroker for RecordingBroker {
    async fn request(&self, request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        self.asked.lock().expect("not poisoned").push(request);
        Ok(ApprovalDecision::Denied)
    }
}

struct Fixture {
    _directory: TempDir,
    vault: Arc<Vault>,
    audit: Arc<MemoryAudit>,
    context: ToolContext,
}

/// An unlocked vault, one policy snapshot, and a broker that would refuse anything it was asked.
/// A question in `asked()` is therefore the failure, not the answer.
fn fixture(access: McpAccess) -> (Fixture, Arc<RecordingBroker>) {
    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("correct horse battery staple").expect("create");
    let core = Core::new(Arc::clone(&vault)).0;

    let broker = Arc::new(RecordingBroker::default());
    let audit = Arc::new(MemoryAudit::default());
    let context = ToolContext::new(core, access.max_scope())
        .with_approvals(Arc::clone(&broker) as Arc<dyn ApprovalBroker>)
        .with_audit(Arc::clone(&audit) as Arc<dyn AuditSink>)
        .with_access(Arc::new(access));

    (
        Fixture {
            _directory: directory,
            vault,
            audit,
            context,
        },
        broker,
    )
}

fn policy(default_scope: McpScope, visible: Option<&[&str]>, session_scopes: &[(&str, McpScope)]) -> McpAccess {
    McpAccess {
        default_scope,
        visible_sessions: visible.map(|ids| ids.iter().map(|id| (*id).to_owned()).collect::<BTreeSet<_>>()),
        session_scopes: session_scopes
            .iter()
            .map(|(id, scope)| ((*id).to_owned(), *scope))
            .collect::<BTreeMap<_, _>>(),
    }
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

fn session(id: &str, name: &str) -> Session {
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

// -- A hidden session is not advertised ---------------------------------

#[tokio::test]
async fn a_hidden_session_is_not_listed_and_its_name_does_not_leak_in_any_form() {
    let (fixture, _broker) = fixture(policy(McpScope::ReadOnly, Some(&["visible-lab"]), &[]));
    for (id, name) in [("visible-lab", "Visible lab"), ("secret-lab", "Secret lab")] {
        fixture
            .vault
            .save_session(session(id, name), Some(SessionSecret::Password("hunter2".to_owned())))
            .expect("save");
    }

    let outcome = local::default_registry()
        .call(
            &fixture.context,
            call("sshell_sessions_list", json!({ "kind": "saved" })),
        )
        .await;
    let ToolOutcome::Succeeded { result } = outcome else {
        panic!("an allowlist is not a failure: {outcome:?}");
    };

    let rows = result.value["sessions"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "one of the two saved sessions is visible");
    assert_eq!(rows[0]["id"], json!("visible-lab"));

    let printed = result.value.to_string();
    assert!(
        !printed.contains("secret-lab"),
        "a hidden session's id reached an agent: {printed}"
    );
    assert!(
        !printed.contains("Secret lab"),
        "a hidden session's name reached an agent: {printed}"
    );

    // Hiding is not deleting: both sessions are still saved, and the policy is what filtered the
    // list. A different policy shows the other one without touching the vault.
    assert_eq!(fixture.vault.sessions().expect("unlocked").len(), 2);
}

// -- A hidden session is refused before any work ------------------------

#[tokio::test]
async fn every_session_tool_refuses_a_hidden_session_before_a_connection_or_a_question() {
    let (fixture, broker) = fixture(policy(McpScope::HighRisk, Some(&["visible-lab"]), &[]));
    let registry = local::default_registry();

    // The destructive arguments are deliberate: with the guard removed these are the calls that
    // would reach a person or a host, so the refusal has to come first for each of them.
    let calls = [
        ("sshell_session_open", json!({ "sessionId": "secret-lab" })),
        (
            "sshell_exec_run",
            json!({ "sessionId": "secret-lab", "command": "rm -rf /srv/app" }),
        ),
        (
            "sshell_batch_exec",
            json!({ "sessionIds": ["secret-lab"], "command": "rm -rf /srv/app" }),
        ),
        ("sshell_sftp_list", json!({ "sessionId": "secret-lab" })),
        (
            "sshell_sftp_read_text",
            json!({ "sessionId": "secret-lab", "path": "/etc/hostname" }),
        ),
        (
            "sshell_sftp_write_text",
            json!({ "sessionId": "secret-lab", "path": "/tmp/note", "text": "hello" }),
        ),
        ("sshell_containers_list", json!({ "sessionId": "secret-lab" })),
        (
            "sshell_container_action",
            json!({ "sessionId": "secret-lab", "containerId": "web", "action": "remove" }),
        ),
        (
            "sshell_k8s_get",
            json!({ "sessionId": "secret-lab", "path": "/api/v1/namespaces/default/pods" }),
        ),
    ];
    let checked = calls.len();

    for (name, arguments) in &calls {
        let error = error_of(registry.call(&fixture.context, call(name, arguments.clone())).await);
        assert_eq!(
            error.code(),
            "MCP-SESSION-HIDDEN",
            "{name} did not refuse a session the policy hides"
        );
        assert_eq!(error.message_key(), Some("errors.mcp.sessionHidden"));
    }

    assert!(
        broker.asked().is_empty(),
        "a hidden session must not cost a person a question: {:?}",
        broker.asked()
    );

    let records = fixture.audit.records();
    assert_eq!(records.len(), checked, "one audit line per refused call");
    assert!(
        records
            .iter()
            .all(|record| record.code.as_deref() == Some("MCP-SESSION-HIDDEN")),
        "the trail has to name the refusal, not only the tool"
    );
}

#[tokio::test]
async fn a_batch_with_one_hidden_session_is_refused_whole() {
    let (fixture, _broker) = fixture(policy(McpScope::HighRisk, Some(&["visible-lab"]), &[]));

    // Nothing in the run may happen for the visible half: a batch that touched one host and
    // reported the other as refused would already have changed something.
    let error = error_of(
        local::default_registry()
            .call(
                &fixture.context,
                call(
                    "sshell_batch_exec",
                    json!({
                        "sessionIds": ["visible-lab", "secret-lab"],
                        "command": "rm -rf /srv/app"
                    }),
                ),
            )
            .await,
    );

    assert_eq!(error.code(), "MCP-SESSION-HIDDEN");
}

// -- A session override is the tool's to enforce ------------------------

#[tokio::test]
async fn a_read_only_session_override_refuses_a_write_and_leaves_the_read_alone() {
    // The global tier offers writes, so the registry's early gate lets a write through; the narrow
    // session is exactly the case that only the tool can catch.
    let (fixture, broker) = fixture(policy(McpScope::SafeWrite, None, &[("restricted", McpScope::ReadOnly)]));
    let registry = local::default_registry();

    let error = error_of(
        registry
            .call(
                &fixture.context,
                call(
                    "sshell_sftp_write_text",
                    json!({ "sessionId": "restricted", "path": "/tmp/note", "text": "hello" }),
                ),
            )
            .await,
    );
    assert_eq!(
        error.code(),
        "POLICY-DENIED",
        "a write ran on a session narrowed to read-only"
    );

    // The same session, a read: the override has nothing to say about it, so the call reaches the
    // session lookup — the first piece of work a read does.
    let error = error_of(
        registry
            .call(
                &fixture.context,
                call(
                    "sshell_sftp_read_text",
                    json!({ "sessionId": "restricted", "path": "/etc/hostname" }),
                ),
            )
            .await,
    );
    assert_eq!(
        error.code(),
        "SESSION-NOT-FOUND",
        "the read stopped at the access gate: {error}"
    );

    // A session without an override keeps the global tier: the refusal above is about the session,
    // not about the tool.
    let error = error_of(
        registry
            .call(
                &fixture.context,
                call(
                    "sshell_sftp_write_text",
                    json!({ "sessionId": "unrestricted", "path": "/tmp/note", "text": "hello" }),
                ),
            )
            .await,
    );
    assert_eq!(
        error.code(),
        "SESSION-NOT-FOUND",
        "a write was refused on a session with no override: {error}"
    );

    assert!(
        broker.asked().is_empty(),
        "none of these calls is destructive enough to ask"
    );
}

//! The seam external tools join through.
//!
//! One table serves every caller: the built-in tools compiled into this build, and whatever a
//! runtime source offers now. What is held still here is that a name is resolved in one order, that
//! the scope gate comes before anything runs — a source's tool is not a way around the tier — and
//! that an external call leaves the same single audit line a built-in one does.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use sevenshell_api::{
    AuditRecord, AuditSink, ToolAccess, ToolCall, ToolContext, ToolDescriptor, ToolError, ToolInputSchema, ToolOutcome,
    ToolRegistry, ToolResult, ToolSource, local,
};
use sevenshell_core::Core;
use sevenshell_types::McpScope;
use sevenshell_vault::Vault;
use tempfile::{TempDir, tempdir};

fn descriptor(name: &str, access: ToolAccess) -> ToolDescriptor {
    ToolDescriptor {
        name: name.to_owned(),
        title: name.to_owned(),
        description: "a tool from an external server".to_owned(),
        input_schema: ToolInputSchema::object(json!({}), &[]),
        access,
    }
}

fn call(name: &str) -> ToolCall {
    ToolCall {
        name: name.to_owned(),
        arguments: json!({}),
    }
}

/// A source that offers a fixed list and remembers every call it was asked to run.
struct FakeSource {
    descriptors: Vec<ToolDescriptor>,
    seen: Mutex<Vec<String>>,
}

impl FakeSource {
    fn new(descriptors: Vec<ToolDescriptor>) -> Self {
        Self {
            descriptors,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl ToolSource for FakeSource {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    async fn call(
        &self,
        name: &str,
        _context: &ToolContext,
        _arguments: Value,
    ) -> Option<Result<ToolResult, ToolError>> {
        if !self.descriptors.iter().any(|descriptor| descriptor.name == name) {
            return None;
        }
        self.seen.lock().expect("not poisoned").push(name.to_owned());
        Some(Ok(ToolResult::new(json!({ "answered": name }))))
    }
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

/// A context around a vault that exists but was never unlocked, so a built-in tool that reads it
/// fails without a network or a key derivation.
fn context(scope: McpScope, audit: Arc<MemoryAudit>) -> (TempDir, ToolContext) {
    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    let core = Core::new(Arc::clone(&vault)).0;
    (directory, ToolContext::new(core, scope).with_audit(audit))
}

fn error_of(outcome: ToolOutcome) -> ToolError {
    match outcome {
        ToolOutcome::Failed { error } => error,
        ToolOutcome::Succeeded { result } => panic!("expected a failure, got {}", result.value),
    }
}

#[test]
fn a_source_joins_the_list_and_a_built_in_tool_keeps_a_shared_name() {
    let registry = local::default_registry();
    registry.add_source(Arc::new(FakeSource::new(vec![
        descriptor("mcp__files__read", ToolAccess::Read),
        // The same name a built-in tool has, spelled with a different access on purpose: if the
        // source won, the read-only test below would be gating the wrong descriptor.
        descriptor("sshell_sessions_list", ToolAccess::Write),
    ])));

    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 11, "ten built-in tools plus one new name");
    assert_eq!(registry.len(), 11);

    let names: Vec<&str> = descriptors.iter().map(|descriptor| descriptor.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "the merged list is in name order");

    let shared = descriptors
        .iter()
        .find(|descriptor| descriptor.name == "sshell_sessions_list")
        .expect("the built-in tool is still there");
    assert_eq!(
        shared.access,
        ToolAccess::Read,
        "the built-in descriptor is the one that stayed"
    );
    assert!(registry.contains("mcp__files__read"));
    assert_eq!(
        registry
            .descriptor("mcp__files__read")
            .expect("the source descriptor")
            .name,
        "mcp__files__read"
    );
}

#[tokio::test]
async fn a_call_reaches_the_source_only_after_the_built_in_list_misses() {
    let audit = Arc::new(MemoryAudit::default());
    let (_directory, context) = context(McpScope::ReadOnly, Arc::clone(&audit));

    let source = Arc::new(FakeSource::new(vec![descriptor("mcp__files__read", ToolAccess::Read)]));
    let registry = local::default_registry();
    registry.add_source(source.clone());

    // The built-in tool runs (and fails on a locked vault) instead of the source's same-named one.
    let built_in = registry.call(&context, call("sshell_sessions_list")).await;
    assert_eq!(error_of(built_in).code(), "VAULT-LOCKED");
    assert!(
        source.seen().is_empty(),
        "a built-in name must not reach a source, whatever the source declares"
    );

    let external = registry.call(&context, call("mcp__files__read")).await;
    assert!(external.is_succeeded(), "the source answered");
    assert_eq!(source.seen(), vec!["mcp__files__read".to_owned()]);

    let records = audit.records.lock().expect("not poisoned").clone();
    assert_eq!(records.len(), 2, "one line per call, successful or not");
    assert_eq!(records[1].tool, "mcp__files__read");
    assert!(
        records[1].session_id.is_none(),
        "an external call is not about a session this window opened"
    );
}

#[tokio::test]
async fn a_source_tool_is_gated_by_the_scope_before_it_runs() {
    let audit = Arc::new(MemoryAudit::default());
    let (_directory, context) = context(McpScope::ReadOnly, Arc::clone(&audit));

    let source = Arc::new(FakeSource::new(vec![descriptor(
        "mcp__files__write",
        ToolAccess::Write,
    )]));
    let registry = ToolRegistry::new();
    registry.add_source(source.clone());

    let outcome = registry.call(&context, call("mcp__files__write")).await;
    let error = error_of(outcome);
    assert_eq!(error.code(), "POLICY-DENIED");
    assert!(
        source.seen().is_empty(),
        "the gate is read before the source is entered, not after"
    );

    let records = audit.records.lock().expect("not poisoned").clone();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].status,
        sevenshell_api::AuditStatus::Denied,
        "a scope refusal is a decision, and the trail says so"
    );
}

#[tokio::test]
async fn a_switched_off_tool_is_not_offered_and_a_direct_call_is_refused_by_name() {
    let audit = Arc::new(MemoryAudit::default());
    let (_directory, context) = context(McpScope::ReadOnly, Arc::clone(&audit));
    let registry = local::default_registry();

    assert!(registry.contains("sshell_sessions_list"));
    registry.set_disabled_tools(BTreeSet::from(["sshell_sessions_list".to_owned()]));

    let names: Vec<String> = registry
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect();
    assert_eq!(names.len(), 9, "the other nine tools stay on the list");
    assert!(
        !names.iter().any(|name| name == "sshell_sessions_list"),
        "a switched-off tool must not be advertised: {names:?}"
    );
    assert!(!registry.contains("sshell_sessions_list"));
    assert!(registry.descriptor("sshell_sessions_list").is_none());

    // A stale agent that still has the name gets a refusal, not "no such tool": the tool exists and
    // somebody switched it off, which is a different sentence.
    let error = error_of(registry.call(&context, call("sshell_sessions_list")).await);
    assert_eq!(error.code(), "MCP-TOOL-DISABLED");
    assert_eq!(error.message_key(), Some("errors.mcp.toolDisabled"));
    assert_eq!(
        error.message_params().get("tool").map(String::as_str),
        Some("sshell_sessions_list")
    );

    let records = audit.records.lock().expect("not poisoned").clone();
    assert_eq!(records.len(), 1, "a refused call is still one audit line");
    assert_eq!(records[0].status, sevenshell_api::AuditStatus::Denied);
    assert_eq!(records[0].code.as_deref(), Some("MCP-TOOL-DISABLED"));
}

#[tokio::test]
async fn a_name_no_source_declares_is_still_an_unknown_tool() {
    let audit = Arc::new(MemoryAudit::default());
    let (_directory, context) = context(McpScope::HighRisk, Arc::clone(&audit));

    let registry = ToolRegistry::new();
    registry.add_source(Arc::new(FakeSource::new(vec![descriptor(
        "mcp__files__read",
        ToolAccess::Read,
    )])));

    let outcome = registry.call(&context, call("mcp__absent__read")).await;
    assert_eq!(error_of(outcome).code(), "MCP-UNKNOWN-TOOL");
}

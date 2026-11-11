//! The loop, driven by a scripted provider and fake tools.
//!
//! Nothing here touches a network, a host or a window: the provider answers with the events a test
//! wrote down, the tools answer with what the test told them to, and the context is a vault that
//! holds no sessions. That is enough to pin the six things that are easy to get subtly wrong:
//!
//! * **One tool per turn**, and every other call answered with a skip rather than quietly dropped.
//! * **The result reaching the next request**, with the id that answers the call.
//! * **The limits**: `max_turns` and `max_result_bytes`, both the host's and neither the model's.
//! * **The scope**: a read-only scope never offers a write tool, so the model is not asked to call
//!   something it would be refused for.
//! * **The tool's own gate**: `acknowledged` is an invalid argument, and an approval comes from the
//!   broker in the context, never from the loop.
//! * **The endings**: end of turn, the turn limit, cancellation and a provider failure each produce
//!   the events the window draws and keep only the turns that completed.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sevenshell_agent::{
    AI_TOOL_PREVIEW_CHARS, AI_TOOL_SKIPPED_CODE, AI_TRUNCATION_MARKER, Agent, AgentError, AgentEvent, RunOutcome,
    RunStopReason,
};
use sevenshell_ai::{
    AiProviderKind, ChatMessage, ContentBlock, Provider, ProviderError, ProviderRequest, ProviderStream, Role,
    StopReason, StreamEvent, Usage,
};
use sevenshell_api::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, Cancel, DenyingBroker, Tool, ToolAccess, ToolContext,
    ToolDescriptor, ToolError, ToolInputSchema, ToolRegistry, ToolResult, parse_arguments,
};
use sevenshell_core::Core;
use sevenshell_types::{AgentOrigin, McpScope, RiskAssessment};
use sevenshell_vault::Vault;
use tempfile::{TempDir, tempdir};

const MODEL: &str = "test-model";

// -- Fixtures -----------------------------------------------------------

/// A context around a vault that exists and holds nothing, with the assistant as its origin.
///
/// The origin is what an audit line and an approval question record, and it is the one fact the
/// host is responsible for: the loop must not have to know it was the assistant that called.
fn fixture(scope: McpScope, approvals: Arc<dyn ApprovalBroker>) -> (TempDir, ToolContext) {
    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("correct horse battery staple").expect("create");
    let core = Core::new(vault).0;
    let context = ToolContext::new(core, scope)
        .with_approvals(approvals)
        .with_origin(AgentOrigin::Ai);
    (directory, context)
}

fn registry(tools: Vec<Arc<FakeTool>>) -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool as Arc<dyn Tool>).expect("registered once");
    }
    Arc::new(registry)
}

fn agent(provider: Arc<dyn Provider>, registry: Arc<ToolRegistry>) -> Agent {
    Agent::new(provider, registry).with_model(MODEL)
}

/// Runs one conversation and returns everything the window would have been told.
async fn run(agent: &Agent, context: &ToolContext, cancel: Cancel) -> (RunOutcome, Vec<AgentEvent>) {
    let (sink, mut events) = tokio::sync::mpsc::unbounded_channel();
    let outcome = agent
        .run(
            context,
            vec![ChatMessage::user("what is this box doing?")],
            sink,
            cancel,
        )
        .await
        .expect("the run has a listener");

    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    (outcome, seen)
}

/// The result of one call, as the window was told it.
fn reported(events: &[AgentEvent], call_id: &str) -> (bool, Option<String>, String, bool) {
    for event in events {
        if let AgentEvent::ToolResult {
            call_id: id,
            ok,
            code,
            preview,
            truncated,
            ..
        } = event
            && id == call_id
        {
            return (*ok, code.clone(), preview.clone(), *truncated);
        }
    }
    panic!("no tool result for `{call_id}` in {events:?}");
}

/// The tool result of one call, as the next request would carry it.
fn carried(history: &[ChatMessage], call_id: &str) -> (String, bool) {
    for message in history {
        for block in &message.blocks {
            if let ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = block
                && tool_use_id == call_id
            {
                return (content.clone(), *is_error);
            }
        }
    }
    panic!("no tool result for `{call_id}` in the history");
}

fn failure(events: &[AgentEvent]) -> (String, String, bool) {
    for event in events {
        if let AgentEvent::Failed {
            code,
            message_key,
            retryable,
            ..
        } = event
        {
            return (code.clone(), message_key.clone(), *retryable);
        }
    }
    panic!("the run failed without saying so: {events:?}");
}

fn done(events: &[AgentEvent]) -> (RunStopReason, u32) {
    for event in events {
        if let AgentEvent::Done { stop_reason, turns, .. } = event {
            return (*stop_reason, *turns);
        }
    }
    panic!("the run ended without a `Done`: {events:?}");
}

// -- The scripted provider ---------------------------------------------

enum Script {
    /// The stream opens and answers with these items, in order.
    Events(Vec<Result<StreamEvent, ProviderError>>),
    /// The stream opens, answers with these items, and then never ends: a provider still working
    /// when somebody presses stop.
    Hang(Vec<StreamEvent>),
    /// The request is refused before anything is streamed.
    Refuse(ProviderError),
}

#[derive(Default)]
struct ScriptedProvider {
    scripts: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<ProviderRequest>>,
}

impl ScriptedProvider {
    fn new(scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into()),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<ProviderRequest> {
        self.requests.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::Local
    }

    async fn stream(&self, request: ProviderRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().expect("not poisoned").push(request);

        let script = self
            .scripts
            .lock()
            .expect("not poisoned")
            .pop_front()
            .unwrap_or(Script::Refuse(ProviderError::Protocol(
                "the script ran out: the loop asked for a turn nobody wrote".to_owned(),
            )));

        match script {
            Script::Refuse(error) => Err(error),
            Script::Events(items) => Ok(Box::pin(futures_util::stream::iter(items))),
            Script::Hang(items) => Ok(Box::pin(
                futures_util::stream::iter(items.into_iter().map(Ok).collect::<Vec<_>>())
                    .chain(futures_util::stream::pending()),
            )),
        }
    }
}

// -- The fake tool ------------------------------------------------------

/// The input every fake tool accepts. Closed, like a real one: the point of the `acknowledged`
/// test is that an argument nobody declared is a failed call rather than something ignored.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FakeInput {
    #[serde(default)]
    command: Option<String>,
}

enum Reply {
    Value(Value),
    Fail(ToolError),
    /// Asks the context for an approval first, and answers with what the person said.
    AskApproval,
}

struct FakeTool {
    name: &'static str,
    access: ToolAccess,
    reply: Reply,
    calls: Mutex<Vec<String>>,
}

impl FakeTool {
    fn new(name: &'static str, access: ToolAccess, reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            name,
            access,
            reply,
            calls: Mutex::new(Vec::new()),
        })
    }

    /// A tool that changes something and answers with one value.
    fn write(name: &'static str, value: Value) -> Arc<Self> {
        Self::new(name, ToolAccess::Write, Reply::Value(value))
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl Tool for FakeTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: self.name.to_owned(),
            title: self.name.to_owned(),
            description: format!("`{}`, for tests", self.name),
            input_schema: ToolInputSchema::object(json!({ "command": { "type": "string" } }), &[]),
            access: self.access,
        }
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: FakeInput = parse_arguments(arguments)?;
        self.calls
            .lock()
            .expect("not poisoned")
            .push(input.command.unwrap_or_default());

        match &self.reply {
            Reply::Value(value) => Ok(ToolResult::new(value.clone())),
            Reply::Fail(error) => Err(error.clone()),
            Reply::AskApproval => {
                let record = context
                    .request_approval(ApprovalRequest::risk(self.name, RiskAssessment::safe()))
                    .await?;
                if record.decision.is_approved() {
                    Ok(ToolResult::new(json!({ "approved": true })))
                } else {
                    Err(ToolError::approval_required("uptime"))
                }
            }
        }
    }
}

/// A broker that answers from a script and remembers what it was asked.
struct RecordingBroker {
    decision: ApprovalDecision,
    asked: Mutex<Vec<ApprovalRequest>>,
}

impl RecordingBroker {
    fn new(decision: ApprovalDecision) -> Arc<Self> {
        Arc::new(Self {
            decision,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<ApprovalRequest> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl ApprovalBroker for RecordingBroker {
    async fn request(&self, request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        self.asked.lock().expect("not poisoned").push(request);
        Ok(self.decision)
    }
}

// -- Stream events, in the shapes a test reads ---------------------------------

fn text(index: u32, text: &str) -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::TextDelta {
        index,
        text: text.to_owned(),
    })
}

fn tool_start(index: u32, id: &str, name: &str) -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::ToolUseStart {
        index,
        id: id.to_owned(),
        name: name.to_owned(),
    })
}

fn tool_done(index: u32, id: &str, name: &str, input: Value) -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::ToolUseDone {
        index,
        id: id.to_owned(),
        name: name.to_owned(),
        input,
    })
}

fn stop(reason: StopReason) -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::Stop { reason })
}

fn usage(input_tokens: u32, output_tokens: u32, cache_read_tokens: Option<u32>) -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::Usage(Usage {
        input_tokens,
        output_tokens,
        cache_read_tokens,
    }))
}

/// One turn that asks for a single command and waits for its result.
fn one_tool_turn(call_id: &str, command: &str) -> Script {
    Script::Events(vec![
        tool_start(0, call_id, "sshell_exec_run"),
        tool_done(0, call_id, "sshell_exec_run", json!({ "command": command })),
        stop(StopReason::ToolUse),
    ])
}

/// One turn that answers and finishes.
fn end_turn(answer: &str) -> Script {
    Script::Events(vec![text(0, answer), stop(StopReason::EndTurn)])
}

// -- One tool per turn ---------------------------------------------------

#[tokio::test]
async fn only_the_first_tool_of_a_turn_runs_and_the_rest_are_answered_as_skipped() {
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            text(0, "Trying both."),
            tool_start(1, "call-a", "sshell_exec_run"),
            tool_done(1, "call-a", "sshell_exec_run", json!({ "command": "uptime" })),
            tool_start(2, "call-b", "sshell_exec_run"),
            tool_done(2, "call-b", "sshell_exec_run", json!({ "command": "df -h" })),
            stop(StopReason::ToolUse),
        ]),
        end_turn("Done."),
    ]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "up 3 days" }));
    let agent = agent(provider, registry(vec![Arc::clone(&exec)]));
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    assert_eq!(outcome.stop_reason, RunStopReason::EndTurn);
    assert_eq!(
        exec.calls(),
        vec!["uptime".to_owned()],
        "the second call of a turn must not run"
    );

    let (ok, code, _, _) = reported(&events, "call-a");
    assert!(ok, "the first call ran and its result is drawn as one");
    assert_eq!(code, None);

    let (ok, code, preview, truncated) = reported(&events, "call-b");
    assert!(!ok, "a skipped call is not a success");
    assert_eq!(code.as_deref(), Some(AI_TOOL_SKIPPED_CODE));
    assert_eq!(preview, "", "nothing ran, so there is nothing to preview");
    assert!(!truncated);

    let (content, is_error) = carried(&outcome.history, "call-b");
    assert!(is_error, "the model has to read the skip as a failed call");
    assert!(
        content.starts_with(AI_TOOL_SKIPPED_CODE),
        "the model reads the code it can branch on: {content}"
    );
}

#[tokio::test]
async fn a_tool_result_answers_its_call_and_is_part_of_the_next_request() {
    let provider = ScriptedProvider::new(vec![one_tool_turn("call-1", "uptime"), end_turn("It is up.")]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "up 3 days" }));
    let agent = agent(Arc::clone(&provider) as Arc<dyn Provider>, registry(vec![exec]));
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (outcome, _events) = run(&agent, &context, Cancel::new()).await;
    let requests = provider.requests();

    assert_eq!(outcome.stop_reason, RunStopReason::EndTurn);
    assert_eq!(requests.len(), 2, "one request per turn");

    let first = &requests[0];
    assert_eq!(first.model, MODEL, "the agent's model is what a request names");
    assert!(
        first
            .system
            .as_deref()
            .is_some_and(|system| system.contains("One tool runs per turn")),
        "the default prompt states the rules this loop enforces: {:?}",
        first.system
    );
    assert_eq!(
        first.messages.len(),
        1,
        "the first request carries the conversation and nothing else"
    );

    let second = &requests[1];
    let assistant = second
        .messages
        .iter()
        .find(|message| message.role == Role::Assistant)
        .expect("the assistant turn is part of the next request");
    assert!(
        assistant
            .blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { id, .. } if id == "call-1")),
        "the tool call keeps its id, which is what the result answers: {assistant:?}"
    );

    let (content, is_error) = carried(&second.messages, "call-1");
    assert!(content.contains("up 3 days"), "the model reads the result: {content}");
    assert!(!is_error);
}

// -- The limits ----------------------------------------------------------

#[tokio::test]
async fn the_turn_limit_stops_the_run_without_calling_it_a_failure() {
    let provider = ScriptedProvider::new(vec![
        one_tool_turn("call-1", "uptime"),
        one_tool_turn("call-2", "uptime"),
        one_tool_turn("call-3", "uptime"),
    ]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "up" }));
    let agent = agent(provider, registry(vec![exec])).with_max_turns(2);
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    assert_eq!(outcome.stop_reason, RunStopReason::MaxTurns);
    assert_eq!(outcome.turns, 2);
    assert_eq!(
        done(&events),
        (RunStopReason::MaxTurns, 2),
        "the window localizes `maxTurns` itself: the number is all it needs"
    );
    assert!(
        !events.iter().any(|event| matches!(event, AgentEvent::Failed { .. })),
        "a limit doing its job is not a failure: {events:?}"
    );
}

#[tokio::test]
async fn a_result_over_the_byte_limit_is_cut_and_the_window_is_told() {
    let provider = ScriptedProvider::new(vec![
        one_tool_turn("call-1", "dmesg"),
        Script::Events(vec![text(0, "A long log."), stop(StopReason::EndTurn)]),
    ]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "log": "x".repeat(4_000) }));
    let agent = agent(provider, registry(vec![exec])).with_max_result_bytes(128);
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    let (ok, _, preview, truncated) = reported(&events, "call-1");
    assert!(ok);
    assert!(truncated, "the window draws a cut result differently");
    assert!(
        preview.chars().count() <= AI_TOOL_PREVIEW_CHARS,
        "a preview is bounded whatever the result was"
    );

    let (content, _) = carried(&outcome.history, "call-1");
    assert!(
        content.ends_with(AI_TRUNCATION_MARKER),
        "the cut is admitted: {content}"
    );
    assert!(
        content.len() <= 128 + AI_TRUNCATION_MARKER.len(),
        "the model is not given more than the limit, marker aside: {} bytes",
        content.len()
    );
}

#[tokio::test]
async fn usage_is_added_up_over_the_turns() {
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start(0, "call-1", "sshell_exec_run"),
            tool_done(0, "call-1", "sshell_exec_run", json!({ "command": "uptime" })),
            usage(10, 5, Some(3)),
            stop(StopReason::ToolUse),
        ]),
        Script::Events(vec![text(0, "Up."), usage(20, 7, Some(4)), stop(StopReason::EndTurn)]),
    ]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "up" }));
    let agent = agent(provider, registry(vec![exec]));
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    let expected = Usage {
        input_tokens: 30,
        output_tokens: 12,
        cache_read_tokens: Some(7),
    };
    assert_eq!(outcome.usage, Some(expected), "a run costs what its turns cost");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::Done { usage: Some(reported), .. } if *reported == expected)),
        "the window draws the same total the host is given"
    );
}

// -- The scope -----------------------------------------------------------

#[tokio::test]
async fn a_read_only_scope_offers_no_write_tool_to_the_model() {
    let provider = ScriptedProvider::new(vec![end_turn("Nothing to do.")]);
    let registry = registry(vec![
        FakeTool::new(
            "sshell_sessions_list",
            ToolAccess::Read,
            Reply::Value(json!({ "sessions": [] })),
        ),
        FakeTool::write("sshell_exec_run", json!({ "stdout": "" })),
    ]);
    let agent = agent(provider, registry);
    let (_directory, context) = fixture(McpScope::ReadOnly, Arc::new(DenyingBroker));

    let (outcome, _events) = run(&agent, &context, Cancel::new()).await;

    assert_eq!(outcome.stop_reason, RunStopReason::EndTurn);
    let offered = agent.tools(context.scope());
    let names: Vec<&str> = offered.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["sshell_sessions_list"],
        "a read-only scope must not offer what it would refuse"
    );
}

#[tokio::test]
async fn a_write_scope_offers_every_tool_in_name_order() {
    let provider = ScriptedProvider::new(vec![end_turn("Nothing to do.")]);
    let registry = registry(vec![
        FakeTool::write("sshell_exec_run", json!({ "stdout": "" })),
        FakeTool::new(
            "sshell_sessions_list",
            ToolAccess::Read,
            Reply::Value(json!({ "sessions": [] })),
        ),
    ]);
    let agent = agent(provider, registry);
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    run(&agent, &context, Cancel::new()).await;

    let offered = agent.tools(context.scope());
    let names: Vec<&str> = offered.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["sshell_exec_run", "sshell_sessions_list"],
        "the list is stable, so two turns describe the same tools"
    );
    assert!(
        offered[0].input_schema.get("additionalProperties") == Some(&json!(false)),
        "the schema the model reads is the closed one the registry enforces"
    );
    assert!(
        !offered[0].description.is_empty(),
        "a tool is described, not just named"
    );
}

// -- The tool's own gate -------------------------------------------------

#[tokio::test]
async fn an_acknowledged_argument_is_refused_by_the_tool_and_never_reaches_it() {
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start(0, "call-1", "sshell_exec_run"),
            tool_done(
                0,
                "call-1",
                "sshell_exec_run",
                json!({ "command": "rm -rf /tmp/x", "acknowledged": true }),
            ),
            stop(StopReason::ToolUse),
        ]),
        end_turn("That call was refused."),
    ]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "ran" }));
    let agent = agent(provider, registry(vec![Arc::clone(&exec)]));
    let (_directory, context) = fixture(McpScope::HighRisk, Arc::new(DenyingBroker));

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    assert!(
        exec.calls().is_empty(),
        "an argument nobody declared is an invalid call, not an approval"
    );
    let (ok, code, _, _) = reported(&events, "call-1");
    assert!(!ok);
    assert_eq!(code.as_deref(), Some("MCP-INVALID-ARGUMENTS"));

    let (content, is_error) = carried(&outcome.history, "call-1");
    assert!(is_error);
    assert!(
        content.contains("MCP-INVALID-ARGUMENTS"),
        "the model reads why its call was not run: {content}"
    );
    assert_eq!(
        outcome.stop_reason,
        RunStopReason::EndTurn,
        "a refused tool is a result the conversation continues from"
    );
}

#[tokio::test]
async fn a_denied_approval_is_a_tool_result_and_the_question_names_the_assistant() {
    let provider = ScriptedProvider::new(vec![
        one_tool_turn("call-1", "systemctl restart nginx"),
        end_turn("Refused."),
    ]);
    let exec = FakeTool::new("sshell_exec_run", ToolAccess::Write, Reply::AskApproval);
    let broker = RecordingBroker::new(ApprovalDecision::Denied);
    let agent = agent(provider, registry(vec![Arc::clone(&exec)]));
    let (_directory, context) = fixture(McpScope::HighRisk, Arc::clone(&broker) as Arc<dyn ApprovalBroker>);

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    let asked = broker.asked();
    assert_eq!(asked.len(), 1, "the tool asks, and only the tool asks");
    assert_eq!(
        asked[0].origin,
        AgentOrigin::Ai,
        "the window says which caller is waiting"
    );

    let (ok, code, _, _) = reported(&events, "call-1");
    assert!(!ok);
    assert_eq!(code.as_deref(), Some("POLICY-APPROVAL-REQUIRED"));
    assert_eq!(
        outcome.stop_reason,
        RunStopReason::EndTurn,
        "a refusal is an answer, not the end of the run"
    );
}

#[tokio::test]
async fn a_tool_that_fails_is_a_result_the_model_reads_and_the_run_continues() {
    let provider = ScriptedProvider::new(vec![
        one_tool_turn("call-1", "uptime"),
        end_turn("It would not answer."),
    ]);
    let exec = FakeTool::new(
        "sshell_exec_run",
        ToolAccess::Write,
        Reply::Fail(ToolError::new("SSH-UNREACHABLE").with_message("the host did not answer")),
    );
    let agent = agent(provider, registry(vec![exec]));
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    let (ok, code, _, _) = reported(&events, "call-1");
    assert!(!ok);
    assert_eq!(code.as_deref(), Some("SSH-UNREACHABLE"));
    let (content, is_error) = carried(&outcome.history, "call-1");
    assert!(is_error);
    assert!(content.contains("SSH-UNREACHABLE"), "{content}");
    assert_eq!(
        outcome.stop_reason,
        RunStopReason::EndTurn,
        "a tool that failed is an answer, not the end of the run"
    );
}

#[tokio::test]
async fn a_tool_this_build_does_not_have_is_an_error_result_rather_than_a_run() {
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start(0, "call-1", "sshell_no_such_tool"),
            tool_done(0, "call-1", "sshell_no_such_tool", json!({})),
            stop(StopReason::ToolUse),
        ]),
        end_turn("It does not have that tool."),
    ]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "ran" }));
    let agent = agent(provider, registry(vec![Arc::clone(&exec)]));
    let (_directory, context) = fixture(McpScope::HighRisk, Arc::new(DenyingBroker));

    let (_outcome, events) = run(&agent, &context, Cancel::new()).await;

    assert!(exec.calls().is_empty(), "no registered tool ran");
    let (ok, code, _, _) = reported(&events, "call-1");
    assert!(!ok);
    assert_eq!(code.as_deref(), Some("MCP-UNKNOWN-TOOL"));
}

#[tokio::test]
async fn a_call_assembled_from_input_fragments_still_runs() {
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start(0, "call-1", "sshell_exec_run"),
            Ok(StreamEvent::ToolInputDelta {
                index: 0,
                json: "{\"command\":".to_owned(),
            }),
            Ok(StreamEvent::ToolInputDelta {
                index: 0,
                json: "\"uptime\"}".to_owned(),
            }),
            stop(StopReason::ToolUse),
        ]),
        end_turn("Up."),
    ]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "up" }));
    let agent = agent(provider, registry(vec![Arc::clone(&exec)]));
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (outcome, _events) = run(&agent, &context, Cancel::new()).await;

    assert_eq!(
        exec.calls(),
        vec!["uptime".to_owned()],
        "the fragments are parsed when the assembled value never arrives"
    );
    assert_eq!(outcome.stop_reason, RunStopReason::EndTurn);
}

// -- The endings ---------------------------------------------------------

#[tokio::test]
async fn a_run_that_is_already_cancelled_never_reaches_the_provider() {
    let provider = ScriptedProvider::new(vec![end_turn("This must not be asked for.")]);
    let agent = agent(Arc::clone(&provider) as Arc<dyn Provider>, registry(Vec::new()));
    let (_directory, context) = fixture(McpScope::ReadOnly, Arc::new(DenyingBroker));

    let cancel = Cancel::new();
    cancel.cancel();
    let (outcome, events) = run(&agent, &context, cancel).await;

    assert_eq!(outcome.stop_reason, RunStopReason::Cancelled);
    assert_eq!(outcome.turns, 0);
    assert!(provider.requests().is_empty(), "nothing was sent");
    assert_eq!(done(&events), (RunStopReason::Cancelled, 0));
    assert_eq!(outcome.history.len(), 1, "the conversation is what it was");
}

#[tokio::test]
async fn cancelling_during_a_stream_stops_before_any_tool_runs() {
    let provider = ScriptedProvider::new(vec![Script::Hang(vec![
        StreamEvent::TextDelta {
            index: 0,
            text: "Let me look".to_owned(),
        },
        StreamEvent::ToolUseStart {
            index: 1,
            id: "call-1".to_owned(),
            name: "sshell_exec_run".to_owned(),
        },
    ])]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "ran" }));
    let agent = Arc::new(agent(provider, registry(vec![Arc::clone(&exec)])));
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (sink, mut events) = tokio::sync::mpsc::unbounded_channel();
    let cancel = Cancel::new();
    let running = {
        let agent = Arc::clone(&agent);
        let cancel = cancel.clone();
        tokio::spawn(async move {
            agent
                .run(&context, vec![ChatMessage::user("look at this")], sink, cancel)
                .await
        })
    };

    let first = events.recv().await.expect("the run says something before it waits");
    assert!(matches!(first, AgentEvent::TextDelta { .. }), "{first:?}");

    cancel.cancel();
    let outcome = running.await.expect("the task joined").expect("the run has a listener");

    assert_eq!(outcome.stop_reason, RunStopReason::Cancelled);
    assert!(
        exec.calls().is_empty(),
        "a person who pressed stop does not want a command started behind the dialog"
    );
    assert_eq!(
        outcome.history.len(),
        1,
        "a turn that never finished is not carried into the next request"
    );
}

#[tokio::test]
async fn a_provider_that_refuses_before_streaming_fails_the_run_with_its_own_code() {
    let provider = ScriptedProvider::new(vec![Script::Refuse(ProviderError::RateLimited {
        retry_after_ms: Some(2_000),
    })]);
    let agent = agent(provider, registry(Vec::new()));
    let (_directory, context) = fixture(McpScope::ReadOnly, Arc::new(DenyingBroker));

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    assert_eq!(outcome.stop_reason, RunStopReason::Error);
    assert_eq!(outcome.turns, 1);
    assert_eq!(
        failure(&events),
        (
            "AI-PROVIDER-UNAVAILABLE".to_owned(),
            "errors.ai.providerUnavailable".to_owned(),
            true
        ),
        "a rate limit is worth retrying, and the window draws the catalog's sentence"
    );
    assert_eq!(done(&events), (RunStopReason::Error, 1));
    assert_eq!(
        outcome.history.len(),
        1,
        "a failed turn is not part of the conversation the next request carries"
    );
}

#[tokio::test]
async fn an_error_in_the_middle_of_a_stream_fails_the_turn_rather_than_shortening_it() {
    let provider = ScriptedProvider::new(vec![Script::Events(vec![
        text(0, "Half an answer"),
        Err(ProviderError::Network("the connection was reset".to_owned())),
    ])]);
    let agent = agent(provider, registry(Vec::new()));
    let (_directory, context) = fixture(McpScope::ReadOnly, Arc::new(DenyingBroker));

    let (outcome, events) = run(&agent, &context, Cancel::new()).await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::TextDelta { text, .. } if text == "Half an answer")),
        "what arrived was drawn as it arrived"
    );
    assert_eq!(outcome.stop_reason, RunStopReason::Error);
    let (code, _, retryable) = failure(&events);
    assert_eq!(code, "AI-PROVIDER-UNAVAILABLE");
    assert!(retryable, "a reset connection is worth another try");
    assert_eq!(
        outcome.history.len(),
        1,
        "a half answer read as a complete one is worse than no answer"
    );
}

#[tokio::test]
async fn a_window_that_stopped_listening_ends_the_run() {
    let provider = ScriptedProvider::new(vec![end_turn("Nobody is left to read this.")]);
    let agent = agent(provider, registry(Vec::new()));
    let (_directory, context) = fixture(McpScope::ReadOnly, Arc::new(DenyingBroker));

    let (sink, receiver) = tokio::sync::mpsc::unbounded_channel();
    drop(receiver);

    let outcome = agent
        .run(&context, vec![ChatMessage::user("anyone there?")], sink, Cancel::new())
        .await;

    assert_eq!(
        outcome,
        Err(AgentError::EventsClosed),
        "a loop that kept calling tools for a closed window would be acting unobserved"
    );
}

// -- What the request says ------------------------------------------------

#[tokio::test]
async fn the_host_decides_the_instructions_and_the_output_budget() {
    let provider = ScriptedProvider::new(vec![end_turn("Short.")]);
    let agent = agent(Arc::clone(&provider) as Arc<dyn Provider>, registry(Vec::new()))
        .with_system("Answer in one line.")
        .with_max_tokens(256);
    let (_directory, context) = fixture(McpScope::ReadOnly, Arc::new(DenyingBroker));

    run(&agent, &context, Cancel::new()).await;

    let request = &provider.requests()[0];
    assert_eq!(request.system.as_deref(), Some("Answer in one line."));
    assert_eq!(request.max_tokens, 256);
    assert_eq!(request.temperature, None, "nothing sets it, so no field is written");
}

#[tokio::test]
async fn a_blank_system_prompt_is_omitted_rather_than_sent_empty() {
    let provider = ScriptedProvider::new(vec![end_turn("Short.")]);
    let agent = agent(Arc::clone(&provider) as Arc<dyn Provider>, registry(Vec::new())).with_system("  ");
    let (_directory, context) = fixture(McpScope::ReadOnly, Arc::new(DenyingBroker));

    run(&agent, &context, Cancel::new()).await;

    assert_eq!(
        provider.requests()[0].system,
        None,
        "a local server that rejects an empty system prompt is not made to"
    );
}

#[tokio::test]
async fn a_preview_is_scrubbed_even_when_the_result_is_carried_whole() {
    let provider = ScriptedProvider::new(vec![one_tool_turn("call-1", "cat /etc/app.conf"), end_turn("Read.")]);
    let exec = FakeTool::write("sshell_exec_run", json!({ "stdout": "password=hunter2 port=8080" }));
    let agent = agent(provider, registry(vec![exec]));
    let (_directory, context) = fixture(McpScope::SafeWrite, Arc::new(DenyingBroker));

    let (_outcome, events) = run(&agent, &context, Cancel::new()).await;

    let (_, _, preview, _) = reported(&events, "call-1");
    assert!(
        !preview.contains("hunter2"),
        "a preview is drawn in the window and may be copied out of it: {preview}"
    );
    assert!(preview.contains("[redacted]"), "{preview}");
    assert!(preview.contains("port=8080"), "the rest stays readable: {preview}");
}

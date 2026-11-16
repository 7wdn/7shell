//! The assistant runtime for the browser host: providers, stored conversations and streaming chat.
//!
//! This is the plain-chat half of the desktop runtime: it speaks the same provider adapters, keeps
//! conversations in the same vault document and emits the same `ai:*` events, without the tool loop
//! (which needs the MCP registry and is wired separately).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sevenshell_ai::{
    AiProviderConfig, AiProviderView, ChatMessage, ContentBlock, Provider, ProviderRequest, Role, StreamEvent,
    ToolSpec, Usage, build_provider,
};
use sevenshell_api::ToolCall;
use sevenshell_core::Core;
use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_types::{AgentOrigin, AiContextSummary, AiSessionDetail, AiSessionSummary, AiToolOutcome, AppSettings};
use sevenshell_vault::Vault;
use tokio::sync::{Mutex, broadcast};

const SESSION_VERSION: u32 = 1;
const MAX_MESSAGES: usize = 200;
const NAME_MAX_CHARS: usize = 60;

pub struct AiRuntime {
    vault: Arc<Vault>,
    events: broadcast::Sender<String>,
    runs: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    tools: Arc<crate::agent::ToolRuntime>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSession {
    #[serde(default = "session_version")]
    version: u32,
    id: String,
    #[serde(default)]
    name: String,
    created_at: u64,
    updated_at: u64,
    #[serde(default)]
    messages: Vec<ChatMessage>,
    #[serde(default)]
    outcomes: BTreeMap<String, AiToolOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_summary: Option<AiContextSummary>,
}

fn session_version() -> u32 {
    SESSION_VERSION
}

impl StoredSession {
    fn new(now: u64) -> Self {
        Self {
            version: SESSION_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            outcomes: BTreeMap::new(),
            context_summary: None,
        }
    }

    fn summary(&self) -> AiSessionSummary {
        AiSessionSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            message_count: self.messages.len() as u32,
            has_summary: self.context_summary.is_some(),
        }
    }

    fn detail(&self) -> AiSessionDetail {
        AiSessionDetail {
            summary: self.summary(),
            messages: self
                .messages
                .iter()
                .map(|message| serde_json::to_value(message).unwrap_or(Value::Null))
                .collect(),
            outcomes: self.outcomes.clone(),
            context_summary: self.context_summary.clone(),
        }
    }

    fn document(&self) -> Result<String, ShellError> {
        serde_json::to_string(self)
            .map_err(|error| ShellError::app_internal(format!("a conversation could not be written: {error}")))
    }

    fn trim(&mut self) {
        if self.messages.len() > MAX_MESSAGES {
            let drop = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(..drop);
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatContext {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    terminal_text: Option<String>,
    #[serde(default)]
    resources: Vec<ChatResource>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatResource {
    server: String,
    uri: String,
    name: String,
    #[serde(default)]
    mime_type: Option<String>,
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatStarted {
    run_id: String,
    session_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderTest {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_key: Option<&'static str>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, ShellError> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|error| ShellError::app_internal(format!("`{key}` is not the expected shape: {error}")))
}

fn optional<T: DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, ShellError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| ShellError::app_internal(format!("`{key}` is not the expected shape: {error}"))),
    }
}

fn to_value<T: Serialize>(value: T) -> Result<Value, ShellError> {
    serde_json::to_value(value)
        .map_err(|error| ShellError::app_internal(format!("an answer could not be built: {error}")))
}

fn catalog_error(code: ErrorCode) -> ShellError {
    let envelope = json!({
        "version": sevenshell_error::ENVELOPE_VERSION,
        "code": code.as_str(),
        "messageKey": code.message_key(),
        "messageParams": {},
        "origin": { "subsystem": "app" },
        "outcome": "notSent",
    });
    serde_json::from_value(envelope).unwrap_or_else(|error| {
        ShellError::app_internal(format!("an assistant failure envelope could not be built: {error}"))
    })
}

fn message_key_for(code: &str) -> &'static str {
    ErrorCode::from_code(code)
        .map(ErrorCode::message_key)
        .unwrap_or("errors.ai.providerUnavailable")
}

impl AiRuntime {
    pub fn new(
        vault: Arc<Vault>,
        _core: Arc<Core>,
        events: broadcast::Sender<String>,
        tools: Arc<crate::agent::ToolRuntime>,
    ) -> Result<Self, ShellError> {
        Ok(Self {
            vault,
            events,
            runs: Arc::new(Mutex::new(HashMap::new())),
            tools,
        })
    }

    fn providers(&self, settings: &AppSettings) -> Vec<AiProviderView> {
        settings
            .ai_providers
            .iter()
            .map(|config| AiProviderView::new(config.clone(), self.key_state(&config.id)))
            .collect()
    }

    fn key_state(&self, provider_id: &str) -> Option<bool> {
        match self.vault.ai_token(provider_id) {
            Ok(Some(_)) => Some(true),
            Ok(None) => Some(false),
            Err(_) => None,
        }
    }

    fn secret(&self, provider_id: &str) -> Result<Option<String>, ShellError> {
        Ok(self
            .vault
            .ai_token(provider_id)?
            .map(|secret| secret.expose().to_owned()))
    }

    fn emit(&self, event: &str, payload: Value) {
        let _ = self
            .events
            .send(json!({ "event": event, "payload": payload }).to_string());
    }

    pub async fn dispatch(&self, cmd: &str, args: &Value) -> Option<Result<Value, ShellError>> {
        match cmd {
            "ai_providers" => Some((|| {
                let settings = sevenshell_platform::load()?;
                to_value(self.providers(&settings))
            })()),
            "ai_provider_key_set" => Some((|| {
                let provider_id: String = arg(args, "providerId")?;
                let key: Option<String> = optional(args, "key")?;
                let settings = sevenshell_platform::load()?;
                let config = settings
                    .ai_providers
                    .iter()
                    .find(|config| config.id == provider_id)
                    .cloned()
                    .ok_or_else(|| catalog_error(ErrorCode::AiNoProvider))?;
                self.vault.set_ai_token(&provider_id, key)?;
                to_value(AiProviderView::new(config, self.key_state(&provider_id)))
            })()),
            "ai_provider_remove" => Some((|| {
                let provider_id: String = arg(args, "providerId")?;
                let mut settings = sevenshell_platform::load()?;
                self.vault.set_ai_token(&provider_id, None)?;
                settings.ai_providers.retain(|config| config.id != provider_id);
                if settings.ai_default_provider_id.as_deref() == Some(provider_id.as_str()) {
                    settings.ai_default_provider_id = None;
                }
                sevenshell_platform::save(&settings)?;
                Ok(Value::Null)
            })()),
            "ai_provider_test" => Some(self.test_provider(args).await),
            "ai_chat_send" => Some(self.start(args)),
            "ai_chat_cancel" => Some((|| {
                let run_id: Option<String> = optional(args, "runId")?;
                let runs = self.runs.clone();
                tokio::spawn(async move {
                    let runs = runs.lock().await;
                    if let Some(run_id) = run_id {
                        if let Some(flag) = runs.get(&run_id) {
                            flag.store(true, Ordering::SeqCst);
                        }
                    } else {
                        for flag in runs.values() {
                            flag.store(true, Ordering::SeqCst);
                        }
                    }
                });
                Ok(Value::Null)
            })()),
            "ai_chat_reset" => Some(Ok(Value::Null)),
            "ai_sessions_list" => Some((|| {
                let sessions = self.vault.ai_sessions()?;
                let mut rows: Vec<AiSessionSummary> = sessions
                    .values()
                    .filter_map(|document| serde_json::from_str::<StoredSession>(document).ok())
                    .map(|session| session.summary())
                    .collect();
                rows.sort_by_key(|left| std::cmp::Reverse(left.updated_at));
                to_value(rows)
            })()),
            "ai_session_get" => Some((|| {
                let session_id: String = arg(args, "sessionId")?;
                let document = self
                    .vault
                    .ai_session(&session_id)?
                    .ok_or_else(|| ShellError::app_internal("that conversation is no longer stored"))?;
                let session: StoredSession = serde_json::from_str(&document)
                    .map_err(|error| ShellError::app_internal(format!("that conversation cannot be read: {error}")))?;
                to_value(session.detail())
            })()),
            "ai_session_rename" => Some((|| {
                let session_id: String = arg(args, "sessionId")?;
                let name: String = arg(args, "name")?;
                let mut session = self.load_session(&session_id)?;
                session.name = name.trim().chars().take(NAME_MAX_CHARS).collect();
                session.updated_at = now();
                self.save_session(&session)?;
                self.emit(
                    "ai:session_saved",
                    json!({ "sessionId": session.id, "updatedAt": session.updated_at }),
                );
                to_value(session.summary())
            })()),
            "ai_session_delete" => Some((|| {
                let session_id: String = arg(args, "sessionId")?;
                self.vault.delete_ai_session(&session_id)?;
                Ok(Value::Null)
            })()),
            _ => None,
        }
    }

    async fn test_provider(&self, args: &Value) -> Result<Value, ShellError> {
        let provider_id: String = arg(args, "providerId")?;
        let settings = sevenshell_platform::load()?;
        let Some(config) = settings
            .ai_providers
            .iter()
            .find(|config| config.id == provider_id)
            .cloned()
        else {
            return to_value(ProviderTest {
                ok: false,
                code: Some(ErrorCode::AiNoProvider.as_str()),
                message_key: Some(ErrorCode::AiNoProvider.message_key()),
            });
        };
        let key = self.secret(&provider_id)?;
        let provider = match build_provider(&config, key.as_deref()) {
            Ok(provider) => provider,
            Err(error) => {
                let code = error.code();
                return to_value(ProviderTest {
                    ok: false,
                    code: Some(code),
                    message_key: Some(message_key_for(code)),
                });
            }
        };
        let request = ProviderRequest {
            model: config.model.clone(),
            system: None,
            messages: vec![ChatMessage::user("Reply with the single word: OK")],
            tools: Vec::new(),
            max_tokens: 16,
            temperature: None,
        };
        match provider.stream(request).await {
            Ok(mut stream) => {
                while let Some(item) = stream.next().await {
                    if let Err(error) = item {
                        let code = error.code();
                        return to_value(ProviderTest {
                            ok: false,
                            code: Some(code),
                            message_key: Some(message_key_for(code)),
                        });
                    }
                    if matches!(item, Ok(StreamEvent::Stop { .. })) {
                        break;
                    }
                }
                to_value(ProviderTest {
                    ok: true,
                    code: None,
                    message_key: None,
                })
            }
            Err(error) => {
                let code = error.code();
                to_value(ProviderTest {
                    ok: false,
                    code: Some(code),
                    message_key: Some(message_key_for(code)),
                })
            }
        }
    }

    fn start(&self, args: &Value) -> Result<Value, ShellError> {
        let text: String = arg(args, "text")?;
        let provider_id: Option<String> = optional(args, "providerId")?;
        let context: Option<ChatContext> = optional(args, "context")?;
        let context = context.unwrap_or_default();
        let session_id: Option<String> = optional(args, "sessionId")?.or_else(|| context.session_id.clone());

        let settings = sevenshell_platform::load()?;
        let config = match provider_id.as_deref() {
            Some(id) => settings.ai_providers.iter().find(|config| config.id == id).cloned(),
            None => settings
                .ai_default_provider_id
                .as_deref()
                .and_then(|id| settings.ai_providers.iter().find(|config| config.id == id))
                .or_else(|| settings.ai_providers.first())
                .cloned(),
        }
        .ok_or_else(|| catalog_error(ErrorCode::AiNoProvider))?;

        let session = match session_id.as_deref() {
            Some(id) if !id.is_empty() => match self.vault.ai_session(id)? {
                Some(document) => serde_json::from_str::<StoredSession>(&document).unwrap_or_else(|_| {
                    let mut session = StoredSession::new(now());
                    session.id = id.to_owned();
                    session
                }),
                None => {
                    let mut session = StoredSession::new(now());
                    session.id = id.to_owned();
                    session
                }
            },
            _ => StoredSession::new(now()),
        };

        let run_id = uuid::Uuid::new_v4().to_string();
        let started = ChatStarted {
            run_id: run_id.clone(),
            session_id: session.id.clone(),
        };
        let flag = Arc::new(AtomicBool::new(false));
        let runs = self.runs.clone();
        let vault = self.vault.clone();
        let events = self.events.clone();
        let key = self.secret(&config.id)?;
        let tools = self.tools.clone();

        tokio::spawn(async move {
            runs.lock().await.insert(run_id.clone(), flag.clone());
            run_chat(
                vault,
                events,
                run_id.clone(),
                flag,
                config,
                key,
                session,
                text,
                context,
                tools,
            )
            .await;
            runs.lock().await.remove(&run_id);
        });

        to_value(started)
    }

    fn load_session(&self, id: &str) -> Result<StoredSession, ShellError> {
        let document = self
            .vault
            .ai_session(id)?
            .ok_or_else(|| ShellError::app_internal("that conversation is no longer stored"))?;
        serde_json::from_str(&document)
            .map_err(|error| ShellError::app_internal(format!("that conversation cannot be read: {error}")))
    }

    fn save_session(&self, session: &StoredSession) -> Result<(), ShellError> {
        self.vault.save_ai_session(&session.id, Some(session.document()?))
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_chat(
    vault: Arc<Vault>,
    events: broadcast::Sender<String>,
    run_id: String,
    cancelled: Arc<AtomicBool>,
    config: AiProviderConfig,
    key: Option<String>,
    mut session: StoredSession,
    text: String,
    context: ChatContext,
    tools: Arc<crate::agent::ToolRuntime>,
) {
    let emit = |event: &str, payload: Value| {
        let _ = events.send(json!({ "event": event, "payload": payload }).to_string());
    };
    let message_id = uuid::Uuid::new_v4().to_string();

    let provider = match build_provider(&config, key.as_deref()) {
        Ok(provider) => provider,
        Err(error) => {
            let code = error.code();
            emit(
                "ai:error",
                json!({
                    "runId": run_id,
                    "code": code,
                    "messageKey": message_key_for(code),
                    "messageParams": {},
                    "retryable": error.is_retryable(),
                }),
            );
            return;
        }
    };

    if session.name.is_empty() {
        session.name = text.trim().chars().take(NAME_MAX_CHARS).collect();
    }
    session.messages.push(ChatMessage::user(text));
    session.updated_at = now();

    let mut system =
        String::from("You are 7Shell's SSH operations assistant. Answer concisely and never invent command output.");
    if let Some(terminal) = context.terminal_text.as_deref().filter(|value| !value.is_empty()) {
        system.push_str("\n\nThe user is looking at this terminal output:\n");
        system.push_str(terminal);
    }
    for resource in &context.resources {
        system.push_str(&format!(
            "\n\n<resource server=\"{}\" uri=\"{}\" name=\"{}\" type=\"{}\">\n{}\n</resource>",
            resource.server,
            resource.uri,
            resource.name,
            resource.mime_type.as_deref().unwrap_or("text/plain"),
            resource.text
        ));
    }

    let tool_specs: Vec<ToolSpec> = tools
        .descriptors()
        .into_iter()
        .map(|descriptor| ToolSpec {
            name: descriptor.name,
            description: descriptor.description,
            input_schema: serde_json::to_value(&descriptor.input_schema).unwrap_or(Value::Null),
        })
        .collect();
    let tool_context = tools.context_for_run(AgentOrigin::Ai, &run_id);
    const MAX_TURNS: u32 = 8;
    let mut turns = 0u32;
    let mut usage_total = Usage::default();
    let mut saw_usage = false;
    let mut stop = "endTurn";
    let mut failed = false;

    loop {
        if cancelled.load(Ordering::SeqCst) {
            stop = "cancelled";
            break;
        }
        if turns >= MAX_TURNS {
            stop = "maxTurns";
            break;
        }
        turns += 1;

        if let Some(split) = compaction_split(&session.messages) {
            let older: Vec<ChatMessage> = session.messages[..split].to_vec();
            let summarized = older.len() as u32;
            let model_summary = summarize(&provider, &config, &older).await;
            let degraded = model_summary.is_none();
            let content = model_summary.unwrap_or_else(|| deterministic_summary(&older));
            session.messages.drain(..split);
            let created_at = now();
            session.context_summary = Some(AiContextSummary {
                version: 1,
                content,
                through_message_id: format!("compacted-{created_at}"),
                created_at,
            });
            session.updated_at = created_at;
            emit(
                "ai:compacted",
                json!({
                    "sessionId": session.id,
                    "summarizedMessages": summarized,
                    "keptMessages": session.messages.len() as u32,
                    "degraded": degraded,
                }),
            );
        }

        let mut request_system = system.clone();
        if let Some(summary) = session.context_summary.as_ref() {
            request_system.push_str("\n\nEarlier conversation summary:\n");
            request_system.push_str(&summary.content);
        }

        let history: Vec<ChatMessage> = session.messages.iter().rev().take(40).rev().cloned().collect();
        let request = ProviderRequest {
            model: config.model.clone(),
            system: Some(request_system),
            messages: history,
            tools: tool_specs.clone(),
            max_tokens: config.max_output_tokens,
            temperature: None,
        };

        let mut stream = match provider.stream(request).await {
            Ok(stream) => stream,
            Err(error) => {
                failed = true;
                let code = error.code();
                emit(
                    "ai:error",
                    json!({
                        "runId": run_id,
                        "code": code,
                        "messageKey": message_key_for(code),
                        "messageParams": {},
                        "retryable": error.is_retryable(),
                    }),
                );
                break;
            }
        };

        let mut assistant_text = String::new();
        let mut assistant_thinking = String::new();
        let mut tool_uses: Vec<(String, String, Value)> = Vec::new();
        let mut stream_failed = false;

        while let Some(item) = stream.next().await {
            if cancelled.load(Ordering::SeqCst) {
                stop = "cancelled";
                break;
            }
            match item {
                Ok(StreamEvent::TextDelta { index, text }) => {
                    assistant_text.push_str(&text);
                    emit(
                        "ai:delta",
                        json!({
                            "runId": run_id,
                            "messageId": message_id,
                            "blockIndex": index,
                            "text": text,
                        }),
                    );
                }
                Ok(StreamEvent::ThinkingDelta { index, text }) => {
                    assistant_thinking.push_str(&text);
                    emit(
                        "ai:thinking",
                        json!({
                            "runId": run_id,
                            "messageId": message_id,
                            "blockIndex": index,
                            "text": text,
                        }),
                    );
                }
                Ok(StreamEvent::ToolUseDone { id, name, input, .. }) => {
                    tool_uses.push((id.clone(), name.clone(), input.clone()));
                    emit(
                        "ai:tool_call",
                        json!({
                            "runId": run_id,
                            "callId": id,
                            "tool": name,
                            "input": input,
                            "sessionId": session.id,
                        }),
                    );
                }
                Ok(StreamEvent::Usage(reported)) => {
                    saw_usage = true;
                    usage_total.input_tokens = usage_total.input_tokens.saturating_add(reported.input_tokens);
                    usage_total.output_tokens = usage_total.output_tokens.saturating_add(reported.output_tokens);
                    if let Some(cache) = reported.cache_read_tokens {
                        usage_total.cache_read_tokens =
                            Some(usage_total.cache_read_tokens.unwrap_or(0).saturating_add(cache));
                    }
                }
                Ok(StreamEvent::Stop { .. }) => break,
                Ok(_) => {}
                Err(error) => {
                    failed = true;
                    stream_failed = true;
                    let code = error.code();
                    emit(
                        "ai:error",
                        json!({
                            "runId": run_id,
                            "code": code,
                            "messageKey": message_key_for(code),
                            "messageParams": {},
                            "retryable": error.is_retryable(),
                        }),
                    );
                    break;
                }
            }
        }
        if stream_failed {
            break;
        }

        let mut blocks = Vec::new();
        if !assistant_thinking.is_empty() {
            blocks.push(ContentBlock::Thinking {
                text: assistant_thinking,
            });
        }
        if !assistant_text.is_empty() {
            blocks.push(ContentBlock::Text { text: assistant_text });
        }
        for (id, name, input) in &tool_uses {
            blocks.push(ContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            });
        }
        if !blocks.is_empty() {
            session.messages.push(ChatMessage {
                role: sevenshell_ai::Role::Assistant,
                blocks,
            });
        }

        if tool_uses.is_empty() {
            break;
        }

        for (call_id, tool, input) in tool_uses {
            let started = Instant::now();
            let outcome = tools
                .call(
                    &tool_context,
                    ToolCall {
                        name: tool.clone(),
                        arguments: input,
                    },
                )
                .await;
            let duration_ms = started.elapsed().as_millis() as u64;
            let (ok, code, content) = match outcome {
                sevenshell_api::ToolOutcome::Succeeded { result } => {
                    (true, None, serde_json::to_string(&result.value).unwrap_or_default())
                }
                sevenshell_api::ToolOutcome::Failed { error } => {
                    let code = error.code().to_owned();
                    let message = error.message().unwrap_or_default();
                    let content = if message.is_empty() {
                        code.clone()
                    } else {
                        format!("{code}: {message}")
                    };
                    (false, Some(code), content)
                }
            };
            let (preview, truncated) = preview_text(&content, 2_000);
            session.outcomes.insert(
                call_id.clone(),
                AiToolOutcome {
                    call_id: call_id.clone(),
                    ok,
                    code: code.clone(),
                    truncated,
                    duration_ms,
                },
            );
            session
                .messages
                .push(ChatMessage::tool_result(call_id.clone(), content, !ok));
            emit(
                "ai:tool_result",
                json!({
                    "runId": run_id,
                    "callId": call_id,
                    "tool": tool,
                    "ok": ok,
                    "code": code,
                    "preview": preview,
                    "truncated": truncated,
                    "durationMs": duration_ms,
                }),
            );
        }
    }

    if !failed {
        session.trim();
        session.updated_at = now();
        if let Ok(document) = session.document()
            && vault.save_ai_session(&session.id, Some(document)).is_ok()
        {
            emit(
                "ai:session_saved",
                json!({ "sessionId": session.id, "updatedAt": session.updated_at }),
            );
        }

        let usage = saw_usage.then(|| {
            json!({
                "inputTokens": usage_total.input_tokens,
                "outputTokens": usage_total.output_tokens,
                "cacheReadTokens": usage_total.cache_read_tokens,
            })
        });
        emit(
            "ai:done",
            json!({
                "runId": run_id,
                "stopReason": stop,
                "turns": turns,
                "usage": usage,
            }),
        );
    }
}

fn preview_text(text: &str, limit: usize) -> (String, bool) {
    let mut preview = String::new();
    for (index, character) in text.chars().enumerate() {
        if index >= limit {
            return (preview, true);
        }
        preview.push(character);
    }
    (preview, false)
}

const COMPACT_TRIGGER_CHARS: usize = 24_000;
const COMPACT_KEEP_MESSAGES: usize = 12;

fn message_text(message: &ChatMessage) -> String {
    message
        .blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } | ContentBlock::Thinking { text } => text.clone(),
            ContentBlock::ToolUse { name, .. } => format!("[tool {name}]"),
            ContentBlock::ToolResult { content, .. } => content.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compaction_split(messages: &[ChatMessage]) -> Option<usize> {
    if messages.len() <= COMPACT_KEEP_MESSAGES {
        return None;
    }
    let chars: usize = messages
        .iter()
        .map(|message| message_text(message).chars().count())
        .sum();
    if chars < COMPACT_TRIGGER_CHARS {
        return None;
    }
    let mut split = messages.len().saturating_sub(COMPACT_KEEP_MESSAGES);
    while split > 0 {
        let message = &messages[split];
        if message.role == Role::User
            && message
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::Text { .. }))
        {
            break;
        }
        split -= 1;
    }
    (split > 0).then_some(split)
}

async fn summarize(provider: &Arc<dyn Provider>, config: &AiProviderConfig, older: &[ChatMessage]) -> Option<String> {
    let transcript = older
        .iter()
        .map(|message| {
            let role = if message.role == Role::User {
                "user"
            } else {
                "assistant"
            };
            format!("{role}: {}", message_text(message))
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let transcript: String = transcript.chars().take(16_000).collect();
    let request = ProviderRequest {
        model: config.model.clone(),
        system: Some(
            "Condense the earlier part of this SSH operations conversation. Keep hosts, commands, \
             errors and decisions; drop pleasantries. Answer with the summary only."
                .to_owned(),
        ),
        messages: vec![ChatMessage::user(transcript)],
        tools: Vec::new(),
        max_tokens: config.max_output_tokens.min(800),
        temperature: None,
    };
    let mut stream = provider.stream(request).await.ok()?;
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::TextDelta { text: delta, .. }) => text.push_str(&delta),
            Ok(StreamEvent::Stop { .. }) => break,
            Err(_) => return None,
            _ => {}
        }
    }
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn deterministic_summary(older: &[ChatMessage]) -> String {
    let mut summary = String::from("Earlier exchanges (condensed):\n");
    for message in older {
        let text = message_text(message).split_whitespace().collect::<Vec<_>>().join(" ");
        let snippet: String = text.chars().take(300).collect();
        let role = if message.role == Role::User {
            "user"
        } else {
            "assistant"
        };
        summary.push_str(&format!("- {role}: {snippet}\n"));
        if summary.chars().count() > 8_000 {
            break;
        }
    }
    summary
}

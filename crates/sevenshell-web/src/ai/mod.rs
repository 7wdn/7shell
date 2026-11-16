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

mod context;

const SESSION_VERSION: u32 = 1;
const MAX_MESSAGES: usize = 200;
const MAX_SESSION_BYTES: usize = 512 * 1024;
const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
const MAX_SESSIONS: usize = 50;
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

    fn size_bytes(&self) -> usize {
        self.document().map(|document| document.len()).unwrap_or(usize::MAX)
    }

    fn trim_to_limits(&mut self) -> Result<(), ShellError> {
        while self.messages.len() > MAX_MESSAGES || self.size_bytes() > MAX_SESSION_BYTES {
            if self.messages.len() <= 1 {
                break;
            }
            let cut = oldest_exchange_end(&self.messages);
            self.messages.drain(..cut.max(1));
            prune_outcomes(self);
        }
        if self.size_bytes() > MAX_SESSION_BYTES {
            return Err(catalog_error(ErrorCode::AiSessionTooLarge));
        }
        Ok(())
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
                let settings = sevenshell_platform::load()?;
                to_value(list_stored(&self.vault, &settings, now())?)
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
                let cleaned = clean_name(&name);
                if !cleaned.is_empty() {
                    session.name = cleaned;
                }
                session.updated_at = now();
                save_stored(&self.vault, &mut session)?;
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
}

fn starts_exchange(message: &ChatMessage) -> bool {
    message.role == Role::User
        && message
            .blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::Text { .. }))
}

fn oldest_exchange_end(messages: &[ChatMessage]) -> usize {
    let mut seen = 0;
    for (index, message) in messages.iter().enumerate() {
        if starts_exchange(message) {
            seen += 1;
            if seen == 2 {
                return index;
            }
        }
    }
    messages.len().saturating_sub(1).max(1)
}

fn prune_outcomes(session: &mut StoredSession) {
    let mut referenced = std::collections::BTreeSet::new();
    for message in &session.messages {
        for block in &message.blocks {
            if let ContentBlock::ToolUse { id, .. } = block {
                referenced.insert(id.as_str());
            }
        }
    }
    session
        .outcomes
        .retain(|call_id, _| referenced.contains(call_id.as_str()));
}

fn clean_name(text: &str) -> String {
    sevenshell_error::scrub(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(NAME_MAX_CHARS)
        .collect()
}

fn save_stored(vault: &Vault, session: &mut StoredSession) -> Result<(), ShellError> {
    session.trim_to_limits()?;
    let document = session.document()?;
    if document.len() > MAX_SESSION_BYTES {
        return Err(catalog_error(ErrorCode::AiSessionTooLarge));
    }
    make_room(vault, &session.id, document.len())?;
    vault.save_ai_session(&session.id, Some(document))
}

fn make_room(vault: &Vault, id: &str, size: usize) -> Result<(), ShellError> {
    let stored = vault.ai_sessions()?;
    let mut others: Vec<(String, u64, usize)> = stored
        .iter()
        .filter(|(other, _)| other.as_str() != id)
        .map(|(other, document)| {
            let updated_at = serde_json::from_str::<StoredSession>(document)
                .map(|session| session.updated_at)
                .unwrap_or(0);
            (other.clone(), updated_at, document.len())
        })
        .collect();

    if size > MAX_TOTAL_BYTES {
        return Err(catalog_error(ErrorCode::AiSessionTooLarge));
    }

    let mut count = others.len() + 1;
    let mut total = others.iter().map(|(_, _, length)| length).sum::<usize>() + size;
    if count <= MAX_SESSIONS && total <= MAX_TOTAL_BYTES {
        return Ok(());
    }

    others.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    for (other, _, length) in others {
        if count <= MAX_SESSIONS && total <= MAX_TOTAL_BYTES {
            break;
        }
        vault.delete_ai_session(&other)?;
        count -= 1;
        total -= length;
    }
    Ok(())
}

fn list_stored(vault: &Vault, settings: &AppSettings, now: u64) -> Result<Vec<AiSessionSummary>, ShellError> {
    let stored = vault.ai_sessions()?;
    let retention_days = settings.ai_session_retention_days.clamp(1, 365) as u64;
    let oldest = now.saturating_sub(retention_days * 86_400);

    let mut rows = Vec::with_capacity(stored.len());
    for (id, document) in stored {
        let Ok(session) = serde_json::from_str::<StoredSession>(&document) else {
            continue;
        };
        if session.updated_at < oldest {
            if let Err(error) = vault.delete_ai_session(&id) {
                log::warn!("an assistant conversation older than the retention stayed: {error}");
            }
            continue;
        }
        rows.push(session.summary());
    }
    rows.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(rows)
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
        session.name = clean_name(&text);
    }
    session.messages.push(ChatMessage::user(text));
    session.updated_at = now();

    let system = context::system_prompt(&config, Some(&context));

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
            let (preview, truncated) = preview_text(&content, 4_000);
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
        session.updated_at = now();
        match save_stored(&vault, &mut session) {
            Ok(()) => emit(
                "ai:session_saved",
                json!({ "sessionId": session.id, "updatedAt": session.updated_at }),
            ),
            Err(error) => log::warn!("an assistant conversation could not be stored: {error}"),
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
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= limit {
        return (text.to_owned(), false);
    }
    let head = limit * 3 / 4;
    let tail = limit - head;
    let omitted = characters.len() - head - tail;
    let mut preview: String = characters[..head].iter().collect();
    preview.push_str(&format!(
        "
… [{omitted} characters omitted] …
"
    ));
    preview.extend(characters[characters.len() - tail..].iter());
    (preview, true)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            role,
            blocks: vec![ContentBlock::Text { text: text.to_owned() }],
        }
    }

    #[test]
    fn trimming_cuts_whole_exchanges_and_keeps_the_newest() {
        let mut session = StoredSession::new(1_700_000_000);
        for index in 0..105 {
            session.messages.push(message(Role::User, &format!("question {index}")));
            session
                .messages
                .push(message(Role::Assistant, &format!("answer {index}")));
        }
        session.trim_to_limits().expect("a short conversation fits");
        assert!(session.messages.len() <= MAX_MESSAGES);
        assert!(
            starts_exchange(&session.messages[0]),
            "a trimmed transcript must begin at an exchange, not inside one"
        );
        assert!(
            session
                .messages
                .last()
                .is_some_and(|message| message_text(message).contains("answer 104")),
            "the newest exchange survives"
        );
    }

    #[test]
    fn a_tool_preview_keeps_both_ends_and_counts_what_it_dropped() {
        let (short, truncated) = preview_text("short", 100);
        assert_eq!(short, "short");
        assert!(!truncated);

        let text = format!("{}{}{}", "a".repeat(50), "middle", "z".repeat(50));
        let (preview, truncated) = preview_text(&text, 40);
        assert!(truncated);
        assert!(preview.starts_with("aaaaaaaa"));
        assert!(preview.ends_with("zzzzzzzz"));
        assert!(preview.contains("characters omitted"));
        assert!(preview.chars().count() < text.chars().count());
    }

    #[test]
    fn a_conversation_name_is_scrubbed_and_one_line() {
        let name = clean_name("why\npassword=hunter2  is this");
        assert!(!name.contains('\n'));
        assert!(
            !name.contains("hunter2"),
            "a pasted credential must not become a list label"
        );
    }

    #[test]
    fn retention_deletes_conversations_older_than_the_setting() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let vault = Vault::open_at(directory.path().join("vault.json"));
        vault.create("master").expect("a vault");
        let now = 1_700_000_000u64;

        let mut old = StoredSession::new(now - 31 * 86_400);
        old.name = "old".to_owned();
        vault
            .save_ai_session(&old.id, Some(old.document().expect("document")))
            .expect("store old");
        let mut fresh = StoredSession::new(now - 2 * 86_400);
        fresh.name = "fresh".to_owned();
        vault
            .save_ai_session(&fresh.id, Some(fresh.document().expect("document")))
            .expect("store fresh");

        let settings = AppSettings {
            ai_session_retention_days: 30,
            ..AppSettings::default()
        };
        let rows = list_stored(&vault, &settings, now).expect("list");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, fresh.id);
        assert!(vault.ai_session(&old.id).expect("read").is_none());
    }
}

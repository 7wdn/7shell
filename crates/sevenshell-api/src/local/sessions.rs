//! What this installation knows about, without connecting to anything — and opening one of them.
//!
//! Two tools on purpose. Listing is a read that never touches a server; opening is the one call in
//! this interface whose subject is a *host key*, and the order it enforces is the project's: probe
//! without sending anything, compare with what is trusted, and only then authenticate. An unknown key
//! is a decision for a person; a changed key is a stop.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use sevenshell_core::OPEN_TIMEOUT;
use sevenshell_error::ShellError;
use sevenshell_types::{OpenOutcome, Protocol};

use crate::context::{ApprovalRecord, ToolContext};
use crate::registry::Tool;
use crate::tool::{ToolAccess, ToolAudit, ToolDescriptor, ToolError, ToolInputSchema, ToolResult, parse_arguments};

use super::cache::saved_session;
use super::gate;
use super::session_id;

pub const NAME: &str = "sshell_sessions_list";
pub const OPEN: &str = "sshell_session_open";

/// Which list is wanted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionsListKind {
    /// The sessions saved in this installation.
    #[default]
    Saved,
    /// The saved sessions that have a terminal open right now.
    Open,
}

impl SessionsListKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Open => "open",
        }
    }
}

/// What `sshell_sessions_list` takes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionsListInput {
    #[serde(default)]
    pub kind: SessionsListKind,
}

/// One saved session, as an agent is allowed to see it.
///
/// A smaller shape than the wire type on purpose: an agent needs to know which session to ask about
/// next, and nothing else here is any of its business — no authentication method, no key reference,
/// no terminal settings, no saved forwards. Nothing in it is secret either way, which is the rule a
/// type is allowed to be handed to an agent under.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedSession {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<String>,
    protocol: Protocol,
    host: String,
    port: u16,
    user: String,
    updated_at: u64,
}

pub struct SessionsListTool;

#[async_trait]
impl Tool for SessionsListTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: NAME.to_owned(),
            title: "List sessions".to_owned(),
            description: "Lists the sessions this installation knows about. `saved` answers with \
                          the saved connections — identifier, name, protocol, address and user — and \
                          `open` answers with the identifiers of the saved sessions that have a \
                          terminal running right now. No password, no key and no terminal handle is \
                          in the answer."
                .to_owned(),
            input_schema: ToolInputSchema::object(
                json!({
                    "kind": {
                        "type": "string",
                        "enum": ["saved", "open"],
                        "default": "saved",
                        "description": "Which list to read. Absent reads as `saved`."
                    }
                }),
                &[],
            ),
            access: ToolAccess::Read,
        }
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: SessionsListInput = parse_arguments(arguments)?;

        let value = match input.kind {
            SessionsListKind::Saved => {
                // A locked vault answers with its own code, which is the diagnosable answer a bridge
                // is asked for: the desktop is running, and nobody has unlocked it yet.
                let sessions = context.core().vault().sessions().map_err(ToolError::from)?;
                let rows: Vec<SavedSession> = sessions
                    .into_iter()
                    .map(|session| SavedSession {
                        id: session.id,
                        name: session.name,
                        group_id: session.group_id,
                        protocol: session.protocol,
                        host: session.host,
                        port: session.port,
                        user: session.user,
                        updated_at: session.updated_at,
                    })
                    .collect();
                json!({ "kind": SessionsListKind::Saved.as_str(), "sessions": rows })
            }
            SessionsListKind::Open => {
                let terminals = context.core().open_terminals().await;
                // Identifiers rather than handles: a handle is how this process names a terminal,
                // and an agent gets to name the session it acts on, never a live object.
                let mut ids: Vec<String> = terminals.into_iter().map(|open| open.session_id).collect();
                ids.sort();
                ids.dedup();
                let rows: Vec<Value> = ids
                    .into_iter()
                    .map(|session_id| json!({ "sessionId": session_id }))
                    .collect();
                json!({ "kind": SessionsListKind::Open.as_str(), "sessions": rows })
            }
        };

        Ok(ToolResult::new(value))
    }
}

// -- Opening a session --------------------------------------------------

/// What `sshell_session_open` takes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionOpenInput {
    pub session_id: String,
}

pub fn session_open_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: OPEN.to_owned(),
        title: "Open a session".to_owned(),
        description: "Opens a terminal for a saved session and keeps it open for the calls that \
                      follow. The host key is verified before any credential is sent: an unknown key \
                      has to be trusted first, and a changed key stops the open rather than asking. \
                      Opening is a write, so a read-only scope refuses it."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({ "sessionId": string_of("The id of a saved session, from sshell_sessions_list.") }),
            &["sessionId"],
        ),
        access: ToolAccess::Write,
    }
}

fn string_of(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

pub struct SessionOpenTool;

#[async_trait]
impl Tool for SessionOpenTool {
    fn descriptor(&self) -> ToolDescriptor {
        session_open_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: SessionOpenInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;

        let session = saved_session(context.core(), session_id);
        let name = session.as_ref().map(|session| session.name.clone());
        // What an audit line remembers about an open: the address, which is the thing the host key
        // was about. The credential is not read here and never becomes a subject.
        let subject = session
            .as_ref()
            .map(|session| gate::subject(&format!("{}:{}", session.host, session.port)));

        let audit = |approval: Option<ApprovalRecord>| {
            let mut audit = ToolAudit::for_session(session_id);
            if let Some(subject) = &subject {
                audit = audit.with_subject(subject.clone());
            }
            if let Some(approval) = approval {
                audit = audit.with_approval(approval);
            }
            audit
        };

        // A terminal that is already up is not opened twice: the fact an agent needs is "it is
        // open", and a second terminal nobody closes is a connection the person did not ask for.
        if is_open(context, session_id).await {
            return Ok(ToolResult::new(json!({
                "sessionId": session_id,
                "opened": true,
                "alreadyOpen": true,
                "tunnels": [],
                "tunnelFailures": [],
            }))
            .with_audit(audit(None)));
        }

        // One attempt may take the core's whole open budget, and a route of several hops spends one
        // attempt per hop. Twice that is a backstop around the core's own timeout rather than a
        // second, tighter limit competing with it.
        let budget = OPEN_TIMEOUT.saturating_mul(2);
        let mut approval: Option<ApprovalRecord> = None;
        let mut trusted_a_key = false;

        loop {
            let outcome = gate::bounded(context, budget, async {
                context.core().open_terminal(session_id).await.map_err(ToolError::from)
            })
            .await?;

            match outcome {
                OpenOutcome::Opened {
                    tunnels,
                    tunnel_failures,
                    ..
                } => {
                    return Ok(ToolResult::new(json!({
                        "sessionId": session_id,
                        "opened": true,
                        "alreadyOpen": false,
                        // No handle: a handle is this process's name for a terminal, and what an
                        // agent acts on is a session id.
                        "tunnels": tunnels.iter().map(tunnel_row).collect::<Vec<Value>>(),
                        "tunnelFailures": tunnel_failures.iter().map(failure_row).collect::<Vec<Value>>(),
                    }))
                    .with_audit(audit(approval)));
                }
                OpenOutcome::HostKeyUnknown { key } => {
                    if trusted_a_key {
                        // One key was trusted and the next hop presents its own. Asking again in the
                        // same call would show a person a fingerprint they did not get to compare,
                        // so the answer names the host instead.
                        return Err(ToolError::from(ShellError::ssh_host_key_unknown(
                            &key.host,
                            key.port,
                            &key.key_type,
                        ))
                        .with_message(format!(
                            "{}:{} presents a host key that is still unknown; trust it and open again",
                            key.host, key.port
                        ))
                        .with_audit(audit(approval)));
                    }

                    let record = gate::approve_host_key(context, OPEN, &key, Some((session_id, name.clone()))).await?;
                    // The trust is written before the retry: opening again without it would present
                    // the same unknown key and be refused for a reason nobody would see.
                    context.core().trust_host_key(&key)?;
                    approval = Some(record);
                    trusted_a_key = true;
                }
                // A key that changed is the attack host keys exist to catch. No question is put to
                // anybody and nothing is sent.
                OpenOutcome::HostKeyMismatch { key, trusted } => {
                    return Err(ToolError::from(ShellError::ssh_host_key_mismatch(
                        &key.host,
                        key.port,
                        &key.key_type,
                    ))
                    .with_message(format!(
                        "the host key of {}:{} changed: {} was presented where {} is trusted. Nothing was sent.",
                        key.host, key.port, key.fingerprint, trusted.fingerprint
                    ))
                    .with_audit(audit(approval)));
                }
            }
        }
    }
}

/// Whether this session already has a terminal.
async fn is_open(context: &ToolContext, session_id: &str) -> bool {
    context
        .core()
        .open_terminals()
        .await
        .iter()
        .any(|open| open.session_id == session_id)
}

fn tunnel_row(tunnel: &sevenshell_types::OpenTunnel) -> Value {
    json!({
        "bindHost": tunnel.bind_host,
        "bindPort": tunnel.bind_port,
        "mode": tunnel.mode,
        "targetHost": tunnel.target_host,
        "targetPort": tunnel.target_port,
        "connections": tunnel.connections,
    })
}

fn failure_row(failure: &sevenshell_types::TunnelFailure) -> Value {
    json!({
        "forwardId": failure.forward_id,
        "bindHost": failure.bind_host,
        "bindPort": failure.bind_port,
        "detail": failure.detail,
    })
}

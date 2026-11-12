//! Containers, from an agent's point of view.
//!
//! The runtime is named by a saved container session, and the connection is the cache's: an agent
//! that lists containers and then stops one is talking to the same runtime, and connecting twice
//! would be two authentications for one conversation.
//!
//! The verbs are a closed list rather than a free string. Every one of them has a verdict in the
//! policy crate, and a verb nobody wrote a rule for would arrive as "caution" — which is an answer
//! to a question an agent should not have been able to ask.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::context::ToolContext;
use crate::registry::Tool;
use crate::tool::{ToolAccess, ToolAudit, ToolDescriptor, ToolError, ToolInputSchema, ToolResult, parse_arguments};

use super::cache::saved_session;
use super::gate;
use super::session_id;

pub const CONTAINERS_LIST: &str = "sshell_containers_list";
pub const CONTAINER_ACTION: &str = "sshell_container_action";

// -- Inputs -------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainersListInput {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerActionInput {
    pub session_id: String,
    pub container_id: String,
    pub action: ContainerAction,
}

/// The verbs a container may be asked to do. A closed list rather than a free string: these are the
/// ones the policy crate has a verdict for, and an unknown verb would arrive as `Caution` — which is
/// an answer to a question nobody should have been able to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    Kill,
    Remove,
}

impl ContainerAction {
    pub const ALL: &'static [ContainerAction] = &[
        ContainerAction::Start,
        ContainerAction::Stop,
        ContainerAction::Restart,
        ContainerAction::Pause,
        ContainerAction::Unpause,
        ContainerAction::Kill,
        ContainerAction::Remove,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Pause => "pause",
            Self::Unpause => "unpause",
            Self::Kill => "kill",
            Self::Remove => "remove",
        }
    }
}

// -- Descriptors --------------------------------------------------------

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn string_enum(description: &str, values: &[&str]) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}

fn session_id_property() -> Value {
    string("The id of a saved session, from sshell_sessions_list.")
}

pub fn containers_list_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: CONTAINERS_LIST.to_owned(),
        title: "List containers".to_owned(),
        description: "Lists the containers a saved container session's runtime reports: id, name, \
                      image, state and the ports the runtime printed."
            .to_owned(),
        input_schema: ToolInputSchema::object(json!({ "sessionId": session_id_property() }), &["sessionId"]),
        access: ToolAccess::Read,
    }
}

pub fn container_action_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: CONTAINER_ACTION.to_owned(),
        title: "Act on a container".to_owned(),
        description: "Starts, stops, restarts, pauses, unpauses, kills or removes one container. \
                      Stopping, killing and removing are graded destructive and stop for a person's \
                      approval on the desktop."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "containerId": string("The container's id or name, as sshell_containers_list reported it."),
                "action": string_enum(
                    "What to do with it.",
                    &["start", "stop", "restart", "pause", "unpause", "kill", "remove"]
                )
            }),
            &["sessionId", "containerId", "action"],
        ),
        access: ToolAccess::Write,
    }
}

// -- The tools ----------------------------------------------------------

pub struct ContainersListTool;

#[async_trait]
impl Tool for ContainersListTool {
    fn descriptor(&self) -> ToolDescriptor {
        containers_list_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: ContainersListInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        // Before the runtime connection is opened: the session's own tier is what decides whether
        // this runtime is reachable at all.
        context.guard_session(session_id, ToolAccess::Read)?;

        let containers = gate::bounded(context, gate::READ_TIMEOUT, async {
            context.cache().containers(session_id).await.map_err(ToolError::from)
        })
        .await?;

        let (rows, total, truncated) = gate::rows(&containers);

        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "containers": rows,
            "total": total,
            "truncated": truncated,
        }))
        .with_audit(ToolAudit::for_session(session_id)))
    }
}

pub struct ContainerActionTool;

#[async_trait]
impl Tool for ContainerActionTool {
    fn descriptor(&self) -> ToolDescriptor {
        container_action_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: ContainerActionInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        let container_id = input.container_id.trim();
        if container_id.is_empty() {
            return Err(ToolError::invalid_arguments("containerId is empty"));
        }
        // Before the verdict is read and before anybody is asked: an action on a session this
        // caller may not see never becomes a question, let alone a command.
        context.guard_session(session_id, ToolAccess::Write)?;

        let action = input.action;
        let subject = gate::subject(&format!("{} {container_id}", action.as_str()));
        let verdict = sevenshell_policy::container_action(action.as_str());

        let mut approval = None;
        let mut acknowledged = false;
        if verdict.needs_approval() {
            let name = saved_session(context.core(), session_id).map(|session| session.name);
            approval = Some(
                gate::approve_risk(context, CONTAINER_ACTION, &verdict, Some((session_id, name)), &subject).await?,
            );
            acknowledged = true;
        }

        gate::bounded(context, gate::READ_TIMEOUT, async {
            context
                .cache()
                .container_action(session_id, container_id, action.as_str(), acknowledged)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        let mut audit = ToolAudit::for_session(session_id)
            .with_subject(subject)
            .with_risk(verdict);
        if let Some(record) = approval {
            audit = audit.with_approval(record);
        }

        Ok(ToolResult::new(json!({
            "sessionId": session_id,
            "containerId": container_id,
            "action": action.as_str(),
            "acknowledged": acknowledged,
        }))
        .with_audit(audit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verbs_the_policy_grades_destructive_are_the_ones_that_ask() {
        for action in ContainerAction::ALL {
            let destructive = matches!(
                action,
                ContainerAction::Stop | ContainerAction::Kill | ContainerAction::Remove
            );
            assert_eq!(
                sevenshell_policy::container_action(action.as_str()).needs_approval(),
                destructive,
                "`{}` is graded {}",
                action.as_str(),
                sevenshell_policy::container_action(action.as_str()).level
            );
        }
    }
}

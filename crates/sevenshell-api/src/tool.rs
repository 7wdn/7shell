//! What a tool is called, what it takes, and what it answers.
//!
//! These types are the frozen half of the agent interface. The MCP bridge, the desktop host and
//! anything that calls a tool later speak them, so a change here changes the contract rather than
//! one caller.
//!
//! **A tool never takes an approval.** `acknowledged` is deliberately absent from every input, and
//! every input refuses unknown fields, so an agent cannot decide on its own that a destructive
//! command has been agreed to. Approval is produced by the host's own window and nowhere else.
//!
//! Two kinds of text travel on a failure, and they are for two different readers: `message` is a
//! plain sentence for the agent (MCP has no locale to negotiate with), while `message_key` and its
//! parameters are the catalog's key for anything the *interface* draws. Nothing user-facing is
//! spelled out here.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sevenshell_types::RiskAssessment;

use crate::context::ApprovalRecord;

/// How much a tool is allowed to change.
///
/// A property of the tool, not of one call: what varies per call is how dangerous a *command* is,
/// which the policy classifier answers. This decides whether the tool is offered at all under the
/// configured scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolAccess {
    /// Look, do not touch: a listing or a read.
    Read,
    /// Something changes: a command runs, a file is written, a container moves.
    Write,
}

/// What the MCP client is told about one tool.
///
/// `description` is prompt material for the agent, not interface copy: there is no locale in an MCP
/// handshake, and a description translated per installation would make two agents see two different
/// tools. Anything a *person* reads about the same action is drawn by the interface from the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    /// The `sshell_`-prefixed name the agent calls. Stable for the life of the tool: an agent may
    /// have it in a prompt long after this build is gone.
    pub name: String,
    /// A short name for a window that lists tools.
    pub title: String,
    /// What the tool does, in the words the agent is given.
    pub description: String,
    /// A JSON Schema object describing the arguments.
    pub input_schema: ToolInputSchema,
    pub access: ToolAccess,
}

/// A JSON Schema object for one tool's arguments.
///
/// A newtype around the schema itself rather than a schema language: the shapes these tools take
/// are small, and a hand-written object is one thing to read instead of a builder plus its output.
/// It is always an *object* schema with `additionalProperties: false`, which is what makes the
/// schema and [``parse_arguments`] tell the same story about unknown fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolInputSchema(Value);

impl ToolInputSchema {
    /// A closed object schema.
    pub fn object(properties: Value, required: &[&str]) -> Self {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": false,
        });
        if !required.is_empty() {
            schema["required"] = serde_json::json!(required);
        }
        Self(schema)
    }

    /// The schema as it goes on the wire.
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// One declared property, when the schema has it.
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.0.get("properties")?.get(name)
    }

    /// Every property name the schema declares.
    pub fn property_names(&self) -> impl Iterator<Item = &str> {
        self.0
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|properties| properties.keys())
            .map(String::as_str)
    }

    /// The properties the schema says are required.
    pub fn required(&self) -> impl Iterator<Item = &str> {
        self.0
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    }
}

/// One call: the tool's name and the arguments as the agent sent them.
///
/// This is what crosses the control endpoint, so it is also what the desktop validates. It carries
/// no acknowledgement and has no field for one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCall {
    pub name: String,
    /// The arguments object. Absent reads as an empty object: a tool with no arguments is still a
    /// tool, and requiring `{}` from every caller would be a rule with no purpose.
    #[serde(default)]
    pub arguments: Value,
}

/// What a tool answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    /// The answer itself, as JSON the agent reads.
    pub value: Value,
    /// What the audit trail should remember beyond the tool's name: the command line, the remote
    /// path, the risk verdict. Not part of the agent's answer — the bridge drops it when it builds
    /// the reply — and absent when there is nothing more to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<ToolAudit>,
}

impl ToolResult {
    pub fn new(value: impl Into<Value>) -> Self {
        Self {
            value: value.into(),
            audit: None,
        }
    }

    pub fn with_audit(mut self, audit: ToolAudit) -> Self {
        self.audit = Some(audit);
        self
    }
}

/// The facts about a call that only the tool knows.
///
/// The registry fills in the name, the scope and how long the call took; a tool fills in what it
/// acted on and what the policy said about it. Keeping the two halves apart is what lets one audit
/// line per call be complete without every tool writing its own line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAudit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// What the action was about, in the tool's own terms: a command line, a remote path. Data, not
    /// a sentence — an audit line is read by whoever investigates, not translated for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<RiskAssessment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalRecord>,
}

impl ToolAudit {
    pub fn for_session(session_id: impl Into<String>) -> Self {
        Self {
            session_id: Some(session_id.into()),
            ..Self::default()
        }
    }

    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    pub fn with_risk(mut self, risk: RiskAssessment) -> Self {
        self.risk = Some(risk);
        self
    }

    pub fn with_approval(mut self, approval: ApprovalRecord) -> Self {
        self.approval = Some(approval);
        self
    }
}

/// A call that did not produce an answer.
///
/// The fields sit behind a `Box` for the reason the error envelope's do: every tool answers
/// `Result<ToolResult, ToolError>`, and a 320-byte error variant would be copied on every
/// *successful* call. This is a newtype, so the JSON is still one flat object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolError(pub(crate) Box<ToolErrorFields>);

/// The fields themselves. Private: they are filled by the named constructors in `error`, so a code
/// and the key it localizes to cannot drift apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolErrorFields {
    pub(crate) code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) message_params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) audit: Option<ToolAudit>,
}

impl ToolError {
    pub(crate) fn from_fields(fields: ToolErrorFields) -> Self {
        Self(Box::new(fields))
    }

    /// The stable code. Codes that come from the error catalog are the catalog's own
    /// (`VAULT-LOCKED`, `POLICY-DENIED`); codes that belong to this layer start with `MCP-`.
    pub fn code(&self) -> &str {
        &self.0.code
    }

    /// A plain sentence for the agent. Absent when the code and the detail are all there is to say.
    pub fn message(&self) -> Option<&str> {
        self.0.message.as_deref()
    }

    /// The catalog key, for anything the desktop draws about the failure.
    pub fn message_key(&self) -> Option<&str> {
        self.0.message_key.as_deref()
    }

    pub fn message_params(&self) -> &BTreeMap<String, String> {
        &self.0.message_params
    }

    /// The scrubbed explanation, when one exists.
    pub fn detail(&self) -> Option<&str> {
        self.0.detail.as_deref()
    }

    /// The same audit facts a successful result carries, so a refused or failed call is as
    /// explainable afterwards as one that ran.
    pub fn audit(&self) -> Option<&ToolAudit> {
        self.0.audit.as_ref()
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())?;
        if let Some(message) = self.message() {
            write!(formatter, ": {message}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ToolError {}

/// What one call came to, as the control endpoint carries it.
///
/// A tagged union rather than an HTTP status, because the distinction that matters to the caller is
/// "the tool ran and this is what it said" against "the tool did not run and this is why" — and both
/// are answers from a working desktop.
///
/// The result is boxed so the two variants are the same size: a success would otherwise carry an
/// audit note worth a couple of hundred bytes on every stack frame a call passes through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ToolOutcome {
    Succeeded { result: Box<ToolResult> },
    Failed { error: ToolError },
}

impl ToolOutcome {
    pub fn succeeded(result: ToolResult) -> Self {
        Self::Succeeded {
            result: Box::new(result),
        }
    }

    pub fn failed(error: ToolError) -> Self {
        Self::Failed { error }
    }

    pub fn is_succeeded(&self) -> bool {
        matches!(self, Self::Succeeded { .. })
    }

    pub fn error(&self) -> Option<&ToolError> {
        match self {
            Self::Failed { error } => Some(error),
            Self::Succeeded { .. } => None,
        }
    }
}

/// Reads one tool's arguments, refusing anything the tool did not declare.
///
/// The refusal is the point: every input is `deny_unknown_fields`, so an argument nobody declared —
/// `acknowledged` above all — is an invalid call rather than something quietly dropped.
pub fn parse_arguments<I: DeserializeOwned>(arguments: Value) -> Result<I, ToolError> {
    let arguments = match arguments {
        Value::Null => Value::Object(serde_json::Map::new()),
        other => other,
    };
    serde_json::from_value(arguments).map_err(|error| ToolError::invalid_arguments(crate::error::describe(&error)))
}

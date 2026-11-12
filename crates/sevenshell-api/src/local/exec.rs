//! Running a command: on one host, or on many at once.
//!
//! One tool would have done, but they answer differently and the difference matters to an agent: one
//! host is a result, and many hosts are rows. A single host that cannot be reached fails the call; a
//! batch reports that host's row and keeps the other hosts' answers, because "nine of ten answered"
//! is the answer a fleet run exists for.
//!
//! Both go through [`sevenshell_core::Core::run_across`], and both are gated the same way: the
//! command is classified before anything is resolved, a destructive verdict is one question on the
//! desktop, and only an approval produces `acknowledged`. Nothing an agent sends can produce it.

use std::collections::BTreeSet;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use sevenshell_core::{MAX_PARALLEL, OPEN_TIMEOUT};
use sevenshell_error::ShellError;
use sevenshell_types::{BatchHostOutcome, BatchHostResult, BatchOutcome, BatchRequest, PresentedHostKey};
use tokio::sync::mpsc;

use crate::context::{ApprovalRecord, ToolContext};
use crate::registry::Tool;
use crate::tool::{ToolAccess, ToolAudit, ToolDescriptor, ToolError, ToolInputSchema, ToolResult, parse_arguments};

use super::cache::saved_session;
use super::gate;
use super::session_id;

pub const EXEC_RUN: &str = "sshell_exec_run";
pub const BATCH_EXEC: &str = "sshell_batch_exec";

// -- Inputs -------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecRunInput {
    pub session_id: String,
    pub command: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchExecInput {
    pub session_ids: Vec<String>,
    pub command: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub parallel: Option<u16>,
}

// -- Descriptors --------------------------------------------------------

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

/// A whole number with both ends. The ends are the clamp the core already applies, stated where an
/// agent can read them rather than discovered by sending an absurd number.
fn integer(description: &str, minimum: u64, maximum: u64) -> Value {
    json!({ "type": "integer", "minimum": minimum, "maximum": maximum, "description": description })
}

fn string_array(description: &str) -> Value {
    json!({ "type": "array", "minItems": 1, "items": { "type": "string" }, "description": description })
}

fn session_id_property() -> Value {
    string("The id of a saved session, from sshell_sessions_list.")
}

fn timeout_ms_property() -> Value {
    integer(
        "How long one host may take, in milliseconds. Absent uses the core's default, and the value is clamped to the range shown.",
        gate::EXEC_TIMEOUT_MIN_MS,
        gate::EXEC_TIMEOUT_MAX_MS,
    )
}

pub fn exec_run_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: EXEC_RUN.to_owned(),
        title: "Run a command".to_owned(),
        description: "Runs one command line on one saved session and answers with its exit status, \
                      standard output and standard error. The command is graded before it runs: a \
                      command graded destructive stops for a person's approval on the desktop and \
                      runs only if they agree."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionId": session_id_property(),
                "command": string("The command line, run by the server's own shell."),
                "timeoutMs": timeout_ms_property()
            }),
            &["sessionId", "command"],
        ),
        access: ToolAccess::Write,
    }
}

pub fn batch_exec_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: BATCH_EXEC.to_owned(),
        title: "Run a command on many sessions".to_owned(),
        description: "Runs one command line on several saved sessions and answers with one row per \
                      session, in the order the ids were given. One session's failure stays that \
                      session's row. The command is graded once for the whole run, and a command \
                      graded destructive stops for a person's approval before any session is \
                      touched."
            .to_owned(),
        input_schema: ToolInputSchema::object(
            json!({
                "sessionIds": string_array("The saved sessions to run on, in the order the rows should come back."),
                "command": string("The command line, run by each server's own shell."),
                "timeoutMs": timeout_ms_property(),
                "parallel": integer("How many sessions to work on at once. Clamped by the core to its own ceiling.", 1, u64::from(MAX_PARALLEL))
            }),
            &["sessionIds", "command"],
        ),
        access: ToolAccess::Write,
    }
}

// -- The tools ----------------------------------------------------------

pub struct ExecRunTool;

#[async_trait]
impl Tool for ExecRunTool {
    fn descriptor(&self) -> ToolDescriptor {
        exec_run_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: ExecRunInput = parse_arguments(arguments)?;
        let session_id = session_id(&input.session_id)?;
        let command = command_of(&input.command)?;
        // Before the command is graded and before anybody is asked about it: whether this tool may
        // run at all is the session's own answer, and a refusal here must not cost a prompt.
        context.guard_session(session_id, ToolAccess::Write)?;
        let timeout_ms = gate::timeout_ms(input.timeout_ms);

        let risk = crate::classify_command(&command);
        let subject = gate::subject(&command);
        let name = saved_session(context.core(), session_id).map(|session| session.name);

        // The matrix may raise a Safe command to Ask, so the gate is consulted for every risk, not
        // only for the classifier's destructive levels. With the default rule this is a no-op for
        // Safe; a tighter layer turns it into a question.
        let mut approval =
            gate::approve_risk(context, EXEC_RUN, &risk, Some((session_id, name)), None, &subject).await?;
        let acknowledged = approval.is_some();

        let request = BatchRequest {
            session_ids: vec![session_id.to_owned()],
            command: command.clone(),
            timeout_ms,
            // One host: a second worker would have nothing to do, and zero would read as "the
            // caller did not say", which is a different answer for the core.
            parallel: 1,
            acknowledged,
        };
        let finished = run(context, EXEC_RUN, request, &mut approval).await?;

        let audit = match approval {
            Some(record) => ToolAudit::for_session(session_id)
                .with_subject(subject.clone())
                .with_risk(finished.risk.clone())
                .with_approval(record),
            None => ToolAudit::for_session(session_id)
                .with_subject(subject.clone())
                .with_risk(finished.risk.clone()),
        };

        let mut rows = finished.results.into_iter();
        let Some(row) = rows.next() else {
            return Err(ToolError::new("APP-INTERNAL")
                .with_message("the run answered with no host")
                .with_audit(audit));
        };

        match row.outcome {
            BatchHostOutcome::Ran {
                exit_code,
                stdout,
                stderr,
            } => {
                let (stdout, stdout_cut) = gate::clip(&stdout, gate::STREAM_LIMIT);
                let (stderr, stderr_cut) = gate::clip(&stderr, gate::STREAM_LIMIT);
                Ok(ToolResult::new(json!({
                    "sessionId": row.session_id,
                    "sessionName": row.session_name,
                    "command": command,
                    "exitCode": exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                    "truncated": { "stdout": stdout_cut, "stderr": stderr_cut },
                    "durationMs": row.duration_ms,
                    "risk": finished.risk,
                }))
                .with_audit(audit))
            }
            // A host that never ran the command is this call's failure, because the call named one
            // host. The code is the envelope's own, so an agent can tell a refused password from a
            // command that ran and said no.
            BatchHostOutcome::Failed { error } => Err(ToolError::new(error.code)
                .with_message(
                    error
                        .detail
                        .unwrap_or_else(|| "the host did not run the command".to_owned()),
                )
                .with_audit(audit)),
        }
    }
}

pub struct BatchExecTool;

#[async_trait]
impl Tool for BatchExecTool {
    fn descriptor(&self) -> ToolDescriptor {
        batch_exec_descriptor()
    }

    async fn call(&self, context: &ToolContext, arguments: Value) -> Result<ToolResult, ToolError> {
        let input: BatchExecInput = parse_arguments(arguments)?;
        let session_ids = batch_session_ids(&input.session_ids)?;
        let command = command_of(&input.command)?;
        // Every named session is checked before any of them is touched: a run that reached nine of
        // ten hosts because the tenth is hidden would be a partial write nobody asked for, and the
        // refusal names no host, so it says nothing about a session the caller may not see.
        for session_id in &session_ids {
            context.guard_session(session_id, ToolAccess::Write)?;
        }
        let timeout_ms = gate::timeout_ms(input.timeout_ms);
        let parallel = input.parallel.map_or(0, |parallel| parallel.clamp(1, MAX_PARALLEL));

        let risk = crate::classify_command(&command);
        let subject = gate::subject(&command);

        // One question for the whole run: the matrix can raise even a safe command to Ask, and the
        // question is about the command, not one host. The core re-decides per host as the journal's
        // stricter layer; this asks before any session is touched.
        let mut approval = gate::approve_risk(context, BATCH_EXEC, &risk, None, None, &subject).await?;
        let acknowledged = approval.is_some();

        let request = BatchRequest {
            session_ids,
            command: command.clone(),
            timeout_ms,
            parallel,
            acknowledged,
        };
        let finished = run(context, BATCH_EXEC, request, &mut approval).await?;

        let mut audit = ToolAudit::default()
            .with_subject(subject)
            .with_risk(finished.risk.clone());
        if let Some(record) = approval {
            audit = audit.with_approval(record);
        }

        let rows: Vec<Value> = finished.results.into_iter().map(host_row).collect();

        Ok(ToolResult::new(json!({
            "command": command,
            "risk": finished.risk,
            "results": rows,
            "durationMs": finished.duration_ms,
        }))
        .with_audit(audit))
    }
}

// -- The run ------------------------------------------------------------

/// What a finished run answered, once the two decisions that are not answers are out of the way.
struct Finished {
    risk: sevenshell_types::RiskAssessment,
    results: Vec<BatchHostResult>,
    duration_ms: u64,
}

/// Runs a request, answering the one thing a run cannot answer by itself: an unknown host key.
///
/// Two kinds of decision come back from the core rather than from a person. A key that has never
/// been seen is a question, and trusting it is answered by asking again — once. A key that has
/// *changed* is a stop: no question is put to anybody, because reaching a server whose identity
/// changed is the exact attack host keys exist to catch.
async fn run(
    context: &ToolContext,
    tool: &str,
    request: BatchRequest,
    approval: &mut Option<ApprovalRecord>,
) -> Result<Finished, ToolError> {
    // The open budget plus the host budget is the worst case the core itself is allowed to take.
    let budget = Duration::from_millis(request.timeout_ms).saturating_add(OPEN_TIMEOUT);
    let mut trusted = false;

    loop {
        let (progress, unread) = mpsc::channel(16);
        // Nobody watches this run's progress: the answer carries every row, and the calls are short
        // enough that a live view would be a stream of one event. The receiver is dropped so a full
        // buffer can never hold the run inside a send.
        drop(unread);

        let outcome = gate::bounded(context, budget, async {
            context
                .cache()
                .core()
                .run_across(request.clone(), progress)
                .await
                .map_err(ToolError::from)
        })
        .await?;

        match outcome {
            BatchOutcome::Finished {
                risk,
                results,
                duration_ms,
                ..
            } => {
                return Ok(Finished {
                    risk,
                    results,
                    duration_ms,
                });
            }
            BatchOutcome::HostKeyUnknown { host, key } => {
                if trusted {
                    // One retry: a second unknown key is a second host in a route, and the person
                    // who answered the first question has not seen the second fingerprint. Saying
                    // which host it is beats asking a question nobody was shown.
                    return Err(unknown(&host.session_id, &key));
                }

                let name = saved_session(context.core(), &host.session_id).map(|session| session.name);
                let record = gate::approve_host_key(context, tool, &key, Some((&host.session_id, name))).await?;
                // A trust that could not be written must stop the run: a retry without it would
                // present the same unknown key and be refused for a reason nobody would see.
                context.core().trust_host_key(&key)?;
                *approval = Some(record);
                trusted = true;
            }
            BatchOutcome::HostKeyMismatch { host, key, trusted } => {
                return Err(mismatch(&host.session_id, &key, &trusted));
            }
            // Unreachable while this tool and the core grade a command with the same classifier AND
            // this tool always sends the acknowledgement it obtained. Answered as a failure rather
            // than a panic: a disagreement between two versions is not the agent's problem.
            BatchOutcome::ApprovalRequired { risk } => {
                return Err(ToolError::new("POLICY-APPROVAL-REQUIRED")
                    .with_message(format!(
                        "the command was graded {} by the core after this tool had already asked",
                        risk.level
                    ))
                    .with_audit(ToolAudit::default().with_risk(risk)));
            }
        }
    }
}

/// A key that has never been seen and was not trusted on the second attempt either.
fn unknown(session_id: &str, key: &PresentedHostKey) -> ToolError {
    ToolError::from(ShellError::ssh_host_key_unknown(&key.host, key.port, &key.key_type))
        .with_message(format!(
            "the host {}:{} was not trusted, and its key is still unknown",
            key.host, key.port
        ))
        .with_audit(ToolAudit::for_session(session_id))
}

/// A key that changed. Nothing was sent, and nothing will be.
fn mismatch(session_id: &str, presented: &PresentedHostKey, trusted: &sevenshell_types::KnownHostEntry) -> ToolError {
    ToolError::from(ShellError::ssh_host_key_mismatch(
        &presented.host,
        presented.port,
        &presented.key_type,
    ))
    .with_message(format!(
        "the host key of {}:{} changed: {} was presented where {} is trusted. Nothing was sent to the server.",
        presented.host, presented.port, presented.fingerprint, trusted.fingerprint
    ))
    .with_audit(ToolAudit::for_session(session_id))
}

/// One host's row, as an agent reads it.
fn host_row(result: BatchHostResult) -> Value {
    match result.outcome {
        BatchHostOutcome::Ran {
            exit_code,
            stdout,
            stderr,
        } => {
            let (stdout, stdout_cut) = gate::clip(&stdout, gate::STREAM_LIMIT);
            let (stderr, stderr_cut) = gate::clip(&stderr, gate::STREAM_LIMIT);
            json!({
                "sessionId": result.session_id,
                "sessionName": result.session_name,
                "ok": true,
                "exitCode": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "truncated": { "stdout": stdout_cut, "stderr": stderr_cut },
                "durationMs": result.duration_ms,
            })
        }
        BatchHostOutcome::Failed { error } => json!({
            "sessionId": result.session_id,
            "sessionName": result.session_name,
            "ok": false,
            // The envelope's own code, key and detail: the words a person would see about this host.
            "error": {
                "code": error.code,
                "messageKey": error.message_key,
                "detail": error.detail,
            },
            "durationMs": result.duration_ms,
        }),
    }
}

// -- Argument checks ----------------------------------------------------

/// The command as sent. Blank is not a command: it would run an empty shell line on every host and
/// answer zero, which looks like success.
fn command_of(command: &str) -> Result<String, ToolError> {
    if command.trim().is_empty() {
        return Err(ToolError::invalid_arguments("command is empty"));
    }
    Ok(command.to_owned())
}

/// The sessions a batch names: at most the contract's ceiling, no blanks, and no repeats.
///
/// A repeated id would run the command twice on one host while the caller reads one row per
/// position, which is the kind of double-run nobody notices until production did it twice.
fn batch_session_ids(ids: &[String]) -> Result<Vec<String>, ToolError> {
    if ids.is_empty() {
        return Err(ToolError::invalid_arguments("sessionIds is empty"));
    }
    if ids.len() > gate::BATCH_LIMIT {
        return Err(ToolError::invalid_arguments(format!(
            "sessionIds names {} sessions; at most {} may be run at once",
            ids.len(),
            gate::BATCH_LIMIT
        )));
    }

    let mut seen = BTreeSet::new();
    let mut checked = Vec::with_capacity(ids.len());
    for id in ids {
        let id = session_id(id)?;
        if !seen.insert(id) {
            return Err(ToolError::invalid_arguments(format!(
                "sessionIds names `{id}` more than once"
            )));
        }
        checked.push(id.to_owned());
    }
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_refuses_the_ceiling_blanks_and_repeats() {
        let too_many: Vec<String> = (0..=gate::BATCH_LIMIT).map(|index| format!("s{index}")).collect();
        assert!(batch_session_ids(&too_many).is_err());

        assert!(batch_session_ids(&["".to_owned()]).is_err());
        assert!(batch_session_ids(&["a".to_owned(), " a ".to_owned()]).is_err());
        assert_eq!(
            batch_session_ids(&["a".to_owned(), "b".to_owned()]).expect("two distinct sessions"),
            vec!["a".to_owned(), "b".to_owned()]
        );
    }

    #[test]
    fn a_blank_command_is_not_a_command() {
        assert!(command_of("   ").is_err());
        assert_eq!(command_of(" echo hi ").expect("a command"), " echo hi ");
    }

    #[test]
    fn a_host_row_cuts_each_stream_at_the_limit_and_says_which_one_it_cut() {
        let long = "x".repeat(gate::STREAM_LIMIT + 10);
        let row = host_row(BatchHostResult {
            session_id: "lab".to_owned(),
            session_name: "The lab".to_owned(),
            outcome: BatchHostOutcome::Ran {
                exit_code: Some(0),
                stdout: long,
                stderr: "short".to_owned(),
            },
            duration_ms: 7,
        });

        assert_eq!(row["truncated"]["stdout"], json!(true));
        assert_eq!(row["truncated"]["stderr"], json!(false));
        assert_eq!(
            row["stdout"].as_str().map(str::len),
            Some(gate::STREAM_LIMIT),
            "a stream is returned up to the limit, not whole"
        );
        assert_eq!(row["stderr"], json!("short"));
        assert_eq!(row["ok"], json!(true));
    }

    #[test]
    fn a_failed_host_row_carries_the_envelope_parts_a_person_would_read() {
        let row = host_row(BatchHostResult {
            session_id: "lab".to_owned(),
            session_name: "The lab".to_owned(),
            outcome: BatchHostOutcome::Failed {
                error: sevenshell_types::HostFailure {
                    code: "SSH-AUTH-FAILED".to_owned(),
                    message_key: "errors.ssh.authFailed".to_owned(),
                    message_params: std::collections::BTreeMap::new(),
                    detail: Some("no password is stored for this session".to_owned()),
                },
            },
            duration_ms: 0,
        });

        assert_eq!(row["ok"], json!(false));
        assert_eq!(row["error"]["code"], json!("SSH-AUTH-FAILED"));
        assert_eq!(row["error"]["messageKey"], json!("errors.ssh.authFailed"));
        assert_eq!(row["error"]["detail"], json!("no password is stored for this session"));
    }
}

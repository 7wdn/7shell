//! Running one command on many hosts.
//!
//! Three rules shape this module, and each is about what a batch must not do:
//!
//! 1. **The policy is decided before a host is resolved.** Every host contributes the layers its
//!    saved session names, the strictest answer wins, and a deny anywhere stops the run while it is
//!    still only a set of decisions. A batch that ran on the hosts that were allowed would be a
//!    partial write nobody asked for.
//! 2. **Connect to every host before running anything.** A host that needs a decision stops the
//!    run while it is still only a set of connections. Otherwise a batch would run the command on
//!    the first nine hosts, discover the tenth needs an answer, and run the command again on the
//!    nine after the answer — twice on nine hosts.
//! 3. **One host's trouble stays that host's.** Unreachable, no stored password, gone from the
//!    vault: each becomes that host's own answer, and the other hosts still run. The one thing
//!    that stops the batch is a decision a person has to make.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sevenshell_error::ShellError;
use sevenshell_policy::{PolicyTarget, classify};
use sevenshell_ssh::SshSession;
use sevenshell_types::{
    BatchHostOutcome, BatchHostRef, BatchHostResult, BatchOutcome, BatchRequest, HostFailure, PolicyAction,
    RiskAssessment, Session,
};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

use crate::Core;
use crate::sessions::{ConnectionOutcome, HostKeyDecision};

/// The operation an audit line names for a run across hosts.
///
/// A dotted operation rather than the tool's name: the core serves the batch workspace as well as
/// an agent, and the audit line has to say what was done, not which caller did it.
const BATCH_ACTION: &str = "batch.exec";

/// How many hosts are worked on at once when the request does not say.
pub const DEFAULT_PARALLEL: u16 = 4;
/// The ceiling, because a hundred simultaneous authentications is not faster, it is a load test.
pub const MAX_PARALLEL: u16 = 16;
/// One host may not be given less than this, whatever the request says.
pub const MIN_HOST_TIMEOUT: Duration = Duration::from_secs(1);
/// Nor more: a command that runs longer than this on many hosts at once is a job for the server's
/// own scheduler, not a request holding connections open.
pub const MAX_HOST_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Grades a command without running it.
///
/// For an interface that wants to show the verdict *before* anybody presses anything: the same
/// classifier and the same rules the run itself consults, so what is shown and what is enforced
/// cannot disagree.
pub fn classify_command(command: &str) -> RiskAssessment {
    classify(command)
}

/// What a batch reports while it is running.
///
/// Best effort on purpose: a batch whose reader went away still runs, and its results still come
/// back to whoever asked for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchEvent {
    /// This host's turn has come: it is connecting, or running the command.
    HostStarted { session_id: String },
    /// This host has an answer, and it will not change.
    HostFinished { result: Box<BatchHostResult> },
}

/// One host in the request, resolved as far as the vault allows.
struct Host {
    session_id: String,
    /// The saved session, or why it could not be read.
    session: Result<Session, ShellError>,
}

impl Host {
    fn name(&self) -> String {
        match &self.session {
            Ok(session) => session.name.clone(),
            // A session that is not there has no name: the id is what the caller asked about, and
            // it is all that is left to label the row with.
            Err(_) => self.session_id.clone(),
        }
    }
}

/// What phase one produced for one host.
enum HostConnection {
    Ready(Arc<SshSession>),
    /// A person has to answer before this can go on. Nothing has run.
    Decision(Box<HostKeyDecision>),
    /// This host has nothing to run with: the session could not be read, or the connection could
    /// not be made. The other hosts do not care.
    Failed(ShellError),
}

impl Core {
    /// Runs one command on every host in the request, and answers with one row per host.
    ///
    /// Takes `Arc<Self>` because the hosts are worked on concurrently, which means tasks that
    /// outlive this call's borrow of the core.
    pub async fn run_across(
        self: &Arc<Self>,
        request: BatchRequest,
        progress: mpsc::Sender<BatchEvent>,
    ) -> Result<BatchOutcome, ShellError> {
        let risk = classify(&request.command);

        let hosts: Vec<Host> = request
            .session_ids
            .iter()
            .map(|session_id| Host {
                session_id: session_id.clone(),
                session: self.session(session_id),
            })
            .collect();

        // The policy gate, one decision per host and before a socket: what the matrix says about a
        // host is decided from the saved session, so the answer does not depend on whether the host
        // is reachable. A deny anywhere stops the whole run — a batch that ran on the hosts that
        // were allowed would be a partial write nobody asked for.
        let subject = audit_subject(&risk);
        let mut strictest = PolicyAction::Allow;
        for host in &hosts {
            let decision = self.policy_decision(
                BATCH_ACTION,
                &risk,
                host_target(host),
                Some(&host.session_id),
                subject.as_deref(),
            );
            strictest = strictest.max(decision.action);
        }

        if strictest == PolicyAction::Deny {
            return Err(ShellError::policy_denied(BATCH_ACTION));
        }
        // An escalation the matrix made — a safe or caution command an installation asks about —
        // is a question like any other. A destructive verdict is Ask on its own, which keeps the
        // old stop for a run that names no host and therefore journaled nothing.
        if (strictest == PolicyAction::Ask || risk.needs_approval()) && !request.acknowledged {
            return Ok(BatchOutcome::ApprovalRequired { risk });
        }

        let started = Instant::now();
        let permits = Arc::new(Semaphore::new(clamp_parallel(request.parallel, hosts.len())));

        // Phase one: a connection for every host, before a single command runs.
        let connected = self.connect_all(&hosts, &permits).await;

        let mut connections: Vec<Option<Arc<SshSession>>> = Vec::with_capacity(hosts.len());
        let mut results: Vec<Option<BatchHostResult>> = Vec::with_capacity(hosts.len());

        for (host, outcome) in hosts.iter().zip(connected) {
            match outcome {
                HostConnection::Ready(connection) => {
                    connections.push(Some(connection));
                    results.push(None);
                }
                HostConnection::Decision(decision) => {
                    // Nothing has run anywhere, and nothing will until this is answered:
                    // decisions are answered in request order, so which one comes back does not
                    // depend on which host happened to answer first. The connections opened so
                    // far are dropped here, which closes them.
                    return Ok(batch_decision(host, *decision));
                }
                HostConnection::Failed(error) => {
                    connections.push(None);
                    let result = failed_result(host, &error);
                    // Best effort: a reader that went away does not fail the run. The `await`
                    // is what actually sends it — dropping the future would drop the event.
                    let _ = progress
                        .send(BatchEvent::HostFinished {
                            result: Box::new(result.clone()),
                        })
                        .await;
                    results.push(Some(result));
                }
            }
        }

        // Phase two: the command, on the hosts that got this far. The connections live in
        // `connections` for the length of the run, and dropping them at the end is what closes
        // them — including for the hosts that never reached this point.
        let timeout = clamp_timeout(request.timeout_ms);
        let mut tasks = JoinSet::new();

        for (index, connection) in connections.iter().enumerate() {
            let Some(connection) = connection.clone() else {
                continue;
            };

            let permits = Arc::clone(&permits);
            let progress = progress.clone();
            let command = request.command.clone();
            let session_id = hosts[index].session_id.clone();
            let session_name = hosts[index].name();

            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let _ = progress
                    .send(BatchEvent::HostStarted {
                        session_id: session_id.clone(),
                    })
                    .await;

                let begun = Instant::now();
                let outcome = match connection.exec(&command, timeout).await {
                    Ok(exec) => BatchHostOutcome::Ran {
                        exit_code: exec.exit_code,
                        stdout: exec.stdout,
                        stderr: exec.stderr,
                    },
                    Err(error) => BatchHostOutcome::Failed {
                        error: host_failure(&error),
                    },
                };

                let result = BatchHostResult {
                    session_id,
                    session_name,
                    outcome,
                    duration_ms: millis(begun.elapsed()),
                };

                let _ = progress
                    .send(BatchEvent::HostFinished {
                        result: Box::new(result.clone()),
                    })
                    .await;
                (index, result)
            });
        }

        while let Some(joined) = tasks.join_next().await {
            let (index, result) = joined.expect("a batch host task must not panic");
            results[index] = Some(result);
        }

        Ok(BatchOutcome::Finished {
            command: request.command,
            risk,
            results: results
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .expect("every host has an answer: it ran, or it failed"),
            duration_ms: millis(started.elapsed()),
        })
    }

    /// A connection for every host, concurrently, answered in request order.
    ///
    /// The connections are opened in parallel and the *answers* are put back in the request's
    /// order, so which host needs a decision first is a property of the request and not of the
    /// network.
    async fn connect_all(self: &Arc<Self>, hosts: &[Host], permits: &Arc<Semaphore>) -> Vec<HostConnection> {
        let mut tasks: JoinSet<(usize, HostConnection)> = JoinSet::new();

        for (index, host) in hosts.iter().enumerate() {
            let Ok(session) = &host.session else {
                continue;
            };

            let core = Arc::clone(self);
            let permits = Arc::clone(permits);
            let session = session.clone();
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await;
                (index, connect_one(&core, session).await)
            });
        }

        let mut collected: Vec<Option<HostConnection>> = (0..hosts.len()).map(|_| None).collect();
        while let Some(joined) = tasks.join_next().await {
            let (index, outcome) = joined.expect("a batch connect task must not panic");
            collected[index] = Some(outcome);
        }

        hosts
            .iter()
            .zip(collected)
            .map(|(host, outcome)| match outcome {
                Some(outcome) => outcome,
                // No task ran for this host, because its session could not be read. That is this
                // host's answer, and it was already known before any connection was attempted.
                None => HostConnection::Failed(
                    host.session
                        .as_ref()
                        .err()
                        .cloned()
                        .expect("a host has no task only when its session failed"),
                ),
            })
            .collect()
    }
}

/// Connects to one host without a terminal, or reports what stands in the way.
async fn connect_one(core: &Core, session: Session) -> HostConnection {
    match core.open_carrier(&session).await {
        Ok(ConnectionOutcome::Ready(connection)) => HostConnection::Ready(connection),
        Ok(ConnectionOutcome::Decision(decision)) => HostConnection::Decision(decision),
        Err(error) => HostConnection::Failed(error),
    }
}

/// The layers one host's decision looks at: its group, its host and port, and no directory.
///
/// A session that could not be read contributes no layer at all — there is no host to name — so
/// only the global rule decides about it. That host's own error is still its row; this is only
/// about whether the run as a whole may go ahead.
fn host_target(host: &Host) -> PolicyTarget<'_> {
    match &host.session {
        Ok(session) => PolicyTarget {
            group_id: session.group_id.as_deref(),
            host: Some(session.host.as_str()),
            port: Some(session.port),
            directory: None,
        },
        Err(_) => PolicyTarget::default(),
    }
}

/// What the audit line may say about a batch command.
///
/// The rule ids, not the command line: a batch runs below the tool layer, where the command has not
/// been masked yet, and a command line is the input most likely to carry a password. An id explains
/// the decision without echoing what was typed. Nothing is invented for a command no rule fired on,
/// which is why a safe command has no subject at all.
fn audit_subject(risk: &RiskAssessment) -> Option<String> {
    if risk.reasons.is_empty() {
        return None;
    }
    Some(
        risk.reasons
            .iter()
            .map(|reason| reason.rule_id.as_str())
            .collect::<Vec<_>>()
            .join(","),
    )
}

fn batch_decision(host: &Host, decision: HostKeyDecision) -> BatchOutcome {
    let host = BatchHostRef {
        session_id: host.session_id.clone(),
        session_name: host.name(),
    };

    match decision {
        HostKeyDecision::Unknown(key) => BatchOutcome::HostKeyUnknown { host, key },
        HostKeyDecision::Mismatch { key, trusted } => BatchOutcome::HostKeyMismatch { host, key, trusted },
    }
}

/// One host's failure, as the interface reads it.
///
/// A copy rather than the error itself: the wire types cannot name the error type — the two crates
/// are peers, and nothing may depend on a peer — so the parts a message needs are carried across
/// deliberately. Nothing is reinterpreted on the way; the code, the key, its parameters and the
/// scrubbed detail are the envelope's own values.
fn host_failure(error: &ShellError) -> HostFailure {
    HostFailure {
        code: error.code().as_str().to_owned(),
        message_key: error.message_key().to_owned(),
        message_params: error.message_params().clone(),
        detail: error.detail().map(str::to_owned),
    }
}

fn failed_result(host: &Host, error: &ShellError) -> BatchHostResult {
    BatchHostResult {
        session_id: host.session_id.clone(),
        session_name: host.name(),
        outcome: BatchHostOutcome::Failed {
            error: host_failure(error),
        },
        duration_ms: 0,
    }
}

/// At least one host at a time, never more than the ceiling, and never more than there is work.
fn clamp_parallel(asked: u16, hosts: usize) -> usize {
    let asked = if asked == 0 {
        DEFAULT_PARALLEL
    } else {
        asked.min(MAX_PARALLEL)
    };
    usize::from(asked).max(1).min(hosts.max(1))
}

fn clamp_timeout(asked_ms: u64) -> Duration {
    Duration::from_millis(asked_ms).clamp(MIN_HOST_TIMEOUT, MAX_HOST_TIMEOUT)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sevenshell_policy::{PolicyAuditEvent, PolicyJournal};
    use sevenshell_types::{AuthMethod, PolicyMatrix, PolicyRule, Protocol, RiskLevel, SessionAuth, TerminalSettings};
    use sevenshell_vault::Vault;
    use tempfile::{TempDir, tempdir};

    use super::*;

    #[derive(Default)]
    struct RecordingJournal {
        events: Mutex<Vec<PolicyAuditEvent>>,
    }

    impl RecordingJournal {
        fn events(&self) -> Vec<PolicyAuditEvent> {
            self.events.lock().expect("not poisoned").clone()
        }
    }

    impl PolicyJournal for RecordingJournal {
        fn record(&self, event: PolicyAuditEvent) {
            self.events.lock().expect("not poisoned").push(event);
        }
    }

    fn rule(action: PolicyAction) -> PolicyRule {
        PolicyRule {
            safe: action,
            caution: action,
            destructive: action,
            catastrophic: action,
        }
    }

    /// Returns the temp directory as well as the vault: dropping it would take the vault file
    /// with it.
    fn vault_with_a_session() -> (TempDir, Arc<Vault>) {
        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("correct horse battery staple").expect("create");
        // Saved without a secret on purpose: the credential failure is known before any socket is
        // opened, which is what keeps these tests offline and deterministic.
        vault.save_session(sample("lab", "lab"), None).expect("save");
        (directory, vault)
    }

    fn sample(id: &str, name: &str) -> Session {
        Session {
            id: id.to_owned(),
            name: name.to_owned(),
            group_id: None,
            protocol: Protocol::Ssh,
            host: "127.0.0.1".to_owned(),
            port: 2222,
            user: "someone".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            desktop: sevenshell_types::DesktopSettings::default(),
            telnet: sevenshell_types::TelnetSettings::default(),
            serial: sevenshell_types::SerialSettings::default(),
            local: sevenshell_types::LocalSettings::default(),
            wsl: sevenshell_types::WslSettings::default(),
            container: sevenshell_types::ContainerSettings::default(),
            kubernetes: sevenshell_types::KubernetesSettings::default(),
            webdav: sevenshell_types::WebDavSettings::default(),
            ftp: sevenshell_types::FtpSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 0,
        }
    }

    fn request(session_ids: &[&str], command: &str) -> BatchRequest {
        BatchRequest {
            session_ids: session_ids.iter().map(|id| (*id).to_owned()).collect(),
            command: command.to_owned(),
            timeout_ms: 5_000,
            parallel: 2,
            acknowledged: false,
        }
    }

    fn failure(result: &BatchHostResult) -> &HostFailure {
        match &result.outcome {
            BatchHostOutcome::Failed { error } => error,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_dangerous_command_comes_back_before_any_host_is_touched() {
        let (_directory, vault) = vault_with_a_session();
        let (core, _events) = Core::new(vault);
        let (progress, _receiver) = mpsc::channel(16);

        // The session does not even exist. Getting the verdict rather than "no such session" is
        // the proof that the gate is before the vault and not after it.
        let outcome = core
            .run_across(request(&["never-existed"], "rm -rf /srv/app"), progress)
            .await
            .expect("waiting for approval is not an error");

        match outcome {
            BatchOutcome::ApprovalRequired { risk } => {
                assert_eq!(risk.level, RiskLevel::Destructive);
                assert!(risk.needs_approval());
                assert!(
                    risk.reasons
                        .iter()
                        .any(|reason| reason.rule_id == "recursive-force-delete"),
                    "the verdict has to name the rule that produced it: {:?}",
                    risk.reasons
                );
            }
            other => panic!("expected a verdict to be shown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_host_rule_that_denies_stops_the_batch_before_any_connection_and_is_journaled() {
        let (_directory, vault) = vault_with_a_session();
        let (core, _events) = Core::new(vault);
        let journal = Arc::new(RecordingJournal::default());
        core.set_policy(
            PolicyMatrix {
                hosts: [("127.0.0.1".to_owned(), rule(PolicyAction::Deny))].into(),
                ..PolicyMatrix::default()
            },
            Some(Arc::clone(&journal) as Arc<dyn PolicyJournal>),
        );
        let (progress, _receiver) = mpsc::channel(16);

        // The fixture's session has no stored password, so a run that got as far as connecting
        // would answer with a host row instead of an error. The refusal proves the gate came first.
        let error = core
            .run_across(request(&["lab"], "uptime"), progress)
            .await
            .expect_err("a denied host stops the whole run");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::PolicyDenied);
        let events = journal.events();
        assert_eq!(events.len(), 1, "one decision, one line — and one host");
        let event = &events[0];
        assert_eq!(event.action, "batch.exec");
        assert_eq!(event.decision, PolicyAction::Deny);
        assert_eq!(event.source, "host:127.0.0.1");
        assert_eq!(event.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(event.session_id.as_deref(), Some("lab"));
    }

    #[tokio::test]
    async fn a_matrix_can_raise_a_safe_command_to_a_question() {
        let (_directory, vault) = vault_with_a_session();
        let (core, _events) = Core::new(vault);
        let journal = Arc::new(RecordingJournal::default());
        core.set_policy(
            PolicyMatrix {
                global: rule(PolicyAction::Ask),
                ..PolicyMatrix::default()
            },
            Some(Arc::clone(&journal) as Arc<dyn PolicyJournal>),
        );
        let (progress, _receiver) = mpsc::channel(16);

        let outcome = core
            .run_across(request(&["lab"], "uptime"), progress)
            .await
            .expect("a question is not an error");

        match outcome {
            BatchOutcome::ApprovalRequired { risk } => {
                assert_eq!(risk.level, RiskLevel::Safe, "the classifier still says harmless");
            }
            other => panic!("expected the matrix's question, got {other:?}"),
        }
        let events = journal.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].decision,
            PolicyAction::Ask,
            "the matrix, not the classifier, asked"
        );
    }

    #[tokio::test]
    async fn an_acknowledged_run_goes_ahead_after_a_matrix_ask() {
        let (_directory, vault) = vault_with_a_session();
        let (core, _events) = Core::new(vault);
        core.set_policy(
            PolicyMatrix {
                global: rule(PolicyAction::Ask),
                ..PolicyMatrix::default()
            },
            None,
        );
        let (progress, _receiver) = mpsc::channel(16);

        let acknowledged = BatchRequest {
            acknowledged: true,
            ..request(&["lab"], "uptime")
        };
        let outcome = core
            .run_across(acknowledged, progress)
            .await
            .expect("an answered question lets the run go");

        // The fixture has no stored password, so the run got past the gate and reached the host's
        // own credential check: the row says so, which is what "proceeded" looks like offline.
        let BatchOutcome::Finished { results, .. } = outcome else {
            panic!("expected the run to proceed, got {outcome:?}");
        };
        assert_eq!(failure(&results[0]).code, "SSH-AUTH-FAILED");
    }

    #[tokio::test]
    async fn a_batch_line_never_carries_the_command_it_ran() {
        let (_directory, vault) = vault_with_a_session();
        let (core, _events) = Core::new(vault);
        let journal = Arc::new(RecordingJournal::default());
        core.set_policy(
            PolicyMatrix::default(),
            Some(Arc::clone(&journal) as Arc<dyn PolicyJournal>),
        );
        let (progress, _receiver) = mpsc::channel(16);

        let command = "mysql -phunter2 -e 'select 1'";
        let _ = core.run_across(request(&["lab"], command), progress).await;

        let events = journal.events();
        assert!(!events.is_empty(), "every host's verdict is journaled");
        for event in events {
            let line = serde_json::to_string(&event).expect("an audit event serializes");
            assert!(!line.contains("hunter2"), "a command line reached the trail: {line}");
            assert!(!line.contains("mysql"), "the command itself reached the trail: {line}");
        }
    }

    #[tokio::test]
    async fn nothing_selected_is_a_finished_run_with_no_hosts() {
        let (_directory, vault) = vault_with_a_session();
        let (core, _events) = Core::new(vault);
        let (progress, _receiver) = mpsc::channel(16);

        let outcome = core
            .run_across(request(&[], "uptime"), progress)
            .await
            .expect("an empty selection is not an error");

        match outcome {
            BatchOutcome::Finished { results, risk, .. } => {
                assert!(results.is_empty());
                assert_eq!(risk.level, RiskLevel::Safe);
            }
            other => panic!("expected a finished run, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn one_hosts_trouble_is_that_hosts_own_row() {
        let (_directory, vault) = vault_with_a_session();
        let (core, _events) = Core::new(vault);
        let (progress, mut events) = mpsc::channel(16);

        let outcome = core
            .run_across(request(&["lab", "never-existed"], "uptime"), progress)
            .await
            .expect("a host's failure is not the run's failure");

        let BatchOutcome::Finished { results, .. } = outcome else {
            panic!("expected a finished run, got {outcome:?}");
        };

        assert_eq!(results.len(), 2, "one row per host, however the hosts turned out");
        assert_eq!(results[0].session_id, "lab");
        assert_eq!(results[1].session_id, "never-existed");

        // The first host has no password stored, which is known before any socket is opened.
        assert_eq!(failure(&results[0]).code, "SSH-AUTH-FAILED");
        assert_eq!(
            failure(&results[0]).detail.as_deref(),
            Some("no password is stored for this session")
        );

        // The second is gone from the vault, and its row says which id was asked about.
        assert_eq!(failure(&results[1]).code, "SESSION-NOT-FOUND");
        assert_eq!(results[1].session_name, "never-existed");

        // A reader that only watches the progress sees every host's answer too.
        let mut finished = 0;
        while let Ok(event) = events.try_recv() {
            if matches!(event, BatchEvent::HostFinished { .. }) {
                finished += 1;
            }
        }
        assert_eq!(finished, 2);
    }

    #[test]
    fn the_parallel_ceiling_and_the_timeout_bounds_hold() {
        assert_eq!(clamp_parallel(0, 10), usize::from(DEFAULT_PARALLEL));
        assert_eq!(
            clamp_parallel(99, 100),
            usize::from(MAX_PARALLEL),
            "the ceiling holds however many hosts there are"
        );
        assert_eq!(clamp_parallel(99, 10), 10, "and the host count bounds it as well");
        assert_eq!(clamp_parallel(4, 2), 2, "no more workers than there are hosts");
        assert_eq!(clamp_parallel(0, 0), 1, "a run with no hosts still needs a worker");

        assert_eq!(clamp_timeout(0), MIN_HOST_TIMEOUT);
        assert_eq!(clamp_timeout(u64::MAX), MAX_HOST_TIMEOUT);
        assert_eq!(clamp_timeout(5_000), Duration::from_secs(5));
    }

    /// The envelope's own values, carried across unchanged: a message that reads "timed out after
    /// 2 seconds" needs the parameter to survive the trip.
    #[test]
    fn a_host_failure_keeps_the_code_the_key_the_parameters_and_the_detail() {
        let error = ShellError::ssh_timeout("10.0.0.7", 2222, 2).with_detail("the command was still running");

        let failure = host_failure(&error);

        assert_eq!(failure.code, "SSH-TIMEOUT");
        assert_eq!(failure.message_key, error.message_key());
        assert_eq!(failure.message_params.get("seconds").map(String::as_str), Some("2"));
        assert_eq!(failure.message_params.get("host").map(String::as_str), Some("10.0.0.7"));
        assert_eq!(failure.detail.as_deref(), Some("the command was still running"));
    }
}

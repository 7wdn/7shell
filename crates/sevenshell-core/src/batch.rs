//! Running one command on many hosts.
//!
//! Two rules shape this module, and both are about what a batch must not do:
//!
//! 1. **Connect to every host before running anything.** A host that needs a decision stops the
//!    run while it is still only a set of connections. Otherwise a batch would run the command on
//!    the first nine hosts, discover the tenth needs an answer, and run the command again on the
//!    nine after the answer — twice on nine hosts.
//! 2. **One host's trouble stays that host's.** Unreachable, no stored password, gone from the
//!    vault: each becomes that host's own answer, and the other hosts still run. The one thing
//!    that stops the batch is a decision a person has to make.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sevenshell_error::ShellError;
use sevenshell_policy::classify;
use sevenshell_ssh::SshSession;
use sevenshell_types::{
    BatchHostOutcome, BatchHostRef, BatchHostResult, BatchOutcome, BatchRequest, HostFailure, RiskAssessment, Session,
};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

use crate::Core;
use crate::sessions::{ConnectionOutcome, HostKeyDecision};

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

        // The gate, before a host is even resolved: a command dangerous enough to need a person
        // does not get as far as touching a server until somebody has seen why.
        if risk.needs_approval() && !request.acknowledged {
            return Ok(BatchOutcome::ApprovalRequired { risk });
        }

        let hosts: Vec<Host> = request
            .session_ids
            .iter()
            .map(|session_id| Host {
                session_id: session_id.clone(),
                session: self.session(session_id),
            })
            .collect();

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
    use sevenshell_types::{AuthMethod, Protocol, RiskLevel, SessionAuth, TerminalSettings};
    use sevenshell_vault::Vault;
    use tempfile::{TempDir, tempdir};

    use super::*;

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

//! One command on several real hosts, from the vault to the last exit status.
//!
//! Ignored by default: it needs a running sshd. `deploy/lab/` has one and `make lab-up` starts it.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 SEVENSHELL_TEST_USER=labuser \
//!   SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-core --test live_batch -- --ignored --test-threads=1
//! ```
//!
//! What only a real server can show: that every host gets its own answer from its own connection,
//! that a host which cannot connect is one row and not the whole run, and that a command which is
//! waiting on a person stops the run *before* anything has run anywhere.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sevenshell_core::{BatchEvent, Core};
use sevenshell_types::{
    AuthMethod, BatchHostOutcome, BatchHostResult, BatchOutcome, BatchRequest, HostFailure, Session, SessionAuth,
    TerminalSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;

/// The marker the "nothing has run yet" test looks for on the far side.
const MARKER: &str = "/tmp/seven-shell-batch-marker";

struct Lab {
    /// Owned so the vault file outlives the test.
    _directory: TempDir,
    core: Arc<Core>,
}

/// Two saved sessions pointing at the same lab server under different names.
///
/// The second name matters: a host key is trusted per host and port, so `lab-alias` arrives
/// untrusted even though it reaches the same machine — which is exactly what a decision needs, and
/// a second container would not be.
fn lab() -> Option<Lab> {
    let (host, port, user, password) = credentials()?;

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    let session = |id: &str, name: &str, host: &str| Session {
        id: id.to_owned(),
        name: name.to_owned(),
        group_id: None,
        protocol: sevenshell_types::Protocol::Ssh,
        host: host.to_owned(),
        port,
        user: user.clone(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    };

    vault
        .save_session(
            session("lab-session", "lab", &host),
            Some(SessionSecret::Password(password.clone())),
        )
        .expect("save session");
    vault
        .save_session(
            session("lab-alias", "lab (alias)", "localhost"),
            Some(SessionSecret::Password(password)),
        )
        .expect("save alias");

    let (core, _events) = Core::new(vault);
    Some(Lab {
        _directory: directory,
        core,
    })
}

fn credentials() -> Option<(String, u16, String, String)> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return None;
    };

    match port.parse() {
        Ok(port) => Some((host, port, user, password)),
        Err(_) => {
            eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
            None
        }
    }
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn request(session_ids: &[&str], command: &str, timeout_ms: u64) -> BatchRequest {
    BatchRequest {
        session_ids: session_ids.iter().map(|id| (*id).to_owned()).collect(),
        command: command.to_owned(),
        timeout_ms,
        parallel: 4,
        acknowledged: false,
    }
}

/// Runs a batch once, with the events it produced.
async fn run_once(lab: &Lab, request: BatchRequest) -> (BatchOutcome, Vec<BatchEvent>) {
    let (progress, mut events) = mpsc::channel(64);
    let outcome = lab.core.run_across(request, progress).await.expect("a batch runs");

    let mut collected = Vec::new();
    while let Ok(event) = events.try_recv() {
        collected.push(event);
    }
    (outcome, collected)
}

/// Trusts whatever host the run asks about, until the run stops asking: the same two steps the
/// interface takes when a person compares a fingerprint, trusts it, and asks again.
async fn trust_everything(lab: &Lab, ids: &[&str]) {
    for _ in 0..4 {
        let (outcome, _) = run_once(lab, request(ids, "true", 20_000)).await;
        match outcome {
            BatchOutcome::HostKeyUnknown { key } => lab.core.trust_host_key(&key).expect("trust the key"),
            _ => return,
        }
    }
    panic!("the run never got past the host keys");
}

/// A batch on hosts whose keys are already trusted.
async fn run(lab: &Lab, ids: &[&str], command: &str, timeout_ms: u64) -> (BatchOutcome, Vec<BatchEvent>) {
    trust_everything(lab, ids).await;
    run_once(lab, request(ids, command, timeout_ms)).await
}

fn finished(outcome: BatchOutcome) -> Vec<BatchHostResult> {
    match outcome {
        BatchOutcome::Finished { results, .. } => results,
        other => panic!("expected a finished run, got {other:?}"),
    }
}

fn ran(result: &BatchHostResult) -> (Option<u32>, &str) {
    match &result.outcome {
        BatchHostOutcome::Ran { exit_code, stdout, .. } => (*exit_code, stdout.as_str()),
        other => panic!("expected {} to have run, got {other:?}", result.session_id),
    }
}

fn failure(result: &BatchHostResult) -> &HostFailure {
    match &result.outcome {
        BatchHostOutcome::Failed { error } => error,
        other => panic!("expected {} to have failed, got {other:?}", result.session_id),
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn every_host_runs_the_command_and_answers_for_itself() {
    let Some(lab) = lab() else {
        return;
    };

    let ids = ["lab-session", "lab-alias"];
    let (outcome, _) = run(&lab, &ids, "echo from-seven-shell; exit 3", 20_000).await;
    let results = finished(outcome);

    assert_eq!(results.len(), 2, "one row per host");
    assert_eq!(results[0].session_id, "lab-session");
    assert_eq!(
        results[1].session_id, "lab-alias",
        "the order is the request's, not the network's"
    );

    for result in &results {
        let (exit_code, stdout) = ran(result);
        assert_eq!(stdout, "from-seven-shell\n");
        assert_eq!(exit_code, Some(3), "the command's own status, per host");
        assert!(
            !result.session_name.is_empty(),
            "a row is readable without looking the id up"
        );
    }
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_host_that_is_not_in_the_vault_is_one_row_and_not_the_run() {
    let Some(lab) = lab() else {
        return;
    };

    let ids = ["lab-session", "session-that-never-existed"];
    let (outcome, _) = run(&lab, &ids, "echo alive", 20_000).await;
    let results = finished(outcome);

    assert_eq!(results.len(), 2);
    let (exit_code, stdout) = ran(&results[0]);
    assert_eq!(
        (exit_code, stdout),
        (Some(0), "alive\n"),
        "the host that exists still ran"
    );

    assert_eq!(failure(&results[1]).code, "SESSION-NOT-FOUND");
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_command_that_takes_too_long_is_that_hosts_own_timeout() {
    let Some(lab) = lab() else {
        return;
    };

    let ids = ["lab-session", "lab-alias"];
    let started = Instant::now();
    let (outcome, _) = run(&lab, &ids, "sleep 60", 2_000).await;
    let results = finished(outcome);
    let elapsed = started.elapsed();

    assert_eq!(results.len(), 2);
    for result in &results {
        let error = failure(result);
        assert_eq!(error.code, "SSH-TIMEOUT");
        assert_eq!(
            error.message_params.get("seconds").map(String::as_str),
            Some("2"),
            "the message can say how long it waited"
        );
    }

    // The timeout is per host and they run together: two two-second timeouts are not four seconds
    // of waiting, and neither is sixty.
    assert!(elapsed < Duration::from_secs(30), "the run took {elapsed:?}");
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn an_acknowledged_dangerous_command_runs_and_is_still_graded() {
    let Some(lab) = lab() else {
        return;
    };

    // Dangerous by the classifier's reading, harmless in the lab: the point is that the
    // acknowledgement lets it through and the verdict still travels with the result.
    let ids = ["lab-session"];
    let mut request = request(&ids, "rm -rf /tmp/seven-shell-scratch", 20_000);
    request.acknowledged = true;
    trust_everything(&lab, &ids).await;

    let (outcome, _) = run_once(&lab, request).await;

    let BatchOutcome::Finished { risk, results, .. } = outcome else {
        panic!("expected a finished run, got {outcome:?}");
    };
    assert!(
        risk.needs_approval(),
        "the verdict is reported, not erased by the acknowledgement"
    );
    assert!(
        risk.reasons
            .iter()
            .any(|reason| reason.rule_id == "recursive-force-delete")
    );
    assert_eq!(ran(&results[0]).0, Some(0));
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_decision_stops_the_run_before_anything_has_run() {
    let Some(lab) = lab() else {
        return;
    };

    // Trust the first host only. The alias stays unknown, and the run must stop because of it —
    // after having done nothing at all on the host that is ready.
    trust_everything(&lab, &["lab-session"]).await;

    let (outcome, _) = run_once(
        &lab,
        request(&["lab-session", "lab-alias"], &format!("touch {MARKER}"), 20_000),
    )
    .await;

    match outcome {
        BatchOutcome::HostKeyUnknown { key } => {
            assert_eq!(key.host, "localhost", "the decision is about the host that needs it");
        }
        other => panic!("expected a decision about the alias, got {other:?}"),
    }

    // Nothing ran anywhere: the command would have created the marker on the server, and the
    // server is the same machine for both names.
    let look = format!("test -e {MARKER} && echo present || echo absent");
    let (outcome, _) = run(&lab, &["lab-session"], &look, 20_000).await;
    assert_eq!(ran(&finished(outcome)[0]).1, "absent\n", "the ready host ran nothing");

    // And once the alias is trusted too, the very same batch does create it — which is what makes
    // the assertion above mean something.
    let ids = ["lab-session", "lab-alias"];
    let (outcome, _) = run(&lab, &ids, &format!("touch {MARKER}"), 20_000).await;
    for result in &finished(outcome) {
        assert_eq!(ran(result).0, Some(0));
    }

    let (outcome, _) = run(&lab, &["lab-session"], &look, 20_000).await;
    assert_eq!(ran(&finished(outcome)[0]).1, "present\n");

    let (outcome, _) = run(&lab, &["lab-session"], &format!("rm -f {MARKER}"), 20_000).await;
    assert_eq!(ran(&finished(outcome)[0]).0, Some(0));
}

#[tokio::test]
#[ignore = "needs the SSH lab: see deploy/lab/README.md"]
async fn a_reader_watching_the_progress_sees_every_host() {
    let Some(lab) = lab() else {
        return;
    };

    // Trust first, so the events below are from the run that actually runs the command.
    let ids = ["lab-session", "lab-alias"];
    trust_everything(&lab, &ids).await;

    let (outcome, events) = run_once(&lab, request(&ids, "echo ready", 20_000)).await;
    let results = finished(outcome);

    let mut started = Vec::new();
    let mut finished_ids = Vec::new();
    for event in &events {
        match event {
            BatchEvent::HostStarted { session_id } => started.push(session_id.clone()),
            BatchEvent::HostFinished { result } => finished_ids.push(result.session_id.clone()),
        }
    }

    assert_eq!(started.len(), 2, "each host says it has started: {started:?}");
    assert_eq!(finished_ids.len(), 2, "each host says it has finished");
    assert_eq!(results.len(), 2);
}

//! The gate a mutating tool walks through, and the words it leaves behind.
//!
//! Four things are decided here, once, so that no tool can get them subtly wrong:
//!
//! * **How long a call may take.** A read is bounded at thirty seconds and a command by the budget
//!   the caller asked for; both are wrapped in [`Cancel`] so a call somebody stopped stops.
//! * **Whether a person has to agree.** The policy matrix is asked first: a deny refuses the call
//!   without a question, and a destructive verdict — or a safe or caution one the matrix raises —
//!   turns into one question on the desktop where a "no" or a silence is a refusal, never a
//!   default yes.
//! * **What an audit line is allowed to say.** A command line can carry a password, so it is masked
//!   and cut before it becomes a subject; output never becomes one at all.
//! * **How much of an answer is returned.** A stream is 64 KiB, a file's text and a resource body
//!   256 KiB, a listing 500 rows: an agent that receives a 40 MB log has learned nothing and cost the
//!   machine a copy of it.
//!
//! The codes below start with `MCP-` because they belong to this layer rather than to the catalog:
//! an agent is the only reader of them, and the interface shows the tool's own failures through the
//! approval record and the audit line instead.

use std::future::Future;
use std::time::Duration;

use sevenshell_error::{MASK, truncate_chars};
use sevenshell_policy::PolicyDecision;
use sevenshell_types::{PolicyAction, RiskAssessment, RiskLevel};

use crate::context::{ApprovalDecision, ApprovalRecord, ApprovalRequest, ToolContext};
use crate::tool::{ToolAudit, ToolError};

/// The call did not finish inside its budget.
pub(crate) const TIMEOUT_CODE: &str = "MCP-TIMEOUT";
/// The answer would have been larger than the interface returns.
pub(crate) const TOO_LARGE_CODE: &str = "MCP-RESULT-TOO-LARGE";

/// How long a read — a listing, a file, a container list, one API path — may take.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// How much of one output stream is returned. Past this an agent is reading a file, not a result.
pub(crate) const STREAM_LIMIT: usize = 64 * 1024;
/// How much text one file read returns, and how much of a resource body.
pub(crate) const TEXT_LIMIT: usize = 256 * 1024;
/// How many rows a listing returns.
pub(crate) const LIST_LIMIT: usize = 500;
/// How many sessions one batch may name. The contract froze this number; the core's own ceiling is
/// per-host concurrency, which is a different question.
pub(crate) const BATCH_LIMIT: usize = 32;

/// A command runs for thirty seconds unless the caller said otherwise.
pub(crate) const EXEC_TIMEOUT_MS: u64 = 30_000;
/// And never for less than a second or more than half an hour, which is the core's own range.
pub(crate) const EXEC_TIMEOUT_MIN_MS: u64 = 1_000;
pub(crate) const EXEC_TIMEOUT_MAX_MS: u64 = 1_800_000;

/// How much of a command line or a path an audit line keeps.
const SUBJECT_CHARS: usize = 400;

/// Tokens that are a secret flag followed by the secret in the next token.
const BARE_SECRET_FLAGS: &[&str] = &[
    "-p",
    "-pw",
    "--password",
    "--pass",
    "--passphrase",
    "--token",
    "--secret",
];

/// Tokens that start with `-p` and are flags rather than a password glued to the flag. Masking these
/// would make an audit line say less than it should, which is its own kind of wrong.
const SHORT_FLAGS_STARTING_WITH_P: &[&str] = &[
    "-print",
    "-print0",
    "-perm",
    "-path",
    "-prune",
    "-pid",
    "-pids",
    "-proxy",
    "-prefix",
    "-port",
    "-protocol",
];

/// The caller's budget, never below a second and never above half an hour.
pub(crate) fn timeout_ms(asked: Option<u64>) -> u64 {
    asked
        .unwrap_or(EXEC_TIMEOUT_MS)
        .clamp(EXEC_TIMEOUT_MIN_MS, EXEC_TIMEOUT_MAX_MS)
}

/// Runs one step under the call's cancellation and a wall-clock budget.
///
/// The budget is a backstop rather than the only limit: the core applies its own per-host timeouts,
/// and this is what keeps a step the core does not bound — a probe, a download of a huge body — from
/// holding a call open for ever.
pub(crate) async fn bounded<T>(
    context: &ToolContext,
    budget: Duration,
    step: impl Future<Output = Result<T, ToolError>>,
) -> Result<T, ToolError> {
    match tokio::time::timeout(budget, context.cancel().guard(step)).await {
        Ok(result) => result,
        Err(_) => Err(ToolError::new(TIMEOUT_CODE)
            .with_message(format!("the call did not finish within {} seconds", budget.as_secs()))),
    }
}

/// Asks the installation's matrix about a verdict, then a person when the matrix asks for one.
///
/// The matrix comes first because it is the narrower rule: a `Deny` is final and must not cost
/// somebody a question they cannot answer usefully, and an `Allow` is the installation saying this
/// action needs no question at all — which only a safe or caution verdict can be, because the
/// resolver raises both destructive levels to `Ask` no matter what a layer says.
///
/// `directory` is the remote path the action names, when the tool knows one. It is the layer that
/// makes "nothing under /etc" possible; a tool that does not act on a path passes `None` rather
/// than inventing one.
///
/// Answers `None` when no approval was needed. The record comes back otherwise, so the caller can
/// put the same decision in the audit line a successful call leaves; a refusal comes back as an
/// error that already carries it, because a refused call is exactly the one somebody will want to
/// explain later.
pub(crate) async fn approve_risk(
    context: &ToolContext,
    tool: &str,
    risk: &RiskAssessment,
    session: Option<(&str, Option<String>)>,
    directory: Option<&str>,
    subject: &str,
) -> Result<Option<ApprovalRecord>, ToolError> {
    let decision = policy_decision(context, tool, risk, &session, directory, subject);
    match decision.action {
        PolicyAction::Deny => {
            return Err(ToolError::denied(tool).with_audit(ToolAudit {
                session_id: session.map(|(session_id, _)| session_id.to_owned()),
                subject: Some(subject.to_owned()),
                risk: Some(risk.clone()),
                approval: None,
            }));
        }
        PolicyAction::Allow => return Ok(None),
        PolicyAction::Ask => {}
    }

    let request = ApprovalRequest::risk(tool, risk.clone()).with_subject(subject);
    let request = match &session {
        Some((session_id, name)) => request.for_session(*session_id, name.clone()),
        None => request,
    };
    let record = context.request_approval(request).await?;

    match record.decision {
        ApprovalDecision::Approved => Ok(Some(record)),
        ApprovalDecision::Denied | ApprovalDecision::TimedOut => Err(refusal(
            record,
            risk,
            session,
            subject,
            "the person at the desktop denied this action",
        )),
    }
}

/// The matrix's verdict for one tool call, with every layer the call can name.
///
/// The session's group, host and port come from the core's own lookup, so a tool cannot pass a
/// target that disagrees with the session it named. A session that cannot be read contributes no
/// layer — the tool is about to fail with that session's error, and the global rule still applies.
fn policy_decision(
    context: &ToolContext,
    tool: &str,
    risk: &RiskAssessment,
    session: &Option<(&str, Option<String>)>,
    directory: Option<&str>,
    subject: &str,
) -> PolicyDecision {
    let saved = session
        .as_ref()
        .and_then(|(session_id, _)| context.core().session_policy_target(session_id));
    let mut target = saved.as_ref().map(|saved| saved.as_target()).unwrap_or_default();
    target.directory = directory;

    context.core().policy_decision(
        tool,
        risk,
        target,
        session.as_ref().map(|(session_id, _)| *session_id),
        Some(subject),
    )
}

/// Asks a person to trust a host key that has never been seen, and refuses unless they agree.
///
/// The subject is the fingerprint: that is the thing a person is asked to compare with what the
/// server's administrator told them, and putting it on the prompt is what makes the comparison
/// possible without a second screen.
pub(crate) async fn approve_host_key(
    context: &ToolContext,
    tool: &str,
    key: &sevenshell_types::PresentedHostKey,
    session: Option<(&str, Option<String>)>,
) -> Result<ApprovalRecord, ToolError> {
    let subject = format!("{}:{} {}", key.host, key.port, key.fingerprint);
    let request = ApprovalRequest::host_key(tool, key.clone()).with_subject(subject.clone());
    let request = match &session {
        Some((session_id, name)) => request.for_session(*session_id, name.clone()),
        None => request,
    };
    let record = context.request_approval(request).await?;

    match record.decision {
        ApprovalDecision::Approved => Ok(record),
        ApprovalDecision::Denied | ApprovalDecision::TimedOut => Err(refusal(
            record,
            &RiskAssessment::safe(),
            session,
            &subject,
            "the host key was not trusted, so nothing was sent to the server",
        )),
    }
}

/// The refusal a person's answer produces, worded for the agent that has to report it.
fn refusal(
    record: ApprovalRecord,
    risk: &RiskAssessment,
    session: Option<(&str, Option<String>)>,
    subject: &str,
    denied: &str,
) -> ToolError {
    let why = match record.decision {
        ApprovalDecision::Denied => denied,
        ApprovalDecision::TimedOut => "nobody answered the approval request before it expired",
        // Not reachable: the callers answer the approved case before building a refusal.
        ApprovalDecision::Approved => "the action was approved",
    };
    let graded = !(risk.reasons.is_empty() && risk.level == RiskLevel::Safe);

    ToolError::denied(subject).with_message(why).with_audit(ToolAudit {
        session_id: session.map(|(session_id, _)| session_id.to_owned()),
        subject: Some(subject.to_owned()),
        risk: graded.then(|| risk.clone()),
        approval: Some(record),
    })
}

/// A command line or a path, as an audit line may keep it.
///
/// The envelope's own scrubber masks the shapes it knows (credential assignments, bearer tokens,
/// private key blocks); the pass here adds the two an operations console sees constantly and that
/// scrubber cannot see — a password glued to a flag (`mysql -phunter2`) and one inside a URL
/// (`https://user:hunter2@host`).
pub(crate) fn subject(text: &str) -> String {
    truncate_chars(&sevenshell_error::scrub(&mask_glued_secrets(text)), SUBJECT_CHARS)
}

/// Masks a secret that is part of the same token as the flag or the address carrying it.
fn mask_glued_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut mask_next = false;

    for piece in text.split_inclusive(char::is_whitespace) {
        let token = piece.trim_end_matches(char::is_whitespace);
        let whitespace = &piece[token.len()..];

        if token.is_empty() {
            out.push_str(whitespace);
            continue;
        }

        if mask_next {
            out.push_str(MASK);
            mask_next = false;
        } else {
            out.push_str(&mask_token(token));
            mask_next = BARE_SECRET_FLAGS.contains(&token.to_ascii_lowercase().as_str());
        }
        out.push_str(whitespace);
    }

    out
}

fn mask_token(token: &str) -> String {
    if let Some(masked) = mask_url_password(token) {
        return masked;
    }

    let lowered = token.to_ascii_lowercase();
    let glued = lowered.starts_with("-p")
        && lowered.len() > 2
        && !SHORT_FLAGS_STARTING_WITH_P.contains(&lowered.as_str())
        && !lowered.starts_with("--");
    if glued {
        return format!("-p{MASK}");
    }

    token.to_owned()
}

/// `scheme://user:password@host/path` with the password replaced.
fn mask_url_password(token: &str) -> Option<String> {
    let scheme_end = token.find("://")? + 3;
    let rest = &token[scheme_end..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let at = authority.rfind('@')?;
    let userinfo = &authority[..at];
    let colon = userinfo.find(':')?;

    Some(format!(
        "{}{}:{}{}{}",
        &token[..scheme_end],
        &userinfo[..colon],
        MASK,
        &authority[at..],
        &rest[authority_end..]
    ))
}

/// Cuts one stream to the limit, on a character boundary, and says whether it cut anything.
pub(crate) fn clip(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }

    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

/// The rows of a listing that are returned, how many there were, and whether any were left out.
///
/// The total travels with the cut so an agent can say "there are 4 100 files and you are looking at
/// the first 500" instead of believing the directory has 500 entries.
pub(crate) fn rows<T>(all: &[T]) -> (&[T], usize, bool) {
    let total = all.len();
    let returned = total.min(LIST_LIMIT);
    (&all[..returned], total, total > returned)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use sevenshell_core::Core;
    use sevenshell_policy::{PolicyAuditEvent, PolicyJournal};
    use sevenshell_types::{McpScope, PolicyMatrix, PolicyRule, RiskReason};
    use sevenshell_vault::Vault;
    use tempfile::tempdir;

    use crate::context::ApprovalBroker;

    use super::*;

    #[derive(Default)]
    struct RecordingJournal {
        events: std::sync::Mutex<Vec<PolicyAuditEvent>>,
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

    /// A broker that would say yes to anything and remembers being asked.
    #[derive(Default)]
    struct CountingBroker {
        asked: AtomicUsize,
    }

    #[async_trait]
    impl ApprovalBroker for CountingBroker {
        async fn request(&self, _request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            Ok(ApprovalDecision::Approved)
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

    fn context_with(
        global: PolicyRule,
        journal: &Arc<RecordingJournal>,
    ) -> (tempfile::TempDir, ToolContext, Arc<CountingBroker>) {
        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        let (core, _events) = Core::new(vault);
        core.set_policy(
            PolicyMatrix {
                global,
                ..PolicyMatrix::default()
            },
            Some(Arc::clone(journal) as Arc<dyn PolicyJournal>),
        );

        let broker = Arc::new(CountingBroker::default());
        let context =
            ToolContext::new(core, McpScope::SafeWrite).with_approvals(Arc::clone(&broker) as Arc<dyn ApprovalBroker>);
        (directory, context, broker)
    }

    fn destructive() -> RiskAssessment {
        RiskAssessment {
            level: RiskLevel::Destructive,
            reasons: vec![RiskReason::new(
                "recursive-force-delete",
                "risk.recursiveForceDelete",
                RiskLevel::Destructive,
            )],
        }
    }

    #[tokio::test]
    async fn a_policy_deny_refuses_before_the_approval_broker_is_asked() {
        let journal = Arc::new(RecordingJournal::default());
        let (_directory, context, broker) = context_with(rule(PolicyAction::Deny), &journal);

        let error = approve_risk(
            &context,
            "sshell_exec_run",
            &destructive(),
            None,
            None,
            "rm -rf /srv/app",
        )
        .await
        .expect_err("a deny refuses the action");

        assert_eq!(error.code(), "POLICY-DENIED");
        assert_eq!(broker.asked.load(Ordering::SeqCst), 0, "a deny is not a question");
        let events = journal.events();
        assert_eq!(events.len(), 1, "a refused action is still on the trail");
        assert_eq!(events[0].action, "sshell_exec_run");
        assert_eq!(events[0].decision, PolicyAction::Deny);
        assert_eq!(events[0].subject.as_deref(), Some("rm -rf /srv/app"));
    }

    #[tokio::test]
    async fn the_default_rule_still_asks_about_a_destructive_verdict() {
        let journal = Arc::new(RecordingJournal::default());
        let (_directory, context, broker) = context_with(PolicyRule::default(), &journal);

        let approval = approve_risk(
            &context,
            "sshell_exec_run",
            &destructive(),
            None,
            None,
            "rm -rf /srv/app",
        )
        .await
        .expect("an approved action");

        assert_eq!(
            approval.map(|approval| approval.decision),
            Some(ApprovalDecision::Approved),
            "the record goes back to the caller for its audit line"
        );
        assert_eq!(broker.asked.load(Ordering::SeqCst), 1);
        assert_eq!(journal.events()[0].decision, PolicyAction::Ask);
    }

    #[tokio::test]
    async fn an_allowing_matrix_answers_without_asking_or_claiming_an_approval() {
        let journal = Arc::new(RecordingJournal::default());
        let (_directory, context, broker) = context_with(PolicyRule::default(), &journal);

        // The write tool's own verdict: caution, because the file changed, but no classifier rule.
        let risk = RiskAssessment {
            level: RiskLevel::Caution,
            reasons: Vec::new(),
        };
        let approval = approve_risk(
            &context,
            "sshell_sftp_write_text",
            &risk,
            None,
            Some("/srv/app.conf"),
            "/srv/app.conf",
        )
        .await
        .expect("an allowed action needs no person");

        assert!(approval.is_none(), "no question means no approval to record");
        assert_eq!(broker.asked.load(Ordering::SeqCst), 0);
        let events = journal.events();
        assert_eq!(events.len(), 1, "the decision is journaled all the same");
        assert_eq!(events[0].decision, PolicyAction::Allow);
        assert_eq!(events[0].directory.as_deref(), Some("/srv/app.conf"));
    }

    #[test]
    fn an_audit_subject_masks_a_password_glued_to_its_flag() {
        assert_eq!(
            subject("mysql -phunter2 -e 'select 1'"),
            "mysql -p[redacted] -e 'select 1'"
        );
        assert_eq!(
            subject("mysql -p hunter2 -e 'select 1'"),
            "mysql -p [redacted] -e 'select 1'"
        );
        assert_eq!(subject("tool --password hunter2"), "tool --password [redacted]");
        assert_eq!(subject("tool --password=hunter2"), "tool --password=[redacted]");
    }

    #[test]
    fn an_audit_subject_leaves_the_flags_that_only_start_with_p_alone() {
        assert_eq!(
            subject("find /srv -print -perm 644"),
            "find /srv -print -perm 644",
            "a flag is not a password"
        );
    }

    #[test]
    fn an_audit_subject_masks_the_password_inside_a_url() {
        assert_eq!(
            subject("curl https://lab:hunter2@example.test/api"),
            "curl https://lab:[redacted]@example.test/api"
        );
    }

    #[test]
    fn an_audit_subject_masks_a_bearer_token() {
        let masked = subject("curl -H 'Authorization: Bearer abcdef123456' https://api.example.test");
        assert!(!masked.contains("abcdef123456"), "{masked}");
    }

    #[test]
    fn an_audit_subject_is_cut_rather_than_kept_whole() {
        let long = "echo ".to_owned() + &"a".repeat(10_000);
        let masked = subject(&long);
        assert!(
            masked.chars().count() <= SUBJECT_CHARS + 32,
            "{}",
            masked.chars().count()
        );
        assert!(masked.ends_with("[truncated]"), "the cut is admitted, not hidden");
    }

    #[test]
    fn a_stream_is_cut_on_a_character_boundary() {
        let text = "é".repeat(10);
        let (clipped, cut) = clip(&text, 5);
        assert!(cut);
        assert_eq!(clipped, "éé", "five bytes cannot hold three two-byte characters");

        let (kept, cut) = clip("hello", 5);
        assert!(!cut);
        assert_eq!(kept, "hello");
    }

    #[test]
    fn a_listing_is_cut_at_the_row_limit_and_says_how_many_were_left() {
        let all: Vec<u32> = (0..=LIST_LIMIT as u32).collect();
        let (returned, total, truncated) = rows(&all);
        assert_eq!(returned.len(), LIST_LIMIT);
        assert_eq!(total, LIST_LIMIT + 1);
        assert!(truncated);

        let short: Vec<u32> = (0..3).collect();
        let (returned, total, truncated) = rows(&short);
        assert_eq!(returned.len(), 3);
        assert_eq!(total, 3);
        assert!(!truncated);
    }

    #[test]
    fn the_command_budget_is_clamped_to_the_stated_range() {
        assert_eq!(timeout_ms(None), EXEC_TIMEOUT_MS);
        assert_eq!(timeout_ms(Some(0)), EXEC_TIMEOUT_MIN_MS);
        assert_eq!(timeout_ms(Some(5_000)), 5_000);
        assert_eq!(timeout_ms(Some(u64::MAX)), EXEC_TIMEOUT_MAX_MS);
    }
}

//! The gate a mutating tool walks through, and the words it leaves behind.
//!
//! Four things are decided here, once, so that no tool can get them subtly wrong:
//!
//! * **How long a call may take.** A read is bounded at thirty seconds and a command by the budget
//!   the caller asked for; both are wrapped in [`Cancel`] so a call somebody stopped stops.
//! * **Whether a person has to agree.** A destructive verdict turns into one question on the desktop,
//!   and a "no" or a silence is a refusal — never a default yes.
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
use sevenshell_types::{RiskAssessment, RiskLevel};

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

/// Asks a person about a destructive verdict, and refuses unless they agree.
///
/// The record comes back so the caller can put the same decision in the audit line a successful call
/// leaves; a refusal comes back as an error that already carries it, because a refused call is
/// exactly the one somebody will want to explain later.
pub(crate) async fn approve_risk(
    context: &ToolContext,
    tool: &str,
    risk: &RiskAssessment,
    session: Option<(&str, Option<String>)>,
    subject: &str,
) -> Result<ApprovalRecord, ToolError> {
    let request = ApprovalRequest::risk(tool, risk.clone()).with_subject(subject);
    let request = match &session {
        Some((session_id, name)) => request.for_session(*session_id, name.clone()),
        None => request,
    };
    let record = context.request_approval(request).await?;

    match record.decision {
        ApprovalDecision::Approved => Ok(record),
        ApprovalDecision::Denied | ApprovalDecision::TimedOut => Err(refusal(
            record,
            risk,
            session,
            subject,
            "the person at the desktop denied this action",
        )),
    }
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
    use super::*;

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

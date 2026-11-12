//! The installation's policy matrix, and the trail every decision leaves.
//!
//! Two things live behind one lock on purpose: a decision made with one matrix must not be written
//! to the trail of another, which is what replacing them separately would allow. The lock is never
//! held across the journal call — a journal writes to a file, and a file that is slow must not stop
//! every other decision in the process — so the handle is cloned out first and the write happens
//! after the snapshot is released.
//!
//! The subject is scrubbed and bounded here rather than at each call site: a caller that forgets is
//! a caller that writes a password into an audit file, and there is no second chance to un-write it.
//! A command line is the input most likely to carry one, which is why the batch path below names
//! the classifier's rule ids instead of the command it ran.

use std::sync::{Arc, MutexGuard};

use sevenshell_error::scrub_bounded;
use sevenshell_policy::{PolicyDecision, PolicyJournal, PolicyTarget, decide};
use sevenshell_types::{PolicyMatrix, RiskAssessment};

use crate::Core;

/// How much of a subject an audit line keeps. The tool layer's gate uses the same bound, so one
/// action reads the same whether its line came from a tool or from the core.
const SUBJECT_CHARS: usize = 400;

/// The matrix and the trail, replaced together.
#[derive(Default)]
pub(crate) struct PolicySnapshot {
    pub(crate) matrix: PolicyMatrix,
    pub(crate) journal: Option<Arc<dyn PolicyJournal>>,
}

/// A saved session's group, host and port, owned so it outlives the vault read that produced it.
///
/// Every field is what the matrix needs to match a layer and nothing else: no user, no protocol, no
/// secret. A caller that has only a connection id cannot build one of these, which is why the
/// container and cluster paths decide against the global rule only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPolicyTarget {
    pub group_id: Option<String>,
    pub host: String,
    pub port: u16,
}

impl SessionPolicyTarget {
    /// The borrowed shape the matrix takes. The directory is left out on purpose: a session does
    /// not name a remote path, and a caller that knows one fills it in itself.
    pub fn as_target(&self) -> PolicyTarget<'_> {
        PolicyTarget {
            group_id: self.group_id.as_deref(),
            host: Some(self.host.as_str()),
            port: Some(self.port),
            directory: None,
        }
    }
}

impl Core {
    /// Replaces the matrix and the trail together.
    ///
    /// A host calls this at startup, on settings save and after the vault opens. Until it does, the
    /// default rule applies — safe and caution run, the destructive levels ask — and nothing is
    /// journaled, which is exactly the behaviour every build had before the matrix existed.
    pub fn set_policy(&self, matrix: PolicyMatrix, journal: Option<Arc<dyn PolicyJournal>>) {
        let mut policy = self.policy_snapshot();
        policy.matrix = matrix;
        policy.journal = journal;
    }

    /// Decides one action and journals it, exactly once.
    ///
    /// `target` names the layers that apply; `session_id` and `subject` are copied into the audit
    /// line, not into the decision. The subject is scrubbed and cut here because this is the last
    /// place that sees it before it becomes permanent.
    pub fn policy_decision(
        &self,
        action: &str,
        risk: &RiskAssessment,
        target: PolicyTarget<'_>,
        session_id: Option<&str>,
        subject: Option<&str>,
    ) -> PolicyDecision {
        let (decision, journal) = {
            let policy = self.policy_snapshot();
            let decision = decide(&policy.matrix, target, risk);
            (decision, policy.journal.clone())
        };

        // Outside the lock: the journal writes to a file, and a decision must not wait on a disk.
        if let Some(journal) = journal {
            journal.record(decision.audit_event(
                crate::sessions::now_seconds(),
                action.to_owned(),
                subject.map(|text| scrub_bounded(text, SUBJECT_CHARS)),
                session_id.map(str::to_owned),
                target,
            ));
        }
        decision
    }

    /// The policy target a saved session names, or `None` when it cannot be read.
    ///
    /// A missing session is not an error here: the caller is about to fail with the session's own
    /// error, and the decision that matters in the meantime is the global rule.
    pub fn session_policy_target(&self, session_id: &str) -> Option<SessionPolicyTarget> {
        self.session(session_id).ok().map(|session| SessionPolicyTarget {
            group_id: session.group_id,
            host: session.host,
            port: session.port,
        })
    }

    fn policy_snapshot(&self) -> MutexGuard<'_, PolicySnapshot> {
        // A panic while replacing the matrix must not turn every later decision into a crash: what
        // the poisoned snapshot holds is still a complete matrix and trail.
        self.policy.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sevenshell_policy::PolicyAuditEvent;
    use sevenshell_types::{PolicyAction, PolicyRule, RiskLevel, RiskReason};
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

    fn core_with_journal() -> (TempDir, Arc<Core>, Arc<RecordingJournal>) {
        let directory = tempdir().expect("temp dir");
        let (core, _events) = Core::new(Arc::new(Vault::open_at(directory.path().join("vault.json"))));
        let journal = Arc::new(RecordingJournal::default());
        (directory, core, journal)
    }

    fn risk(level: RiskLevel) -> RiskAssessment {
        RiskAssessment {
            level,
            reasons: vec![RiskReason::new("test-rule", "risk.test", level)],
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

    #[test]
    fn an_allow_is_journaled_once_with_the_target_it_looked_at() {
        let (_directory, core, journal) = core_with_journal();
        core.set_policy(
            PolicyMatrix::default(),
            Some(Arc::clone(&journal) as Arc<dyn PolicyJournal>),
        );

        let target = PolicyTarget {
            group_id: Some("lab"),
            host: Some("web"),
            port: Some(2222),
            directory: Some("/srv/app"),
        };
        let decision = core.policy_decision(
            "sshell_exec_run",
            &risk(RiskLevel::Safe),
            target,
            Some("session-1"),
            Some("uptime"),
        );

        assert_eq!(decision.action, PolicyAction::Allow);
        let events = journal.events();
        assert_eq!(events.len(), 1, "one decision, one line");
        let event = &events[0];
        assert_eq!(event.action, "sshell_exec_run");
        assert_eq!(event.decision, PolicyAction::Allow);
        assert_eq!(event.source, "global");
        assert_eq!(event.subject.as_deref(), Some("uptime"));
        assert_eq!(event.session_id.as_deref(), Some("session-1"));
        assert_eq!(event.group_id.as_deref(), Some("lab"));
        assert_eq!(event.host.as_deref(), Some("web"));
        assert_eq!(event.directory.as_deref(), Some("/srv/app"));
        assert!(event.at > 0, "the line is timestamped");
    }

    #[test]
    fn a_group_cannot_loosen_a_global_ask() {
        let (_directory, core, _journal) = core_with_journal();
        let matrix = PolicyMatrix {
            global: PolicyRule {
                caution: PolicyAction::Ask,
                ..PolicyRule::default()
            },
            groups: [("lab".to_owned(), rule(PolicyAction::Allow))].into(),
            ..PolicyMatrix::default()
        };
        core.set_policy(matrix, None);

        let decision = core.policy_decision(
            "batch.exec",
            &risk(RiskLevel::Caution),
            PolicyTarget {
                group_id: Some("lab"),
                ..PolicyTarget::default()
            },
            None,
            None,
        );

        assert_eq!(
            decision.action,
            PolicyAction::Ask,
            "a group cannot undo the global rule"
        );
        assert_eq!(decision.source, sevenshell_policy::PolicySource::Global);
    }

    #[test]
    fn a_host_rule_can_tighten_a_safe_action_to_ask() {
        let (_directory, core, _journal) = core_with_journal();
        let matrix = PolicyMatrix {
            hosts: [("web:2222".to_owned(), rule(PolicyAction::Ask))].into(),
            ..PolicyMatrix::default()
        };
        core.set_policy(matrix, None);

        let decision = core.policy_decision(
            "batch.exec",
            &risk(RiskLevel::Safe),
            PolicyTarget {
                host: Some("web"),
                port: Some(2222),
                ..PolicyTarget::default()
            },
            None,
            None,
        );

        assert_eq!(
            decision.action,
            PolicyAction::Ask,
            "the matrix raises a harmless action"
        );
        assert_eq!(
            decision.source,
            sevenshell_policy::PolicySource::Host("web:2222".to_owned())
        );
    }

    #[test]
    fn a_directory_deny_wins_over_every_allowing_layer() {
        let (_directory, core, _journal) = core_with_journal();
        let matrix = PolicyMatrix {
            directories: [("/prod".to_owned(), rule(PolicyAction::Deny))].into(),
            ..PolicyMatrix::default()
        };
        core.set_policy(matrix, None);

        let decision = core.policy_decision(
            "sshell_sftp_write_text",
            &risk(RiskLevel::Safe),
            PolicyTarget {
                directory: Some("/prod/releases/web.conf"),
                ..PolicyTarget::default()
            },
            None,
            None,
        );

        assert!(decision.denied(), "a deny is never outvoted");
        assert_eq!(
            decision.source,
            sevenshell_policy::PolicySource::Directory("/prod".to_owned())
        );
    }

    #[test]
    fn a_password_shaped_subject_is_masked_and_bounded() {
        let (_directory, core, journal) = core_with_journal();
        core.set_policy(
            PolicyMatrix::default(),
            Some(Arc::clone(&journal) as Arc<dyn PolicyJournal>),
        );

        let subject = format!("mysql -e 'password=hunter2' {}", "x".repeat(2048));
        core.policy_decision(
            "sshell_exec_run",
            &risk(RiskLevel::Safe),
            PolicyTarget::default(),
            Some("session-1"),
            Some(&subject),
        );

        let events = journal.events();
        let kept = events[0].subject.as_deref().expect("the subject travels");
        assert!(!kept.contains("hunter2"), "a password shape must be masked: {kept}");
        assert!(kept.contains("[redacted]"), "the mask is visible, not silent: {kept}");
        assert!(
            kept.chars().count() <= SUBJECT_CHARS + 20,
            "an audit line is bounded, got {} characters",
            kept.chars().count()
        );
    }
}

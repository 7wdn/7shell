//! The four-layer policy matrix: global, group, host and directory.
//!
//! A layer can only make a decision stricter than the one above it. "Most restrictive wins" is the
//! contract, and it is why a matching `Allow` in a group cannot undo a global `Ask`: the result is
//! the maximum of every layer that applies. The source recorded with the decision is the most
//! specific layer that supplied the winning action, which is what an audit line has to name.
//!
//! The matrix is deliberately separate from the risk classifier. The classifier answers "how
//! dangerous is this"; this module answers "what does this installation's policy do about that
//! danger, here". A caller that skips it still gets the old approval behaviour only if it also
//! skips the gate — which is the thing the core is being changed to stop doing.

use std::fmt;

use serde::{Deserialize, Serialize};
use sevenshell_types::{PolicyAction, PolicyMatrix, PolicyRule, RiskAssessment};

/// Where one decision looked: the session's group, host and the directory an action names.
///
/// Every field is optional because an action may not have that dimension: a container action has a
/// connection and no remote directory, a batch has hosts and no directory. A missing field simply
/// means that layer cannot match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PolicyTarget<'a> {
    pub group_id: Option<&'a str>,
    pub host: Option<&'a str>,
    pub port: Option<u16>,
    pub directory: Option<&'a str>,
}

/// The layer that supplied the winning action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicySource {
    Global,
    Group(String),
    Host(String),
    Directory(String),
}

impl fmt::Display for PolicySource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Global => formatter.write_str("global"),
            Self::Group(id) => write!(formatter, "group:{id}"),
            Self::Host(key) => write!(formatter, "host:{key}"),
            Self::Directory(path) => write!(formatter, "directory:{path}"),
        }
    }
}

/// One action's verdict, with the rule layer that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    pub action: PolicyAction,
    pub source: PolicySource,
    pub risk: RiskAssessment,
}

impl PolicyDecision {
    pub fn needs_approval(&self) -> bool {
        self.action == PolicyAction::Ask
    }

    pub fn denied(&self) -> bool {
        self.action == PolicyAction::Deny
    }

    /// The line an audit trail keeps about this decision.
    pub fn audit_event(
        &self,
        at: u64,
        action: impl Into<String>,
        subject: Option<String>,
        session_id: Option<String>,
        target: PolicyTarget<'_>,
    ) -> PolicyAuditEvent {
        PolicyAuditEvent {
            at,
            action: action.into(),
            risk: self.risk.clone(),
            decision: self.action,
            source: self.source.to_string(),
            subject,
            session_id,
            group_id: target.group_id.map(str::to_owned),
            host: target.host.map(str::to_owned),
            directory: target.directory.map(str::to_owned),
        }
    }
}

/// One policy decision, exactly as the JSONL trail carries it.
///
/// Deserializable as well as serializable: the trail is read back into the same type the core wrote,
/// so the interface can never be shown a second shape that disagrees with what was recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyAuditEvent {
    /// Seconds since the Unix epoch, the same clock the rest of the application records.
    pub at: u64,
    /// The operation, not the command line: an audit line must not become a copy of a tool's input.
    pub action: String,
    pub risk: RiskAssessment,
    pub decision: PolicyAction,
    /// `global`, `group:<id>`, `host:<host[:port]>` or `directory:<path>`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

/// Resolves the most restrictive decision among the layers that apply.
pub fn decide(matrix: &PolicyMatrix, target: PolicyTarget<'_>, risk: &RiskAssessment) -> PolicyDecision {
    let level = risk.level;
    let mut action = matrix.global.action_for(level);
    let mut source = PolicySource::Global;

    if let Some(group) = target.group_id
        && let Some(rule) = matrix.groups.get(group)
    {
        (action, source) = tighten(
            action,
            source,
            rule.action_for(level),
            PolicySource::Group(group.to_owned()),
        );
    }

    if let Some(host) = target.host {
        // A rule may name the host alone or one port on it. Both can exist; both apply.
        if let Some(port) = target.port
            && let Some(rule) = matrix.hosts.get(&format!("{host}:{port}"))
        {
            (action, source) = tighten(
                action,
                source,
                rule.action_for(level),
                PolicySource::Host(format!("{host}:{port}")),
            );
        }
        if let Some(rule) = matrix.hosts.get(host) {
            (action, source) = tighten(
                action,
                source,
                rule.action_for(level),
                PolicySource::Host(host.to_owned()),
            );
        }
    }

    if let Some(directory) = target.directory {
        // Every prefix applies; the most restrictive wins. The recorded source is the longest
        // matching prefix that supplied that winning action.
        let mut matching: Vec<(&String, &PolicyRule)> = matrix
            .directories
            .iter()
            .filter(|(prefix, _)| directory_contains(prefix, directory))
            .collect();
        matching.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
        for (prefix, rule) in matching {
            (action, source) = tighten(
                action,
                source,
                rule.action_for(level),
                PolicySource::Directory((*prefix).clone()),
            );
        }
    }

    // A destructive action can be made stricter by any layer, but never looser than approval: the
    // matrix decides where and what to ask about, not whether a destructive action gets a person at
    // all. That is the red line every build has held.
    if level.needs_approval() && action == PolicyAction::Allow {
        action = PolicyAction::Ask;
        source = PolicySource::Global;
    }

    PolicyDecision {
        action,
        source,
        risk: risk.clone(),
    }
}

/// Takes a candidate only when it is at least as strict; equal strictness prefers the more specific
/// layer because that is the rule a person will look for when reading the audit line.
fn tighten(
    current: PolicyAction,
    current_source: PolicySource,
    candidate: PolicyAction,
    candidate_source: PolicySource,
) -> (PolicyAction, PolicySource) {
    let candidate_wins =
        candidate > current || (candidate == current && specificity(&candidate_source) > specificity(&current_source));
    if candidate_wins {
        (candidate, candidate_source)
    } else {
        (current, current_source)
    }
}

fn specificity(source: &PolicySource) -> u8 {
    match source {
        PolicySource::Global => 0,
        PolicySource::Group(_) => 1,
        PolicySource::Host(_) => 2,
        PolicySource::Directory(_) => 3,
    }
}

/// Whether a configured prefix covers a path. `/` covers everything, and a prefix only matches on a
/// path boundary so `/srv` does not cover `/srv-old`.
fn directory_contains(prefix: &str, path: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return true;
    }
    path.trim_end_matches('/') == prefix || path.starts_with(&format!("{prefix}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_types::{RiskLevel, RiskReason};

    fn risk(level: RiskLevel) -> RiskAssessment {
        RiskAssessment {
            level,
            reasons: Vec::<RiskReason>::new(),
        }
    }

    fn asking() -> PolicyRule {
        PolicyRule {
            safe: PolicyAction::Ask,
            caution: PolicyAction::Ask,
            destructive: PolicyAction::Ask,
            catastrophic: PolicyAction::Ask,
        }
    }

    #[test]
    fn a_layer_can_tighten_but_never_loosen_the_global_rule() {
        let matrix = PolicyMatrix {
            global: PolicyRule::default(),
            groups: [(
                "lab".to_owned(),
                PolicyRule {
                    safe: PolicyAction::Allow,
                    caution: PolicyAction::Allow,
                    destructive: PolicyAction::Allow,
                    catastrophic: PolicyAction::Allow,
                },
            )]
            .into(),
            ..PolicyMatrix::default()
        };
        let decision = decide(
            &matrix,
            PolicyTarget {
                group_id: Some("lab"),
                ..PolicyTarget::default()
            },
            &risk(RiskLevel::Destructive),
        );
        assert_eq!(
            decision.action,
            PolicyAction::Ask,
            "a group cannot make destructive safe"
        );
        assert_eq!(decision.source, PolicySource::Global);
    }

    #[test]
    fn the_most_specific_layer_is_named_when_it_tightens() {
        let matrix = PolicyMatrix {
            hosts: [("web".to_owned(), asking())].into(),
            ..PolicyMatrix::default()
        };
        let decision = decide(
            &matrix,
            PolicyTarget {
                host: Some("web"),
                ..PolicyTarget::default()
            },
            &risk(RiskLevel::Caution),
        );
        assert_eq!(decision.action, PolicyAction::Ask);
        assert_eq!(decision.source, PolicySource::Host("web".to_owned()));
    }

    #[test]
    fn host_port_rules_are_more_specific_than_host_rules() {
        let matrix = PolicyMatrix {
            hosts: [
                ("web".to_owned(), PolicyRule::default()),
                ("web:2222".to_owned(), asking()),
            ]
            .into(),
            ..PolicyMatrix::default()
        };
        let target = PolicyTarget {
            host: Some("web"),
            port: Some(2222),
            ..PolicyTarget::default()
        };
        let decision = decide(&matrix, target, &risk(RiskLevel::Safe));
        assert_eq!(decision.action, PolicyAction::Ask);
        assert_eq!(decision.source, PolicySource::Host("web:2222".to_owned()));
    }

    #[test]
    fn directory_prefixes_match_on_a_boundary_and_the_longest_wins() {
        let matrix = PolicyMatrix {
            directories: [
                ("/srv".to_owned(), asking()),
                ("/srv/old".to_owned(), PolicyRule::default()),
            ]
            .into(),
            ..PolicyMatrix::default()
        };
        let decision = decide(
            &matrix,
            PolicyTarget {
                directory: Some("/srv/old/data"),
                ..PolicyTarget::default()
            },
            &risk(RiskLevel::Safe),
        );
        assert_eq!(decision.action, PolicyAction::Ask, "the /srv rule still applies");

        let outside = decide(
            &matrix,
            PolicyTarget {
                directory: Some("/srv-old/data"),
                ..PolicyTarget::default()
            },
            &risk(RiskLevel::Safe),
        );
        assert_eq!(outside.action, PolicyAction::Allow, "/srv does not cover /srv-old");
    }

    #[test]
    fn deny_at_any_matching_layer_wins() {
        let matrix = PolicyMatrix {
            global: PolicyRule::default(),
            directories: [(
                "/prod".to_owned(),
                PolicyRule {
                    safe: PolicyAction::Deny,
                    caution: PolicyAction::Deny,
                    destructive: PolicyAction::Deny,
                    catastrophic: PolicyAction::Deny,
                },
            )]
            .into(),
            ..PolicyMatrix::default()
        };
        let decision = decide(
            &matrix,
            PolicyTarget {
                directory: Some("/prod/releases"),
                ..PolicyTarget::default()
            },
            &risk(RiskLevel::Safe),
        );
        assert!(decision.denied());
        assert_eq!(decision.source, PolicySource::Directory("/prod".to_owned()));
    }

    #[test]
    fn an_audit_event_names_the_layer_and_never_the_rule_input() {
        let decision = decide(
            &PolicyMatrix::default(),
            PolicyTarget::default(),
            &risk(RiskLevel::Destructive),
        );
        let event = decision.audit_event(
            42,
            "sshell_batch_exec",
            Some("a command".to_owned()),
            None,
            PolicyTarget::default(),
        );
        assert_eq!(event.decision, PolicyAction::Ask);
        assert_eq!(event.source, "global");
        assert_eq!(event.at, 42);
    }
}

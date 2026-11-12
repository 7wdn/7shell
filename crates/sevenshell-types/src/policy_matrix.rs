//! The four-layer policy matrix behind invariant 7.
//!
//! A rule per risk level, and four layers that can only make a decision stricter: global, group,
//! host and directory. "Most restrictive wins" is the whole inheritance rule; a group cannot make a
//! destructive command safer than the global default. The terminal is deliberately outside this
//! matrix — a person typing a command is the decision — while unattended batch, container, cluster
//! and tool actions are inside it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::risk::RiskLevel;

/// What a rule says about one risk level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyAction {
    #[default]
    Allow,
    /// Stop for a person before anything runs.
    Ask,
    /// Refuse before anything runs; asking cannot override it.
    Deny,
}

/// One rule, by the classifier's four levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRule {
    pub safe: PolicyAction,
    pub caution: PolicyAction,
    pub destructive: PolicyAction,
    pub catastrophic: PolicyAction,
}

impl Default for PolicyRule {
    fn default() -> Self {
        // The behaviour every build had before the matrix existed: safe and caution run, the two
        // destructive levels stop for a person. A layer can tighten this; nothing can loosen it.
        Self {
            safe: PolicyAction::Allow,
            caution: PolicyAction::Allow,
            destructive: PolicyAction::Ask,
            catastrophic: PolicyAction::Ask,
        }
    }
}

impl PolicyRule {
    pub const fn action_for(self, level: RiskLevel) -> PolicyAction {
        match level {
            RiskLevel::Safe => self.safe,
            RiskLevel::Caution => self.caution,
            RiskLevel::Destructive => self.destructive,
            RiskLevel::Catastrophic => self.catastrophic,
        }
    }
}

/// The rules an installation has written, by layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyMatrix {
    #[serde(default)]
    pub global: PolicyRule,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, PolicyRule>,
    /// Keys are a host, or `host:port` for one port only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosts: BTreeMap<String, PolicyRule>,
    /// Keys are remote path prefixes; `/` matches everything.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub directories: BTreeMap<String, PolicyRule>,
}

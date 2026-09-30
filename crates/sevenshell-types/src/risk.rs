//! How dangerous a command is, as the interface and the policy crate both see it.
//!
//! The vocabulary lives here, next to the other wire types, because three sides have to agree on
//! it: the classifier that produces a verdict, the layer that decides whether a command may run
//! unattended, and the interface that explains the verdict to a person. The *rules* — which
//! command shape means what — are in `sevenshell-policy`, together with the corpus that locks
//! them; this module is only the answer's shape.

use std::fmt;

use serde::{Deserialize, Serialize};

/// How much damage a command could do.
///
/// Ordered: a caller compares levels to decide whether to warn, to require an unlock window, or to
/// refuse outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Nothing here is destructive.
    Safe,
    /// Worth a look: it changes something, elevates privileges, or cannot be read confidently.
    Caution,
    /// It destroys data or takes a service down, on purpose.
    Destructive,
    /// It can destroy a machine or a fleet: a filesystem, a device, or code from the network
    /// running as root.
    Catastrophic,
}

impl RiskLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Caution => "caution",
            Self::Destructive => "destructive",
            Self::Catastrophic => "catastrophic",
        }
    }

    /// Whether this level should stop an unattended action and wait for a person.
    pub const fn needs_approval(self) -> bool {
        matches!(self, Self::Destructive | Self::Catastrophic)
    }
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One rule that fired, and the message key that explains it.
///
/// `label_key` is a key into the interface's translations (`risk.*`), not a sentence: the same
/// verdict is shown in every language the application speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskReason {
    /// Stable identifier, so a person can quote the rule they disagree with.
    pub rule_id: String,
    pub label_key: String,
    pub level: RiskLevel,
}

impl RiskReason {
    pub fn new(rule_id: &str, label_key: &str, level: RiskLevel) -> Self {
        Self {
            rule_id: rule_id.to_owned(),
            label_key: label_key.to_owned(),
            level,
        }
    }
}

/// What the classifier concluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskAssessment {
    /// The worst level any fired rule reached.
    pub level: RiskLevel,
    pub reasons: Vec<RiskReason>,
}

impl RiskAssessment {
    /// The verdict for a set of fired rules: the level is the worst of them.
    pub fn from_reasons(reasons: Vec<RiskReason>) -> Self {
        let level = reasons
            .iter()
            .map(|reason| reason.level)
            .max()
            .unwrap_or(RiskLevel::Safe);
        Self { level, reasons }
    }

    /// The verdict for a command nobody looked at.
    pub fn safe() -> Self {
        Self {
            level: RiskLevel::Safe,
            reasons: Vec::new(),
        }
    }

    /// Whether an unattended caller should stop and ask.
    pub fn needs_approval(&self) -> bool {
        self.level.needs_approval()
    }
}

//! Who asked for an operation.
//!
//! The MCP bridge and the in-application assistant share one tool layer, one approval window and one
//! audit trail. What tells the two apart is this value: the window can say who is asking, and the
//! audit line can say who asked. It is also why a decision recorded as `mcp` keeps meaning MCP even
//! after a second caller exists.
//!
//! Absent reads as [`AgentOrigin::Mcp`]: every request and every audit line written before the
//! assistant existed was an MCP one, and a file that predates this distinction must not be read as
//! something else.

use serde::{Deserialize, Serialize};

/// Who an operation belongs to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentOrigin {
    /// A client driving the tools over the loopback endpoint.
    #[default]
    Mcp,
    /// The assistant panel inside this application.
    Ai,
}

impl AgentOrigin {
    pub const ALL: &'static [AgentOrigin] = &[AgentOrigin::Mcp, AgentOrigin::Ai];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Ai => "ai",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|origin| origin.as_str() == id)
    }
}

impl std::fmt::Display for AgentOrigin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_origin_round_trips_through_its_own_name() {
        for origin in AgentOrigin::ALL {
            assert_eq!(
                AgentOrigin::from_id(origin.as_str()),
                Some(*origin),
                "an origin whose name does not read back is one a stored decision cannot hold"
            );
        }
    }

    #[test]
    fn an_origin_nobody_named_is_mcp() {
        assert_eq!(
            AgentOrigin::default(),
            AgentOrigin::Mcp,
            "an audit line written before the assistant existed was an MCP call"
        );
    }
}

//! Which sessions an agent may see, and what it may do on each one.
//!
//! A snapshot rather than the settings file itself: the settings carry groups and an allowlist, and
//! the host is the side that knows which saved session belongs to which group. By the time a tool
//! sees this value, a session id has one scope and one visibility answer, and the tool never has to
//! ask a second store whether it is allowed to proceed.
//!
//! The global scope stays the default. Session overrides win, then group overrides (already folded
//! onto sessions by the host), then the default. `max_scope` exists only for the registry's early
//! refusal; the per-session answer is what a tool enforces.

use std::collections::{BTreeMap, BTreeSet};

use sevenshell_types::McpScope;

/// A resolved view of one installation's MCP access policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpAccess {
    pub default_scope: McpScope,
    /// `None` means every saved session is visible; `Some` is the exact allowlist.
    pub visible_sessions: Option<BTreeSet<String>>,
    /// Effects of group and session overrides, already resolved to session ids.
    pub session_scopes: BTreeMap<String, McpScope>,
}

impl McpAccess {
    /// The open policy an installation has before it has configured anything.
    pub fn open(scope: McpScope) -> Self {
        Self {
            default_scope: scope,
            visible_sessions: None,
            session_scopes: BTreeMap::new(),
        }
    }

    pub fn visible(&self, session_id: &str) -> bool {
        self.visible_sessions
            .as_ref()
            .is_none_or(|sessions| sessions.contains(session_id))
    }

    pub fn scope_for(&self, session_id: &str) -> McpScope {
        self.session_scopes
            .get(session_id)
            .copied()
            .unwrap_or(self.default_scope)
    }

    /// The widest scope any session may have.
    ///
    /// The registry refuses a write tool outright when even this cannot allow it; a narrower
    /// session is refused by the tool itself, after it knows which session the call names.
    pub fn max_scope(&self) -> McpScope {
        self.session_scopes
            .values()
            .copied()
            .fold(self.default_scope, strictest_wide)
    }

    /// Every visible session id, in the order the caller supplied them.
    pub fn filter_visible<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        ids.into_iter()
            .filter(|id| self.visible(id))
            .map(str::to_owned)
            .collect()
    }
}

fn strictest_wide(left: McpScope, right: McpScope) -> McpScope {
    if rank(right) > rank(left) { right } else { left }
}

fn rank(scope: McpScope) -> u8 {
    match scope {
        McpScope::ReadOnly => 0,
        McpScope::SafeWrite => 1,
        McpScope::HighRisk => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access() -> McpAccess {
        McpAccess {
            default_scope: McpScope::ReadOnly,
            visible_sessions: Some(BTreeSet::from(["one".to_owned(), "two".to_owned()])),
            session_scopes: BTreeMap::from([("two".to_owned(), McpScope::SafeWrite)]),
        }
    }

    #[test]
    fn a_session_override_wins_over_the_global_default() {
        let access = access();
        assert_eq!(access.scope_for("one"), McpScope::ReadOnly);
        assert_eq!(access.scope_for("two"), McpScope::SafeWrite);
        assert_eq!(
            access.max_scope(),
            McpScope::SafeWrite,
            "the early gate must not be narrower"
        );
    }

    #[test]
    fn hidden_is_not_the_same_as_absent() {
        let access = access();
        assert!(access.visible("one"));
        assert!(!access.visible("three"));
        assert_eq!(access.filter_visible(["three", "one"]), vec!["one".to_owned()]);
    }

    #[test]
    fn an_unconfigured_allowlist_shows_everything() {
        let access = McpAccess::open(McpScope::ReadOnly);
        assert!(access.visible("anything"));
    }
}

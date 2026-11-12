//! How the settings page groups the tools it can switch on and off.
//!
//! This is presentation metadata, not a second registry: a tool's name, schema and gate still live
//! in one place. The list exists so a window can say "these two are sessions" without keying that
//! knowledge off a prefix, and so a test can refuse a new tool that nobody has categorised.

use serde::Serialize;

/// The heading a tool is drawn under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogCategory {
    Sessions,
    Exec,
    Files,
    Containers,
    Kubernetes,
    External,
}

/// How a person should read the tool's reach before switching it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolRisk {
    /// It only looks.
    Read,
    /// It changes one named thing.
    Write,
    /// What it does depends on the command or action it is given, which the policy classifier
    /// grades per call.
    CommandGraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: &'static str,
    pub category: CatalogCategory,
    pub risk: ToolRisk,
}

/// Every built-in tool, in the order the settings page lists them.
pub const BUILTIN_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "sshell_sessions_list",
        category: CatalogCategory::Sessions,
        risk: ToolRisk::Read,
    },
    CatalogEntry {
        name: "sshell_session_open",
        category: CatalogCategory::Sessions,
        risk: ToolRisk::Write,
    },
    CatalogEntry {
        name: "sshell_exec_run",
        category: CatalogCategory::Exec,
        risk: ToolRisk::CommandGraded,
    },
    CatalogEntry {
        name: "sshell_batch_exec",
        category: CatalogCategory::Exec,
        risk: ToolRisk::CommandGraded,
    },
    CatalogEntry {
        name: "sshell_sftp_list",
        category: CatalogCategory::Files,
        risk: ToolRisk::Read,
    },
    CatalogEntry {
        name: "sshell_sftp_read_text",
        category: CatalogCategory::Files,
        risk: ToolRisk::Read,
    },
    CatalogEntry {
        name: "sshell_sftp_write_text",
        category: CatalogCategory::Files,
        risk: ToolRisk::Write,
    },
    CatalogEntry {
        name: "sshell_containers_list",
        category: CatalogCategory::Containers,
        risk: ToolRisk::Read,
    },
    CatalogEntry {
        name: "sshell_container_action",
        category: CatalogCategory::Containers,
        risk: ToolRisk::Write,
    },
    CatalogEntry {
        name: "sshell_k8s_get",
        category: CatalogCategory::Kubernetes,
        risk: ToolRisk::Read,
    },
];

pub fn catalog_entry(name: &str) -> Option<&'static CatalogEntry> {
    BUILTIN_CATALOG.iter().find(|entry| entry.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_tool_has_exactly_one_catalogue_entry() {
        let mut names: Vec<&str> = BUILTIN_CATALOG.iter().map(|entry| entry.name).collect();
        names.sort_unstable();
        let mut unique = names.clone();
        unique.dedup();
        assert_eq!(names, unique, "a name may not appear twice");
        assert_eq!(
            BUILTIN_CATALOG.len(),
            10,
            "the settings page expects the first wave's ten tools"
        );
    }

    #[test]
    fn a_name_that_is_not_a_builtin_has_no_entry() {
        assert!(catalog_entry("sshell_not_a_tool").is_none());
        assert!(catalog_entry("mcp__files__read").is_none());
    }

    #[test]
    fn the_catalogue_has_exactly_one_entry_per_registered_tool() {
        // Both directions in one comparison: a registered tool with no entry would be invisible to
        // the settings page, and an entry for nothing would be a switch that controls no tool.
        let mut registered: Vec<String> = crate::local::default_registry()
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.name)
            .collect();
        registered.sort_unstable();

        let mut catalogued: Vec<&str> = BUILTIN_CATALOG.iter().map(|entry| entry.name).collect();
        catalogued.sort_unstable();

        assert_eq!(
            catalogued,
            registered.iter().map(String::as_str).collect::<Vec<_>>(),
            "the settings page lists exactly the tools this build registers, once each"
        );
    }
}

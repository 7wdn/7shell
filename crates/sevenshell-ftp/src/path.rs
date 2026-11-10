//! Remote path hygiene.
//!
//! FTP has no session root of its own: every path a server accepts is either absolute in whatever
//! file system it exposes, or relative to a working directory nobody in this crate asked for. A
//! file browser still has to behave like a browser, so every path a caller names is normalized into
//! one vocabulary — a leading `/`, no `.` or `..`, no doubled separators — and a path that would
//! step above the login directory is refused rather than clamped. Clamping would silently act on a
//! different location than the one that was asked for, which is worse than an error.
//!
//! CR, LF and NUL are refused in every argument. FTP carries commands and their arguments on the
//! control connection as lines, so a line break inside a path is not a strange file name — it is a
//! second command. The client below already guards this, but it is the kind of guard that has to be
//! visible where the value is built, not only inside a dependency.

/// Why a remote path cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathProblem {
    /// The path carries a character that would end the control-connection line.
    ControlCharacter,
    /// The path has more `..` segments than it has parents.
    OutsideRoot,
}

impl PathProblem {
    /// The sentence that goes into the error envelope. Deliberately static: a refused path may be
    /// anything, and the detail should not repeat it back.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::ControlCharacter => "the path contains a line break or NUL",
            Self::OutsideRoot => "the path leaves the session root",
        }
    }
}

/// Whether a value meant for one command line is safe to put there.
pub(crate) fn carries_control_character(value: &str) -> bool {
    value.chars().any(|character| matches!(character, '\r' | '\n' | '\0'))
}

/// Collapses a caller's path into the session's own form: `/` or `/a/b`.
///
/// Absolute and relative spellings mean the same thing here, because everything is resolved
/// against the directory the login landed in. A leading `/` is therefore not the server's file
/// system root but the session root; the caller never has to know which of the two the server
/// would have used.
pub(crate) fn normalize(path: &str) -> Result<String, PathProblem> {
    if carries_control_character(path) {
        return Err(PathProblem::ControlCharacter);
    }

    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                // Popping the last component is correct only while there is one; an empty stack
                // means this `..` would leave the session root.
                if parts.pop().is_none() {
                    return Err(PathProblem::OutsideRoot);
                }
            }
            component => parts.push(component),
        }
    }

    if parts.is_empty() {
        Ok("/".to_owned())
    } else {
        Ok(format!("/{}", parts.join("/")))
    }
}

/// The parent of an already normalized session path. The parent of the root is the root, which is
/// what makes `stat("/")` and a listing of `/` agree about where they are.
pub(crate) fn parent(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(index) => path[..index].to_owned(),
    }
}

/// The last component of an already normalized session path, empty for the root.
pub(crate) fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or_default()
}

/// Joins a directory listing's name onto the directory it came from.
///
/// Returns `None` when the name cannot be a single component: the entry is then left out of the
/// listing rather than turned into a path that would address something else. A server that sends a
/// name with a slash in it is not describing a child, and a name that is `.` or `..` is the
/// directory itself or its parent, neither of which belongs in a file panel.
pub(crate) fn child(parent: &str, name: &str) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." || carries_control_character(name) || name.contains('/') {
        return None;
    }

    if parent == "/" {
        Some(format!("/{name}"))
    } else {
        Some(format!("{parent}/{name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_segment_cannot_walk_above_the_session_root() {
        assert_eq!(normalize("../etc"), Err(PathProblem::OutsideRoot));
        assert_eq!(normalize("/a/../../b"), Err(PathProblem::OutsideRoot));
    }

    #[test]
    fn dot_segments_and_doubled_separators_collapse() {
        assert_eq!(normalize("/a/./b//c/").as_deref(), Ok("/a/b/c"));
        assert_eq!(normalize("a/b/../c").as_deref(), Ok("/a/c"));
        assert_eq!(normalize("./").as_deref(), Ok("/"));
        assert_eq!(normalize("").as_deref(), Ok("/"));
    }

    #[test]
    fn a_line_break_in_a_path_is_refused() {
        assert_eq!(normalize("docs/evil\r\nDELE x"), Err(PathProblem::ControlCharacter));
        assert_eq!(normalize("docs\nstory"), Err(PathProblem::ControlCharacter));
    }

    #[test]
    fn a_nul_in_a_path_is_refused() {
        assert_eq!(normalize("docs\0name"), Err(PathProblem::ControlCharacter));
    }

    #[test]
    fn the_parent_of_the_root_is_the_root() {
        assert_eq!(parent("/"), "/");
        assert_eq!(parent("/a"), "/");
        assert_eq!(parent("/a/b"), "/a");
    }

    #[test]
    fn a_listed_name_becomes_exactly_one_child() {
        assert_eq!(child("/", "hello.txt").as_deref(), Some("/hello.txt"));
        assert_eq!(child("/docs", "hello.txt").as_deref(), Some("/docs/hello.txt"));
        assert!(child("/docs", "..").is_none());
        assert!(child("/docs", ".").is_none());
        assert!(child("/docs", "").is_none());
        assert!(child("/docs", "a/b").is_none());
        assert!(child("/docs", "a\r\nDELE b").is_none());
    }

    #[test]
    fn the_leaf_of_a_normalized_path_is_its_name() {
        assert_eq!(leaf("/a/b.txt"), "b.txt");
        assert_eq!(leaf("/"), "");
    }
}

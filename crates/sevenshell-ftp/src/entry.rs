//! A directory as entries, and the two dialects a server may describe it in.
//!
//! `MLSD` (RFC 3659) is the only listing with a defined shape, so it is the one this crate asks
//! for first. `LIST` has no specification at all — its output is whatever `ls -l` the server felt
//! like running — so it is the fallback, parsed for the two families that cover nearly every
//! server in service: the Unix `-rw-r--r-- …` line and the Windows `04-20-21 03:40PM <DIR>` line.
//! A server that speaks neither is reported as such instead of being shown as an empty folder.
//!
//! Two fields are deliberately weaker than they look:
//!
//! * **A symlink is an entry, not its target.** `LIST` names it, and following it here would mean
//!   a second round trip per row.
//! * **A missing modification time is `None`, not 1970.** `MLSD` may omit `modify`; the parser
//!   reports that as the epoch, which no file panel should render as a date.

use std::time::{SystemTime, UNIX_EPOCH};

use suppaftp::list::{File as Listed, ListParser};

use crate::path;

/// One entry of a remote directory, in the shape the file browser draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpEntry {
    /// The name as the server spelled it, which is what a rename or a delete will address.
    pub name: String,
    /// The session path of the entry: always starts at `/`, and `/` is the directory the login
    /// landed in.
    pub path: String,
    pub is_dir: bool,
    /// Bytes, as the server reported them. Directories are reported as `0` because `LIST` and
    /// `MLSD` disagree about whether their size means anything.
    pub size: u64,
    /// Seconds since the Unix epoch, UTC. `None` when the server did not say.
    pub modified: Option<i64>,
}

/// Why a listing could not be turned into entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListingError {
    /// The server sent data, and not one line of it was in a format this crate reads.
    Unreadable,
}

/// Entries from an `MLSD` body. A line that does not parse is skipped: one server quirk should not
/// hide the rest of the directory. A body that contains data and yields nothing understandable is
/// an error, because an empty panel would look like an empty directory.
pub(crate) fn from_mlsd(parent: &str, lines: &[String]) -> Result<Vec<FtpEntry>, ListingError> {
    let mut entries = Vec::new();
    let mut understood = false;

    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // RFC 3659 lets a server omit the facts for `.` and `..`; those parse as entries named
        // `cdir`/`pdir` and are dropped by `from_listed` since neither name is a child.
        if let Ok(listed) = ListParser::parse_mlsd(line) {
            understood = true;
            if let Some(entry) = from_listed(parent, listed) {
                entries.push(entry);
            }
        }
    }

    if !understood && lines.iter().any(|line| !line.trim().is_empty()) {
        return Err(ListingError::Unreadable);
    }
    Ok(entries)
}

/// Entries from a `LIST` body, in either of the two dialects described in the module docs.
pub(crate) fn from_list(parent: &str, lines: &[String]) -> Result<Vec<FtpEntry>, ListingError> {
    let mut entries = Vec::new();
    let mut candidates = 0;
    let mut understood = false;

    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        // `ls -l` prints a block count before the rows. It is not an entry and some servers do
        // print it, so it is skipped without counting as something that failed to parse.
        if line.starts_with("total ") {
            continue;
        }

        candidates += 1;
        let listed = ListParser::parse_posix(line).or_else(|_| ListParser::parse_dos(line));
        if let Ok(listed) = listed {
            understood = true;
            if let Some(entry) = from_listed(parent, listed) {
                entries.push(entry);
            }
        }
    }

    if candidates > 0 && !understood {
        return Err(ListingError::Unreadable);
    }
    Ok(entries)
}

/// One `MLST` reply line, which is a single `MLSD` entry with facts.
///
/// The name is taken from the path that was asked for, not from the reply: a server may echo the
/// pathname it was given (an absolute one) or just the last component, and both describe the same
/// entry. The facts are the part only the server knows.
pub(crate) fn from_mlst(path: &str, line: &str) -> Result<FtpEntry, ListingError> {
    let listed = ListParser::parse_mlst(line.trim_end_matches('\r')).map_err(|_| ListingError::Unreadable)?;
    let name = if path == "/" { "/" } else { path::leaf(path) };
    if name.is_empty() {
        return Err(ListingError::Unreadable);
    }
    Ok(FtpEntry {
        name: name.to_owned(),
        path: path.to_owned(),
        is_dir: listed.is_directory(),
        size: u64::try_from(listed.size()).unwrap_or(u64::MAX),
        modified: modified_seconds(listed.modified()),
    })
}

fn from_listed(parent: &str, listed: Listed) -> Option<FtpEntry> {
    let name = listed.name().to_owned();
    let entry_path = path::child(parent, &name)?;
    Some(FtpEntry {
        name,
        path: entry_path,
        is_dir: listed.is_directory(),
        size: u64::try_from(listed.size()).unwrap_or(u64::MAX),
        modified: modified_seconds(listed.modified()),
    })
}

fn modified_seconds(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        // The parser uses the epoch for "no `modify` fact", so a zero here means unknown rather
        // than a file from 1970.
        Ok(elapsed) if !elapsed.is_zero() => i64::try_from(elapsed.as_secs()).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_mlsd_line_carries_type_size_and_time() {
        let lines = vec!["type=file;size=5;modify=20240102030405; hello.txt".to_owned()];
        let entries = from_mlsd("/", &lines).expect("listing parses");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.name, "hello.txt");
        assert_eq!(entry.path, "/hello.txt");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 5);
        assert_eq!(entry.modified, Some(1_704_164_645));
    }

    #[test]
    fn the_directory_itself_and_its_parent_are_not_listed_as_children() {
        let lines = vec![
            "type=cdir;modify=20240102030405; .".to_owned(),
            "type=pdir;modify=20240102030405; ..".to_owned(),
            "type=dir;modify=20240102030405; docs".to_owned(),
        ];
        let entries = from_mlsd("/", &lines).expect("listing parses");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "docs");
        assert!(entries[0].is_dir);
    }

    #[test]
    fn an_mlsd_line_without_a_modify_fact_reports_no_time() {
        let lines = vec!["type=file;size=12; note.txt".to_owned()];
        let entries = from_mlsd("/", &lines).expect("listing parses");
        assert_eq!(entries[0].modified, None);
    }

    #[test]
    fn an_mlsd_body_of_junk_is_an_error_instead_of_an_empty_directory() {
        // A line with no facts at all is legal in MLSD (the name alone); a *type* the protocol
        // does not define is not.
        let lines = vec!["type=teleport; odd".to_owned()];
        assert_eq!(from_mlsd("/", &lines), Err(ListingError::Unreadable));
    }

    #[test]
    fn an_empty_mlsd_body_is_an_empty_directory() {
        let lines: Vec<String> = Vec::new();
        assert_eq!(from_mlsd("/", &lines), Ok(Vec::new()));
    }

    #[test]
    fn a_unix_list_line_is_read() {
        let lines = vec![
            "total 4".to_owned(),
            "-rw-r--r-- 1 alice alice 12 Nov  5 13:46 hello.txt".to_owned(),
            "drwxr-xr-x 2 alice alice 4096 Nov  5 13:46 docs".to_owned(),
        ];
        let entries = from_list("/srv", &lines).expect("listing parses");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "hello.txt");
        assert_eq!(entries[0].path, "/srv/hello.txt");
        assert_eq!(entries[0].size, 12);
        assert!(!entries[0].is_dir);
        assert!(entries[1].is_dir);
        assert_eq!(entries[1].path, "/srv/docs");
    }

    #[test]
    fn a_windows_list_line_is_read() {
        let lines = vec![
            "04-20-21  03:40PM       <DIR>          docs".to_owned(),
            "04-20-21  03:40PM                 1234 hello.txt".to_owned(),
        ];
        let entries = from_list("/", &lines).expect("listing parses");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "docs");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "hello.txt");
        assert_eq!(entries[1].size, 1234);
        assert!(entries[1].modified.is_some());
    }

    #[test]
    fn a_list_body_of_junk_is_an_error_instead_of_an_empty_directory() {
        let lines = vec!["one\ntwo".to_owned()];
        assert_eq!(from_list("/", &lines), Err(ListingError::Unreadable));
    }

    #[test]
    fn a_listed_name_with_a_separator_is_left_out() {
        let lines = vec!["-rw-r--r-- 1 alice alice 1 Nov  5 13:46 ../escape".to_owned()];
        let entries = from_list("/", &lines).expect("listing parses");
        assert!(entries.is_empty());
    }

    #[test]
    fn a_single_mlst_line_becomes_an_entry_beside_its_parent() {
        let entry = from_mlst(
            "/docs/report.txt",
            " type=file;size=7;modify=20240102030405; /docs/report.txt",
        )
        .expect("MLST line parses");
        assert_eq!(entry.name, "report.txt");
        assert_eq!(entry.path, "/docs/report.txt");
        assert_eq!(entry.size, 7);
    }
}

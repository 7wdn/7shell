//! Listing a remote directory through the shell, for a server that has no SFTP.
//!
//! The file browser speaks SFTP, but a server can be old enough to offer nothing but a login shell,
//! and a browser that cannot list a directory cannot show one at all. This module is the listing a
//! shell can produce: one command, and the reading of its answer. It knows nothing about sessions
//! beyond "run this and give me what it printed", so the caller decides when a shell listing is what
//! is happening.
//!
//! Two shapes are understood, and the command picks between them on the far side:
//!
//! * **GNU `find`** with `-mindepth`, `-maxdepth` and `-printf`: one NUL-terminated record per entry,
//!   tab-separated fields, the name last. Every byte a POSIX name may contain except NUL is
//!   representable — a newline included — because a record ends at NUL and not at a newline. A tab
//!   inside a name is kept too, because the name is read as "everything after the ninth tab". The
//!   one field that cannot carry a tab is a *link target*, which sits before the name and would
//!   shift it; that is the price of the shape that keeps every name.
//! * **`ls -lAH`** in its POSIX long form under `LC_ALL=C`: the fallback for a server whose `find` is
//!   not GNU. It is read from the right — the name is whatever follows the date field — so a name
//!   with spaces, quotes, an apostrophe or Chinese characters survives. A newline in a name does
//!   not: the continuation line cannot be told from a record, so such a listing fails instead of
//!   showing a name with its tail missing. (A crafted continuation line could mimic a record; the
//!   long format simply has no escape for a newline, and GNU `find` is the shape that does.)
//!
//! Both shapes report `lstat`, not `stat`: a symbolic link is described as a link and its target is
//! reported, never followed. Nothing here opens a file. The one link that *is* followed is the
//! directory the caller named, through `-H`: a browser that showed a linked directory as empty would
//! be wrong about a perfectly ordinary home directory, while links *inside* it are still entries.
//!
//! The path is one shell word ([`quote`]), and the script is handed to `/bin/sh` rather than written
//! for whatever shell the account happens to use, so its syntax does not depend on the login shell.
//! A path containing a CR, LF or NUL is refused before a command exists, and a relative path that
//! looks like an option is written as `./…` so neither `find` nor `ls` can read it as one.
//!
//! There is no fallback *decision* here. [`SshSession::probe_shell_capabilities`] reports what the
//! server has — `scp`, a listing command, and which shape the listing command can produce — and the
//! caller decides what that means.

use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::EntryKind;

use crate::SshSession;

/// The line the far side prints before a GNU `find` listing.
const MARKER_FIND: &str = "7shell-listing:find";

/// The line the far side prints before an `ls -lAH` listing.
const MARKER_LS: &str = "7shell-listing:ls";

/// How much output one listing may produce.
///
/// A directory of fifty thousand entries is a few megabytes in the `find` shape. The ceiling exists
/// so a listing of `/` cannot turn into a copy of the file system in memory, and a listing that
/// reaches it is refused rather than shown in part: a directory with entries missing is worse than
/// one that failed to load, because nothing about it says it is incomplete.
const MAX_LISTING_BYTES: usize = 16 * 1024 * 1024;

/// How much output the capability probe may produce.
const MAX_PROBE_BYTES: usize = 64 * 1024;

/// The English month names `ls` prints under `LC_ALL=C`.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Which command produced a listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingFormat {
    /// GNU `find` with `-printf`, one NUL-terminated record per entry.
    Find,
    /// POSIX `ls -lAH`, one line per entry.
    Ls,
}

/// One entry of a directory listed through the shell.
///
/// The fields are the ones [`SftpEntry`](sevenshell_types::SftpEntry) carries, so a caller builds a
/// listing for the panel by joining `name` to the directory it asked about and copying the rest
/// across; `path` is the only field that cannot come from here, because joining is the caller's
/// business. `size` is what `lstat` reports, which for a directory is the file system's own number
/// rather than zero — the same thing the SFTP path passes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub mode: Option<u32>,
    /// The owner's name, or the number it printed when there was no name for it.
    pub owner: Option<String>,
    pub group: Option<String>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// Seconds since the Unix epoch, from `find` only. An `ls -lAH` date is text in the server's own
    /// timezone and, in its time-of-day form, does not even carry a year; turning that into a number
    /// with this machine's clock would be inventing a date, so the fallback leaves the field empty.
    pub modified_at: Option<u64>,
    /// The target a symbolic link names, reported rather than resolved.
    pub link_target: Option<String>,
}

/// What a listing found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellListing {
    /// The path the listing was asked for, not a canonical one: a shell cannot say where a linked
    /// directory really is without another command.
    pub path: String,
    pub entries: Vec<ShellEntry>,
    /// Entries that were there and could not be represented: a name that is not text, or one that
    /// came back as a path. Counted rather than shown wrong, and counted rather than failed,
    /// because one odd name should not hide the rest of a directory.
    pub skipped: u64,
    pub format: ListingFormat,
}

/// What the server has, as far as a shell can tell.
///
/// Facts, not a decision: a caller that finds `scp` and a listing route can offer the SCP-era
/// browser, and a caller that finds neither can say why the server cannot be browsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellCapabilities {
    pub scp: bool,
    pub find: bool,
    /// Whether `find` understands `-mindepth`, `-maxdepth` and `-printf`, which is what the
    /// machine-readable listing needs. BSD and BusyBox `find` do not.
    pub find_printf: bool,
    pub ls: bool,
    /// Whether `ls` accepts `-lA` and `-H`, which is the fallback's invocation.
    pub ls_long: bool,
}

impl ShellCapabilities {
    /// Whether this server can list a directory at all, by either route.
    pub fn can_list(&self) -> bool {
        self.find_printf || self.ls_long
    }

    /// Reads the probe command's answer.
    ///
    /// Every question has to be answered: a missing line means the script did not run the way it
    /// was written, and guessing "no" for the one that went missing would turn a broken probe into a
    /// fact about the server. The error is the reason, in plain words; the caller owns the envelope.
    pub fn parse(stdout: &str) -> Result<Self, String> {
        let mut answers: [Option<bool>; 5] = [None; 5];
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Anything that is not one of the questions is ignored: a login shell may have printed
            // a banner before the script ran, and that is not this script's answer.
            let Some((question, answer)) = line.split_once(' ') else {
                continue;
            };
            let slot = match question {
                "scp" => &mut answers[0],
                "find" => &mut answers[1],
                "find-printf" => &mut answers[2],
                "ls" => &mut answers[3],
                "ls-long" => &mut answers[4],
                _ => continue,
            };
            let answer = match answer.trim() {
                "1" => true,
                "0" => false,
                other => return Err(format!("`{other}` is not a yes or a no for `{question}`")),
            };
            if slot.replace(answer).is_some() {
                return Err(format!("`{question}` was answered twice"));
            }
        }
        Ok(Self {
            scp: answers[0].ok_or("the probe did not say whether scp exists")?,
            find: answers[1].ok_or("the probe did not say whether find exists")?,
            find_printf: answers[2].ok_or("the probe did not say whether find has -printf")?,
            ls: answers[3].ok_or("the probe did not say whether ls exists")?,
            ls_long: answers[4].ok_or("the probe did not say whether ls accepts -lA")?,
        })
    }
}

/// The command that lists `path`, choosing the machine-readable shape when the far side can.
///
/// The capability check is a dry run of the same `find` invocation the listing uses, so a server
/// whose `find` is present but not GNU takes the fallback instead of failing. Both branches print a
/// marker first: the answer is read as "which marker arrived", which is also how a server that ran
/// neither command is told apart from an empty directory.
///
/// `-H` is the one place a link is followed, and it is about the directory that was named rather
/// than about what is inside it: without it, listing a linked directory would show nothing at all.
pub fn list_command(path: &str) -> Result<String, ShellError> {
    let operand = quote(&checked_operand(path)?);
    let script = format!(
        "LC_ALL=C; export LC_ALL; \
         if command -v find >/dev/null 2>&1 \
         && find -H {operand} -mindepth 1 -maxdepth 1 -printf '' >/dev/null 2>&1; then \
         printf '{MARKER_FIND}\\n'; \
         find -H {operand} -mindepth 1 -maxdepth 1 -printf '%y\\t%m\\t%s\\t%u\\t%g\\t%U\\t%G\\t%T@\\t%l\\t%f\\0'; \
         else printf '{MARKER_LS}\\n'; ls -lAH {operand}; fi"
    );
    Ok(format!("/bin/sh -c {}", quote(&script)))
}

/// The command that asks which tools the server has.
///
/// The two listing checks are dry runs of the real invocations, not just `command -v`: a `find`
/// without `-printf` is present and useless here, and an `ls` without `-A` is the same.
pub fn probe_command() -> String {
    let script = "LC_ALL=C; export LC_ALL; \
        for tool in scp find ls; do \
        if command -v \"$tool\" >/dev/null 2>&1; then printf '%s 1\\n' \"$tool\"; \
        else printf '%s 0\\n' \"$tool\"; fi; done; \
        if command -v find >/dev/null 2>&1 \
        && find -H . -mindepth 1 -maxdepth 1 -printf '' >/dev/null 2>&1; \
        then printf 'find-printf 1\\n'; else printf 'find-printf 0\\n'; fi; \
        if command -v ls >/dev/null 2>&1 && ls -lAH . >/dev/null 2>&1; \
        then printf 'ls-long 1\\n'; else printf 'ls-long 0\\n'; fi";
    format!("/bin/sh -c {}", quote(script))
}

impl SshSession {
    /// Lists one directory through the shell.
    ///
    /// The timeout covers the whole command, as [`exec`](SshSession::exec) does. A listing that
    /// would take more output than [`MAX_LISTING_BYTES`] is refused rather than shortened, and a
    /// non-zero status is a failure even when entries were printed: a `find` that could not finish
    /// did not produce a directory that can be shown.
    pub async fn list_via_shell(&self, path: &str, timeout: Duration) -> Result<ShellListing, ShellError> {
        let command = list_command(path)?;
        let captured = self.exec_capture(&command, timeout, MAX_LISTING_BYTES).await?;
        if captured.truncated {
            return Err(ShellError::scp_operation_failed(
                path,
                "the listing produced more output than this will carry, so it was refused rather than shown in part",
            ));
        }
        if captured.exit_code != Some(0) {
            return Err(ShellError::scp_operation_failed(
                path,
                failure_detail("the command that lists a directory", &captured.stderr),
            ));
        }
        parse_listing(path, &captured.stdout)
    }

    /// Asks the server what it has, without deciding anything with the answer.
    pub async fn probe_shell_capabilities(&self, timeout: Duration) -> Result<ShellCapabilities, ShellError> {
        let captured = self.exec_capture(&probe_command(), timeout, MAX_PROBE_BYTES).await?;
        if captured.truncated {
            return Err(ShellError::ssh_channel_failed(
                &self.host,
                self.port,
                "the capability probe printed more than it has questions",
            ));
        }
        if captured.exit_code != Some(0) {
            return Err(ShellError::ssh_channel_failed(
                &self.host,
                self.port,
                failure_detail("the capability probe", &captured.stderr),
            ));
        }
        ShellCapabilities::parse(&captured.stdout).map_err(|reason| {
            ShellError::ssh_channel_failed(
                &self.host,
                self.port,
                format!("the capability probe's answer could not be read: {reason}"),
            )
        })
    }
}

/// Reads one listing answer.
///
/// The marker line says which command ran; the body is read in that shape. Output without a marker
/// is an error and never an empty directory: a shell that could not run the command and a directory
/// with nothing in it must not look the same to a file browser.
pub fn parse_listing(path: &str, stdout: &str) -> Result<ShellListing, ShellError> {
    let (format, body) = split_marker(path, stdout)?;
    let mut listing = ShellListing {
        path: path.to_owned(),
        entries: Vec::new(),
        skipped: 0,
        format,
    };
    match format {
        ListingFormat::Find => parse_find(path, body, &mut listing)?,
        ListingFormat::Ls => parse_ls(path, body, &mut listing)?,
    }
    Ok(listing)
}

/// Finds the marker line, tolerating whatever the account's shell printed before it.
///
/// The marker is printed with its newline, so a marker without one is a cut answer and not an empty
/// directory; only whole lines are accepted.
fn split_marker<'a>(path: &str, stdout: &'a str) -> Result<(ListingFormat, &'a str), ShellError> {
    let mut start = 0;
    while let Some(end) = stdout[start..].find('\n') {
        let next = start + end + 1;
        let line = &stdout[start..start + end];
        let line = line.strip_suffix('\r').unwrap_or(line);
        match line {
            MARKER_FIND => return Ok((ListingFormat::Find, &stdout[next..])),
            MARKER_LS => return Ok((ListingFormat::Ls, &stdout[next..])),
            _ => {}
        }
        start = next;
    }
    Err(malformed(
        path,
        &format!(
            "the answer did not begin with this side's marker, so the listing command did not run (it began with `{}`)",
            clipped(stdout, 120)
        ),
    ))
}

/// Reads GNU `find` records: nine tab-separated fields and the name, NUL after each.
fn parse_find(path: &str, body: &str, listing: &mut ShellListing) -> Result<(), ShellError> {
    if body.is_empty() {
        return Ok(());
    }
    let Some(records) = body.strip_suffix('\0') else {
        return Err(malformed(path, "the answer ended in the middle of an entry"));
    };
    for record in records.split('\0') {
        // The name is the last field and may contain tabs of its own; splitting from the left and
        // keeping the remainder whole is what makes that work.
        let fields: Vec<&str> = record.splitn(10, '\t').collect();
        if fields.len() != 10 {
            return Err(malformed(
                path,
                &format!("an entry has {} fields where ten belong", fields.len()),
            ));
        }
        let kind = match fields[0] {
            "d" => EntryKind::Directory,
            "f" => EntryKind::File,
            "l" => EntryKind::Symlink,
            // Devices, fifos, sockets and find's own `?`: worth showing, not something to open.
            "b" | "c" | "p" | "s" | "D" | "?" => EntryKind::Other,
            other => {
                return Err(malformed(
                    path,
                    &format!("`{other}` is not a file type a find listing starts with"),
                ));
            }
        };
        let mode = u32::from_str_radix(fields[1], 8)
            .map(|mode| mode & 0o7777)
            .map_err(|_| malformed(path, "an entry's mode is not octal"))?;
        let size = fields[2]
            .parse::<u64>()
            .map_err(|_| malformed(path, "an entry's size is not a number"))?;
        let uid = fields[5]
            .parse::<u32>()
            .map_err(|_| malformed(path, "an entry's owner id is not a number"))?;
        let gid = fields[6]
            .parse::<u32>()
            .map_err(|_| malformed(path, "an entry's group id is not a number"))?;
        push(
            listing,
            ShellEntry {
                name: fields[9].to_owned(),
                kind,
                size,
                mode: Some(mode),
                owner: (!fields[3].is_empty()).then(|| fields[3].to_owned()),
                group: (!fields[4].is_empty()).then(|| fields[4].to_owned()),
                uid: Some(uid),
                gid: Some(gid),
                // A fractional `%T@` is seconds and then a dot; only the seconds are a date this
                // crate's shape carries. A value that did not parse is reported as absent rather
                // than as 1970.
                modified_at: fields[7].split('.').next().and_then(|whole| whole.parse::<u64>().ok()),
                link_target: (!fields[8].is_empty()).then(|| fields[8].to_owned()),
            },
        );
    }
    Ok(())
}

/// Reads `ls -lAH` output: a `total` line, then one long-format line per entry.
fn parse_ls(path: &str, body: &str, listing: &mut ShellListing) -> Result<(), ShellError> {
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.split_whitespace().next() == Some("total") {
            continue;
        }
        push(listing, parse_ls_line(path, line)?);
    }
    Ok(())
}

/// One long-format line.
///
/// Everything is read from the left down to the date, and the name is then the rest of the line: a
/// name may contain spaces, quotes or anything else, so it is never split into fields. The date
/// itself is only validated, not converted — see [`ShellEntry::modified_at`]. A link's name is cut
/// at the first ` -> `; a link *named* `a -> b` is the one case `ls` cannot disambiguate.
fn parse_ls_line(path: &str, line: &str) -> Result<ShellEntry, ShellError> {
    let fields = fields_of(line);
    if fields.len() < 8 {
        return Err(malformed(
            path,
            &format!(
                "a line has {} fields where a long listing has at least eight",
                fields.len()
            ),
        ));
    }
    let (kind, mode) = parse_ls_mode(path, fields[0].text)?;
    if fields[1].text.parse::<u32>().is_err() {
        return Err(malformed(path, "the link count is not a number"));
    }
    let owner = fields[2].text;
    let group = fields[3].text;

    let mut at = 4;
    let size;
    if fields[at].text.ends_with(',') {
        // A device: `ls -l` writes the major and minor numbers as two fields separated by a space,
        // so the size column is two tokens wide and the date begins one later.
        let minor = fields.get(at + 1).map_or("", |field| field.text);
        if minor.parse::<u32>().is_err() {
            return Err(malformed(path, "a device's minor number is not a number"));
        }
        size = 0;
        at += 2;
    } else {
        size = fields[at]
            .text
            .parse::<u64>()
            .map_err(|_| malformed(path, "an entry's size is not a number"))?;
        at += 1;
    }

    let month = fields.get(at).map_or("", |field| field.text);
    if !MONTHS.contains(&month) {
        return Err(malformed(
            path,
            &format!("`{month}` is not an English month name, so the listing is not in the C locale"),
        ));
    }
    at += 1;
    let day = fields.get(at).map_or("", |field| field.text);
    if day.parse::<u32>().map(|day| (1..=31).contains(&day)) != Ok(true) {
        return Err(malformed(path, "the day of the month is not a number between 1 and 31"));
    }
    at += 1;
    let stamp = fields.get(at).map_or("", |field| field.text);
    if !is_clock(stamp) && !is_year(stamp) {
        return Err(malformed(path, &format!("`{stamp}` is neither a time nor a year")));
    }

    // One space separates the date from the name, and it is the *end* of the date field that says
    // where that is: a name beginning with a space keeps it, which taking the next field's start
    // would lose.
    let name_start = fields[at].end + 1;
    let Some(rest) = line.get(name_start..) else {
        return Err(malformed(path, "a line ends before its name"));
    };
    if rest.is_empty() {
        return Err(malformed(path, "a line ends before its name"));
    }

    // Only a link line carries a target, and only a link's name is cut before it. A regular file
    // called `a -> b` is one name; the type character is what tells them apart.
    let (name, link_target) = if kind == EntryKind::Symlink {
        match rest.split_once(" -> ") {
            Some((name, target)) => (name, Some(target.to_owned())),
            None => return Err(malformed(path, "a symbolic link's line does not carry its target")),
        }
    } else {
        (rest, None)
    };

    Ok(ShellEntry {
        name: name.to_owned(),
        kind,
        size,
        mode: Some(mode),
        owner: Some(owner.to_owned()),
        group: Some(group.to_owned()),
        // `ls` prints the number itself when it has no name for it, so a numeric owner is both the
        // name it showed and the id it stands for.
        uid: owner.parse::<u32>().ok(),
        gid: group.parse::<u32>().ok(),
        modified_at: None,
        link_target,
    })
}

/// The file type and permission bits of `drwxr-xr-x` (an ACL marker at the end is ignored).
fn parse_ls_mode(path: &str, text: &str) -> Result<(EntryKind, u32), ShellError> {
    let mut characters = text.chars();
    let kind = match characters.next() {
        Some('d') => EntryKind::Directory,
        Some('-') => EntryKind::File,
        Some('l') => EntryKind::Symlink,
        Some('b' | 'c' | 'p' | 's' | 'D') => EntryKind::Other,
        Some(other) => {
            return Err(malformed(
                path,
                &format!("`{other}` is not a file type a long listing starts with"),
            ));
        }
        None => return Err(malformed(path, "a line begins with nothing")),
    };
    let permissions: Vec<char> = characters.take(9).collect();
    if permissions.len() != 9 {
        return Err(malformed(path, "a permission string is shorter than ten characters"));
    }

    let mut mode = 0u32;
    for (start, shift) in [(0usize, 6u32), (3, 3), (6, 0)] {
        let read = match permissions[start] {
            'r' => 0o4,
            '-' => 0,
            other => return Err(malformed(path, &format!("`{other}` is not a read bit"))),
        };
        let write = match permissions[start + 1] {
            'w' => 0o2,
            '-' => 0,
            other => return Err(malformed(path, &format!("`{other}` is not a write bit"))),
        };
        let execute = match permissions[start + 2] {
            'x' => 0o1,
            's' if start == 0 => {
                mode |= 0o4000;
                0o1
            }
            'S' if start == 0 => {
                mode |= 0o4000;
                0
            }
            's' if start == 3 => {
                mode |= 0o2000;
                0o1
            }
            'S' if start == 3 => {
                mode |= 0o2000;
                0
            }
            't' if start == 6 => {
                mode |= 0o1000;
                0o1
            }
            'T' if start == 6 => {
                mode |= 0o1000;
                0
            }
            '-' => 0,
            other => return Err(malformed(path, &format!("`{other}` is not an execute bit"))),
        };
        mode |= ((read << shift) | (write << shift) | (execute << shift)) as u32;
    }
    Ok((kind, mode))
}

/// A whitespace-separated field and where it ends in the line.
struct Field<'a> {
    text: &'a str,
    end: usize,
}

/// Splits a line into fields while keeping their byte offsets, which is what lets the name be taken
/// as "everything after the date" rather than "the last field".
fn fields_of(line: &str) -> Vec<Field<'_>> {
    let bytes = line.as_bytes();
    let mut fields = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        fields.push(Field {
            text: &line[start..index],
            end: index,
        });
    }
    fields
}

fn is_clock(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && bytes[..2].iter().all(u8::is_ascii_digit)
        && bytes[3..].iter().all(u8::is_ascii_digit)
}

fn is_year(text: &str) -> bool {
    (4..=6).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether a name can be carried as one name.
///
/// A path separator would turn an entry into something the caller's join cannot keep inside the
/// directory; a NUL is not a name anywhere; and the replacement character is how the output became
/// text, so a name carrying one is either an undecodable byte or a character this side cannot tell
/// apart from one. Rewriting a name would rename a file, so such an entry is counted and left out.
fn representable(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
        && !name.contains('\u{FFFD}')
}

fn push(listing: &mut ShellListing, entry: ShellEntry) {
    if representable(&entry.name) {
        listing.entries.push(entry);
    } else {
        listing.skipped += 1;
    }
}

/// Checks a path that is about to become one shell word.
fn checked_operand(path: &str) -> Result<String, ShellError> {
    if path.is_empty() {
        return Err(ShellError::app_internal(
            "a directory listing was asked for without a path",
        ));
    }
    if path.contains('\0') {
        return Err(ShellError::app_internal(
            "a directory listing was asked for a path containing a NUL",
        ));
    }
    if path.contains('\n') || path.contains('\r') {
        return Err(ShellError::app_internal(
            "a directory listing was asked for a path containing a line break",
        ));
    }
    // `--` is not accepted by every `ls` this has to run against, so a relative path that would
    // read as an option is written with `./` instead: the same name, and no option shape left.
    Ok(if path.starts_with('-') {
        format!("./{path}")
    } else {
        path.to_owned()
    })
}

/// One shell word, whatever the path contains.
///
/// Everything inside single quotes is literal to a POSIX shell, including `$`, backticks and
/// backslashes; the only character that cannot appear is the quote itself, which is closed, escaped
/// and reopened. The same rule the SCP commands use, and the same reason: a remote name may be
/// anything, and after this it is still one argument.
fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
}

/// An entry this side cannot read.
fn malformed(path: &str, reason: &str) -> ShellError {
    ShellError::scp_operation_failed(path, format!("the listing could not be read: {reason}"))
}

/// A far side's own complaint, kept to one readable line.
fn failure_detail(what: &str, stderr: &str) -> String {
    let said = clipped(stderr.trim(), 200);
    if said.is_empty() {
        format!("{what} failed without saying anything")
    } else {
        format!("{what} failed: {said}")
    }
}

/// Text from the far side, made safe to show: control characters out, length bounded.
fn clipped(text: &str, limit: usize) -> String {
    let mut clipped: String = text
        .chars()
        .map(|character| if character.is_control() { ' ' } else { character })
        .take(limit)
        .collect();
    if text.chars().count() > limit {
        clipped.push('…');
    }
    clipped.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_one_shell_word_whatever_is_in_it() {
        assert_eq!(quote("/var/log/nginx"), "'/var/log/nginx'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote("x; rm -rf /"), "'x; rm -rf /'");
        assert_eq!(quote("$(reboot)"), "'$(reboot)'");
        assert_eq!(quote("a b`c`d"), "'a b`c`d'");
        assert_eq!(quote("项目/日志"), "'项目/日志'");
    }

    #[test]
    fn a_path_that_looks_like_an_option_is_written_as_a_relative_path() {
        assert_eq!(checked_operand("-rf").expect("a path"), "./-rf");
        assert_eq!(checked_operand("--help").expect("a path"), "./--help");
        assert_eq!(checked_operand("/srv").expect("a path"), "/srv");
        assert_eq!(checked_operand("..").expect("a path"), "..");
    }

    #[test]
    fn a_path_with_a_line_break_or_a_nul_never_becomes_a_command() {
        assert!(checked_operand("").is_err());
        assert!(checked_operand("a\nb").is_err());
        assert!(checked_operand("a\rb").is_err());
        assert!(checked_operand("a\0b").is_err());
        assert!(list_command("a\nb").is_err());
    }

    #[test]
    fn a_permission_string_becomes_the_mode_bits_the_panel_shows() {
        assert_eq!(
            parse_ls_mode("/x", "drwxr-xr-x").expect("a mode"),
            (EntryKind::Directory, 0o755)
        );
        assert_eq!(
            parse_ls_mode("/x", "-rw-r--r--").expect("a mode"),
            (EntryKind::File, 0o644)
        );
        assert_eq!(parse_ls_mode("/x", "-rwsr-xr-x").expect("a mode").1, 0o4755);
        assert_eq!(parse_ls_mode("/x", "drwxrwxrwt").expect("a mode").1, 0o1777);
        // `S` and `T` are the bits without the execute bit they would have replaced.
        assert_eq!(parse_ls_mode("/x", "-rwSr-Sr-T").expect("a mode").1, 0o7644);
        // An ACL marker rides on the end of the string and is not a permission bit.
        assert_eq!(parse_ls_mode("/x", "-rw-r--r--+").expect("a mode").1, 0o644);
    }

    #[test]
    fn a_permission_string_this_side_cannot_read_is_refused() {
        assert!(parse_ls_mode("/x", "zrw-r--r--").is_err());
        assert!(parse_ls_mode("/x", "-rw-r--r").is_err());
        assert!(parse_ls_mode("/x", "-rwxrwxrwz").is_err());
    }

    #[test]
    fn a_time_and_a_year_are_told_apart_by_their_shape() {
        assert!(is_clock("10:30"));
        assert!(!is_clock("1:030"));
        assert!(!is_clock("10-30"));
        assert!(is_year("2024"));
        assert!(!is_year("24"));
        assert!(!is_year("20x4"));
    }
}

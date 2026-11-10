//! Shell listings against answers written down by hand.
//!
//! The far side of a listing is a command's output, so it can be written down here: one fixture per
//! shape, including the names a real file system allows and the answers a server gives when the
//! command it was sent did not run. Nothing here needs a server, which is the point — a real old
//! server is exactly the case that is hard to arrange, and the case these tests are about.

use sevenshell_error::ErrorCode;
use sevenshell_ssh::scp_listing::{ListingFormat, ShellCapabilities, list_command, parse_listing, probe_command};
use sevenshell_types::{EntryKind, SftpEntry};

/// Reads the single-quoted script back out of a command this crate built.
///
/// The command is `/bin/sh -c '<script>'` and the script may itself contain quotes in the `'\''`
/// shape, so decoding is how a test can assert what the far side's shell will actually read.
fn script_of(command: &str) -> String {
    let opening = command.find('\'').expect("the script should be quoted");
    let rest = &command[opening + 1..];
    let bytes = rest.as_bytes();
    let mut decoded = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            if bytes[index..].starts_with(b"'\\''") {
                decoded.push('\'');
                index += 4;
                continue;
            }
            return decoded;
        }
        let character = rest[index..].chars().next().expect("a character");
        decoded.push(character);
        index += character.len_utf8();
    }
    panic!("the script had no closing quote");
}

/// One GNU `find` record: nine fixed fields and the name last, as `-printf` writes it.
fn find_record(kind: char, mode: &str, size: u64, mtime: &str, link: &str, name: &str) -> String {
    format!("{kind}\t{mode}\t{size}\tlabuser\tlabgroup\t1000\t1000\t{mtime}\t{link}\t{name}")
}

/// A whole answer in the `find` shape.
fn find_answer(records: &[String]) -> String {
    let mut answer = String::from("7shell-listing:find\n");
    for record in records {
        answer.push_str(record);
        answer.push('\0');
    }
    answer
}

#[test]
fn a_path_is_one_shell_word_and_the_locale_is_pinned() {
    let command = list_command("/srv/it's a $(reboot); x").expect("a path");
    assert!(
        command.starts_with("/bin/sh -c '"),
        "the script should run under sh whatever the login shell is: {command}"
    );
    let script = script_of(&command);
    assert!(
        script.starts_with("LC_ALL=C; export LC_ALL;"),
        "the locale should be pinned before anything is listed: {script}"
    );
    assert!(
        script.contains("find -H '/srv/it'\\''s a $(reboot); x' -mindepth 1"),
        "the path should be one quoted word: {script}"
    );
}

#[test]
fn a_path_that_would_be_read_as_an_option_is_prefixed_instead() {
    let script = script_of(&list_command("-rf").expect("a path"));
    assert!(
        script.contains("find -H './-rf' -mindepth 1"),
        "find should not see an option: {script}"
    );
    assert!(
        script.contains("ls -lAH './-rf'"),
        "ls should not see an option: {script}"
    );
    assert!(
        !script.contains("find '-rf'"),
        "the bare path should not appear: {script}"
    );
}

#[test]
fn a_path_with_a_line_break_or_a_nul_never_becomes_a_command() {
    for refused in ["", "a\nb", "a\rb", "a\0b"] {
        assert!(
            list_command(refused).is_err(),
            "`{refused}` should have been refused before a command existed"
        );
    }
}

#[test]
fn the_probe_asks_about_scp_and_both_listing_shapes() {
    let script = script_of(&probe_command());
    assert!(
        script.contains("for tool in scp find ls") && script.contains("command -v \"$tool\""),
        "scp, find and ls should each be asked about: {script}"
    );
    assert!(
        script.contains("command -v find") && script.contains("find -H . -mindepth 1 -maxdepth 1 -printf ''"),
        "find's options should be dry-run rather than assumed: {script}"
    );
    assert!(
        script.contains("command -v ls") && script.contains("ls -lAH ."),
        "ls -lAH should be dry-run too: {script}"
    );
}

#[test]
fn a_find_listing_carries_every_name_a_posix_file_system_allows() {
    let answer = find_answer(&[
        find_record('d', "755", 4096, "1756912345.0000000000", "", "logs"),
        find_record('f', "644", 12, "1700000000.1234567890", "", "it's \"quoted\" 项目.txt"),
        find_record('l', "777", 9, "1756912345.0000000000", "/etc/passwd", "current"),
        find_record('p', "644", 0, "1700000000.0000000000", "", "两个\n行"),
        find_record('f', "600", 1, "1700000000.0000000000", "", "tab\there"),
    ]);
    let listing = parse_listing("/srv", &answer).expect("a listing");

    assert_eq!(listing.format, ListingFormat::Find);
    assert_eq!(listing.path, "/srv");
    assert_eq!(listing.skipped, 0);
    assert_eq!(listing.entries.len(), 5);

    let directory = listing
        .entries
        .iter()
        .find(|entry| entry.name == "logs")
        .expect("the directory");
    assert_eq!(directory.kind, EntryKind::Directory);
    assert_eq!(directory.mode, Some(0o755));
    assert_eq!(directory.size, 4096);
    assert_eq!(directory.owner.as_deref(), Some("labuser"));
    assert_eq!(directory.group.as_deref(), Some("labgroup"));
    assert_eq!((directory.uid, directory.gid), (Some(1000), Some(1000)));
    assert_eq!(directory.modified_at, Some(1_756_912_345));
    assert_eq!(directory.link_target, None);

    let file = listing
        .entries
        .iter()
        .find(|entry| entry.name == "it's \"quoted\" 项目.txt")
        .expect("a name with spaces, quotes and Chinese should survive");
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(file.size, 12);
    assert_eq!(file.modified_at, Some(1_700_000_000));

    let link = listing
        .entries
        .iter()
        .find(|entry| entry.name == "current")
        .expect("the symlink");
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(link.link_target.as_deref(), Some("/etc/passwd"));

    assert!(
        listing.entries.iter().any(|entry| entry.name == "两个\n行"),
        "a name with a newline should survive a NUL-terminated record"
    );
    assert!(
        listing.entries.iter().any(|entry| entry.name == "tab\there"),
        "a name with a tab should survive because the name is the last field"
    );
    let fifo = listing
        .entries
        .iter()
        .find(|entry| entry.name == "两个\n行")
        .expect("the fifo");
    assert_eq!(fifo.kind, EntryKind::Other);
}

#[test]
fn an_entry_whose_name_is_not_a_name_is_counted_rather_than_shown() {
    let answer = find_answer(&[
        find_record('f', "644", 1, "1700000000.0", "", "a/b"),
        find_record('f', "644", 1, "1700000000.0", "", ".."),
        find_record('f', "644", 1, "1700000000.0", "", ""),
        find_record('f', "644", 1, "1700000000.0", "", "bad\u{FFFD}name"),
        find_record('f', "644", 1, "1700000000.0", "", "kept"),
    ]);
    let listing = parse_listing("/srv", &answer).expect("a listing");
    assert_eq!(
        listing.skipped, 4,
        "the four names that cannot be represented should be counted"
    );
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, "kept");
}

#[test]
fn a_find_answer_this_side_cannot_read_is_an_error_and_not_a_short_directory() {
    let short = "7shell-listing:find\nf\t644\t1\0";
    let error = parse_listing("/srv", short).expect_err("a record with three fields is not a listing");
    assert_eq!(error.code(), ErrorCode::ScpOperationFailed);

    let cut = "7shell-listing:find\nf\t644\t1\tu\tg\t1\t1\t1.0\t\tname";
    assert!(
        parse_listing("/srv", cut).is_err(),
        "a record without its NUL terminator is a cut answer"
    );
    assert!(
        parse_listing("/srv", "7shell-listing:find").is_err(),
        "a marker without its newline is a cut answer too, not an empty directory"
    );
    assert!(parse_listing("/srv", "find: not found\n").is_err());
    assert!(parse_listing("/srv", "").is_err());
}

#[test]
fn an_empty_directory_is_an_empty_listing_and_a_silent_shell_is_an_error() {
    let empty = parse_listing("/srv", "7shell-listing:find\n").expect("an empty listing");
    assert!(empty.entries.is_empty());
    assert_eq!(empty.skipped, 0);

    let ls_empty = parse_listing("/srv", "7shell-listing:ls\ntotal 0\n").expect("an empty listing");
    assert!(ls_empty.entries.is_empty());
    assert_eq!(ls_empty.format, ListingFormat::Ls);

    let error = parse_listing("/srv", "/bin/sh: command not found\n")
        .expect_err("a shell that ran nothing is not an empty directory");
    assert_eq!(error.code(), ErrorCode::ScpOperationFailed);
}

#[test]
fn a_banner_before_the_marker_is_not_mistaken_for_the_listing() {
    let answer = format!(
        "Welcome to the server\n{}",
        find_answer(&[find_record('f', "644", 3, "1700000000.0", "", "kept")])
    );
    let listing = parse_listing("/srv", &answer).expect("a listing");
    assert_eq!(listing.entries.len(), 1);
}

#[test]
fn a_long_listing_is_read_from_the_right_so_a_spaced_name_survives() {
    let answer = "7shell-listing:ls\n\
        total 20\n\
        drwxr-xr-x 2 labuser labgroup 4096 Nov  9 10:30 项目 目录\n\
        -rw-r--r-- 1 labuser labgroup  123 Nov  9 10:30 it's \"quoted\".txt\n\
        lrwxrwxrwx 1 root    root        9 Jan  2  2024 current -> /etc/passwd\n\
        -rw------- 1 root    root      512 Dec 31  1999 old file\n";
    let listing = parse_listing("/srv", answer).expect("a listing");

    assert_eq!(listing.format, ListingFormat::Ls);
    assert_eq!(listing.skipped, 0);
    assert_eq!(listing.entries.len(), 4);

    let directory = listing
        .entries
        .iter()
        .find(|entry| entry.name == "项目 目录")
        .expect("the directory");
    assert_eq!(directory.kind, EntryKind::Directory);
    assert_eq!(directory.mode, Some(0o755));
    assert_eq!(directory.size, 4096);
    assert_eq!(directory.owner.as_deref(), Some("labuser"));
    assert_eq!((directory.uid, directory.gid), (None, None));
    assert_eq!(
        directory.modified_at, None,
        "an ls date is text in the server's timezone and must not be turned into a number"
    );

    let file = listing
        .entries
        .iter()
        .find(|entry| entry.name == "it's \"quoted\".txt")
        .expect("a name with spaces and quotes should survive");
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(file.size, 123);

    let link = listing
        .entries
        .iter()
        .find(|entry| entry.name == "current")
        .expect("the symlink");
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(link.link_target.as_deref(), Some("/etc/passwd"));

    assert!(
        listing.entries.iter().any(|entry| entry.name == "old file"),
        "the year form of the date should parse too"
    );
}

#[test]
fn a_device_line_in_a_long_listing_does_not_shift_the_name() {
    let answer = "7shell-listing:ls\n\
        total 0\n\
        crw-rw-rw- 1 root root 1, 3 Nov  9 10:30 null\n";
    let listing = parse_listing("/dev", answer).expect("a listing");
    assert_eq!(listing.entries.len(), 1);
    let device = &listing.entries[0];
    assert_eq!(device.name, "null");
    assert_eq!(device.kind, EntryKind::Other);
    assert_eq!(device.size, 0);
    assert_eq!(device.mode, Some(0o666));
}

#[test]
fn a_long_listing_line_that_is_not_a_long_listing_is_an_error() {
    let noise = "7shell-listing:ls\ntotal 4\nbash: ls: command not found\n";
    assert!(
        parse_listing("/srv", noise).is_err(),
        "a shell error is not a directory"
    );

    let localized = "7shell-listing:ls\n-rw-r--r-- 1 root root 12 Nov.  9 10:30 x\n";
    assert!(
        parse_listing("/srv", localized).is_err(),
        "a month that is not English means the listing is not in the C locale"
    );
}

#[test]
fn a_name_with_a_newline_breaks_the_long_format_rather_than_losing_its_tail() {
    let answer = "7shell-listing:ls\n\
        -rw-r--r-- 1 root root 1 Nov  9 10:30 first\n\
        second line\n";
    assert!(
        parse_listing("/srv", answer).is_err(),
        "the continuation line cannot be a record, and half a name is worse than a failure"
    );
}

#[test]
fn the_probe_answer_is_read_as_five_facts() {
    let capabilities =
        ShellCapabilities::parse("scp 1\nfind 1\nls 1\nfind-printf 0\nls-long 1\n").expect("five answers");
    assert!(capabilities.scp);
    assert!(capabilities.find);
    assert!(capabilities.ls);
    assert!(
        !capabilities.find_printf,
        "a dry run that failed is a fact, not a guess"
    );
    assert!(capabilities.ls_long);
    assert!(capabilities.can_list());

    let none = ShellCapabilities::parse("scp 0\nfind 0\nls 0\nfind-printf 0\nls-long 0\n").expect("five answers");
    assert!(!none.can_list());
}

#[test]
fn a_probe_answer_that_is_missing_or_doubled_is_an_error() {
    assert!(
        ShellCapabilities::parse("scp 1\nfind 1\nls 1\nfind-printf 0\n").is_err(),
        "an unanswered question must not become a no"
    );
    assert!(
        ShellCapabilities::parse("scp 1\nscp 0\nfind 1\nls 1\nfind-printf 0\nls-long 1\n").is_err(),
        "an answer given twice is not one answer"
    );
    assert!(
        ShellCapabilities::parse("scp maybe\nfind 1\nls 1\nfind-printf 0\nls-long 1\n").is_err(),
        "a word that is not yes or no is not an answer"
    );
    assert!(
        ShellCapabilities::parse("Last login: Monday\nscp 1\nfind 1\nls 1\nfind-printf 1\nls-long 0\n").is_ok(),
        "a login banner is not one of the questions"
    );
}

#[test]
fn a_shell_entry_maps_onto_the_panel_entry_without_inventing_a_field() {
    // This is the mapping the caller performs. If the wire type grows or renames a field, this stops
    // compiling rather than leaving the new field silently at its default.
    let answer = find_answer(&[find_record('f', "640", 42, "1700000000.5", "", "报告.txt")]);
    let mut listing = parse_listing("/srv", &answer).expect("a listing");
    let shell = listing.entries.remove(0);
    let wire = SftpEntry {
        path: format!("/srv/{}", shell.name),
        name: shell.name,
        kind: shell.kind,
        size: shell.size,
        modified_at: shell.modified_at,
        mode: shell.mode,
        owner: shell.owner,
        group: shell.group,
        uid: shell.uid,
        gid: shell.gid,
        link_target: shell.link_target,
    };
    assert_eq!(wire.path, "/srv/报告.txt");
    assert_eq!(wire.kind, EntryKind::File);
    assert_eq!(wire.size, 42);
    assert_eq!(wire.mode, Some(0o640));
}

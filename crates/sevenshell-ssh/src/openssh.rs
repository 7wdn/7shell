//! The parts of `~/.ssh/config` a connection actually needs.
//!
//! OpenSSH's configuration language is large, and most of it is about tools this build is not:
//! X11, local commands, control sockets, canonicalisation. What a saved connection needs is the
//! small set that decides *where to connect and who to be*: `Host`, `HostName`, `User`, `Port`,
//! `IdentityFile`, `IdentitiesOnly`, `Include`, `ProxyJump` and `ProxyCommand`.
//!
//! Two rules are deliberately conservative:
//!
//! * **A directive this build cannot honour is carried, not discarded.** A `ProxyCommand` or a
//!   `Match` block changes whether connecting directly would even reach the right machine. The
//!   parser records it in [`ResolvedHost::unsupported`], and the connection layer refuses rather
//!   than silently taking a different route.
//! * **The order is OpenSSH's.** For every option except `IdentityFile`, the first value obtained
//!   wins; identity files accumulate and are tried in the order they were written. A parser that
//!   picked the "most specific" block would disagree with the client the person already uses.
//!
//! Paths are not touched here: `load` reads the files it is pointed at, and everything else is
//! plain text in and plain values out.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use russh::keys::PublicKey;

/// The identities OpenSSH tries, in its own order, when a host names none.
pub const DEFAULT_IDENTITY_FILES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// One destination named by `ProxyJump`.
///
/// The host is still a name: whether it is an alias with its own `Host` block, or a literal
/// address, is decided where the configuration is resolved, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpStep {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

/// Parses a `ProxyJump` value: comma-separated `[user@]host[:port]`, or `none` for no jump.
///
/// A malformed element is returned as written rather than dropped: a route that quietly loses a
/// hop dials a different machine than the person's own `ssh` would. Brackets are honoured for
/// IPv6, and an unbracketed address with more than one colon is read as a host without a port,
/// which is what OpenSSH expects.
pub fn parse_proxy_jump(value: &str) -> Result<Vec<JumpStep>, String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }

    let mut steps = Vec::new();
    for raw in value.split(',') {
        let hop = raw.trim();
        if hop.is_empty() {
            return Err(raw.to_owned());
        }
        steps.push(parse_jump_step(hop)?);
    }
    Ok(steps)
}

fn parse_jump_step(hop: &str) -> Result<JumpStep, String> {
    let (user, rest) = match hop.rsplit_once('@') {
        Some((user, rest)) => {
            if user.is_empty() {
                return Err(hop.to_owned());
            }
            (Some(user.to_owned()), rest)
        }
        None => (None, hop),
    };

    let (host, port) = if let Some(bracketed) = rest.strip_prefix('[') {
        let Some((address, after)) = bracketed.split_once(']') else {
            return Err(hop.to_owned());
        };
        let port = match after {
            "" => None,
            after => match after.strip_prefix(':') {
                Some(port) => Some(proxy_port(port).ok_or_else(|| hop.to_owned())?),
                None => return Err(hop.to_owned()),
            },
        };
        (address.to_owned(), port)
    } else if rest.matches(':').count() == 1 {
        let (address, port) = rest.rsplit_once(':').expect("one colon");
        (
            address.to_owned(),
            Some(proxy_port(port).ok_or_else(|| hop.to_owned())?),
        )
    } else {
        // No colon at all, or an unbracketed IPv6 address whose colons are not a port.
        (rest.to_owned(), None)
    };

    if host.is_empty() {
        return Err(hop.to_owned());
    }
    Ok(JumpStep { user, host, port })
}

fn proxy_port(value: &str) -> Option<u16> {
    value.parse::<u16>().ok().filter(|port| *port != 0)
}

/// What this machine's own `known_hosts` says about the key a server just presented.
///
/// `Changed` is a stop, not a question: this build never edits the person's OpenSSH files, so there
/// is nothing here it could offer to replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemHostKey {
    Trusted,
    Unknown,
    Changed,
}

/// Checks a presented key against an OpenSSH `known_hosts` file.
///
/// The path is passed in because the caller owns where `~/.ssh` is; a test or a controlled run may
/// point it somewhere else, and this crate does not guess at a home directory. A file that cannot
/// be read is `Unknown`, which asks the person through this application's own prompt — never a
/// silent trust.
pub fn check_system_known_host(path: &Path, host: &str, port: u16, key_type: &str, public_key: &str) -> SystemHostKey {
    let Ok(presented) = russh::keys::parse_public_key_base64(public_key) else {
        return SystemHostKey::Unknown;
    };
    if presented.algorithm().as_str() != key_type {
        return SystemHostKey::Unknown;
    }
    let Ok(recorded) = russh::keys::known_hosts::known_host_keys_path(host, port, path) else {
        return SystemHostKey::Unknown;
    };
    verdict(&presented, &recorded)
}

/// What the records for one endpoint say about the key just presented.
///
/// A record that matches is an answer even when the file also holds an older record of the same
/// algorithm: the file is asserting the person trusted this key, and which line came first is an
/// accident of history. `ssh` itself stops at the first same-algorithm line it reads, so a stale
/// line ahead of the current one makes it refuse where the record it needs is right there
/// (issue #13, where other clients connected while this one refused). Only records that speak for
/// this key's algorithm count; a different algorithm says nothing about this key and is treated
/// as unseen.
fn verdict(presented: &PublicKey, recorded: &[(usize, PublicKey)]) -> SystemHostKey {
    let mut same_algorithm_differs = false;
    for (_, key) in recorded {
        if key.algorithm() != presented.algorithm() {
            continue;
        }
        if key.key_data() == presented.key_data() {
            return SystemHostKey::Trusted;
        }
        same_algorithm_differs = true;
    }
    if same_algorithm_differs {
        SystemHostKey::Changed
    } else {
        SystemHostKey::Unknown
    }
}

/// How deep an `Include` chain may go before the rest is ignored.
const MAX_INCLUDE_DEPTH: usize = 16;

/// One parsed file, with `Include` already expanded by [`load`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedConfig {
    global: Options,
    blocks: Vec<Block>,
    /// A `Match` block this build cannot evaluate. Every resolution reports it.
    unsupported_match: bool,
    /// An `Include` reached through [`parse`] rather than [`load`], which follows them.
    unsupported_include: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Block {
    patterns: Vec<Pattern>,
    options: Options,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    text: String,
    negated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Options {
    host_name: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity_files: Vec<String>,
    identities_only: Option<bool>,
    proxy_jump: Option<String>,
    proxy_command: Option<String>,
    unsupported: Vec<String>,
}

impl Options {
    /// Fills what is still unset and appends what accumulates, which is OpenSSH's first-obtained
    /// rule expressed as a merge.
    fn absorb(&mut self, other: &Self) {
        if self.host_name.is_none() {
            self.host_name = other.host_name.clone();
        }
        if self.user.is_none() {
            self.user = other.user.clone();
        }
        if self.port.is_none() {
            self.port = other.port;
        }
        if self.identities_only.is_none() {
            self.identities_only = other.identities_only;
        }
        if self.proxy_jump.is_none() {
            self.proxy_jump = other.proxy_jump.clone();
        }
        if self.proxy_command.is_none() {
            self.proxy_command = other.proxy_command.clone();
        }
        self.identity_files.extend(other.identity_files.iter().cloned());
        for name in &other.unsupported {
            if !self.unsupported.contains(name) {
                self.unsupported.push(name.clone());
            }
        }
    }
}

/// What one alias resolves to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedHost {
    /// The real address, or nothing when the config names none and the alias itself is the host.
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// `IdentityFile` values in the order they were written, with `~` still abbreviated.
    pub identity_files: Vec<String>,
    pub identities_only: bool,
    /// Carried so the connection layer can refuse rather than silently connect directly.
    pub proxy_jump: Option<String>,
    /// The same, for a command this build will not execute.
    pub proxy_command: Option<String>,
    /// Directives a connection must not ignore: `ProxyCommand`, `Match`, a bad port. `ProxyJump`
    /// is not here — it is a route this build follows, carried in [`Self::proxy_jump`].
    pub unsupported: Vec<String>,
    /// Whether any `Host` block matched. A caller that promised the alias comes from this file
    /// refuses when it does not, rather than dialling the alias as a literal host name.
    pub matched: bool,
}

impl ResolvedHost {
    /// The address to dial. A config without `HostName` means the alias itself is the address.
    pub fn address(&self, alias: &str) -> String {
        self.host_name.clone().unwrap_or_else(|| alias.to_owned())
    }
}

impl ParsedConfig {
    /// A concrete alias the picker can offer: a positive pattern with no wildcard.
    pub fn aliases(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut aliases = Vec::new();
        for block in &self.blocks {
            for pattern in &block.patterns {
                if pattern.negated || pattern.text.is_empty() {
                    continue;
                }
                if pattern.text.contains(['*', '?']) {
                    continue;
                }
                let key = pattern.text.to_ascii_lowercase();
                if seen.insert(key) {
                    aliases.push(pattern.text.clone());
                }
            }
        }
        aliases
    }

    /// Everything a connection to `alias` needs, with the directives it must not ignore.
    pub fn resolve(&self, alias: &str) -> ResolvedHost {
        let mut merged = Options::default();
        merged.absorb(&self.global);
        let mut matched = false;
        for block in &self.blocks {
            if matches(&block.patterns, alias) {
                matched = true;
                merged.absorb(&block.options);
            }
        }

        let mut unsupported = merged.unsupported;
        if self.unsupported_match && !unsupported.iter().any(|name| name == "Match") {
            unsupported.push("Match".to_owned());
        }
        if self.unsupported_include && !unsupported.iter().any(|name| name == "Include") {
            unsupported.push("Include".to_owned());
        }

        ResolvedHost {
            host_name: merged.host_name,
            user: merged.user,
            port: merged.port,
            identity_files: merged.identity_files,
            identities_only: merged.identities_only.unwrap_or(false),
            proxy_jump: merged.proxy_jump,
            proxy_command: merged.proxy_command,
            unsupported,
            matched,
        }
    }
}

/// Parses text that has already had its `Include` lines expanded.
///
/// An `Include` here is reported as unsupported: the caller asked the parser to read a text, not a
/// file tree, and silently pretending the directive was not there would change the route.
pub fn parse(text: &str) -> ParsedConfig {
    let mut config = ParsedConfig::default();
    parse_lines(text, &mut config);
    config
}

/// Reads a config file and every file it includes, then parses the result.
///
/// A missing main file is an empty configuration, not an error: a machine without OpenSSH is a
/// normal state. `home` is used for `~` and `%d` in include patterns.
pub fn load(path: &Path, home: &Path) -> ParsedConfig {
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut seen = HashSet::new();
    let text = expand_file(path, base, home, 0, &mut seen);
    parse(&text)
}

fn parse_lines(text: &str, config: &mut ParsedConfig) {
    let mut current: Option<usize> = None;
    let mut inside_match = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = split_directive(line) else {
            continue;
        };

        match key.to_ascii_lowercase().as_str() {
            "host" => {
                inside_match = false;
                let patterns = patterns(value);
                if patterns.is_empty() {
                    current = None;
                    continue;
                }
                config.blocks.push(Block {
                    patterns,
                    options: Options::default(),
                });
                current = Some(config.blocks.len() - 1);
            }
            "match" => {
                // Conditions such as `exec` can decide whether a later block applies. Refusing is
                // the only answer that cannot silently take a different route.
                config.unsupported_match = true;
                inside_match = true;
                current = None;
            }
            "include" => {
                config.unsupported_include = true;
            }
            _ => {
                if inside_match {
                    continue;
                }
                let options = match current {
                    Some(index) => &mut config.blocks[index].options,
                    None => &mut config.global,
                };
                apply(options, key, value);
            }
        }
    }
}

fn split_directive(line: &str) -> Option<(&str, &str)> {
    let split = line
        .char_indices()
        .find(|(_, character)| character.is_whitespace() || *character == '=')
        .map(|(index, _)| index);
    let (key, rest) = match split {
        Some(index) => (&line[..index], line[index..].trim_start_matches(['=', ' ', '\t'])),
        None => (line, ""),
    };
    if key.is_empty() {
        return None;
    }
    Some((key, unquote(rest)))
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn patterns(value: &str) -> Vec<Pattern> {
    value
        .split_whitespace()
        .filter(|pattern| !pattern.is_empty())
        .map(|pattern| match pattern.strip_prefix('!') {
            Some(rest) => Pattern {
                text: rest.to_owned(),
                negated: true,
            },
            None => Pattern {
                text: pattern.to_owned(),
                negated: false,
            },
        })
        .collect()
}

fn apply(options: &mut Options, key: &str, value: &str) {
    match key.to_ascii_lowercase().as_str() {
        "hostname" => set_once(&mut options.host_name, value),
        "user" => set_once(&mut options.user, value),
        "port" => match value.parse::<u16>() {
            Ok(port) => {
                if options.port.is_none() {
                    options.port = Some(port);
                }
            }
            Err(_) => note_unsupported(&mut options.unsupported, "Port"),
        },
        "identityfile" => {
            if !value.is_empty() {
                options.identity_files.push(value.to_owned());
            }
        }
        "identitiesonly" => {
            if options.identities_only.is_none() {
                options.identities_only = Some(matches!(value.to_ascii_lowercase().as_str(), "yes" | "true" | "on"));
            }
        }
        "proxyjump" => set_once(&mut options.proxy_jump, value),
        "proxycommand" => set_once(&mut options.proxy_command, value),
        _ => {}
    }
}

fn set_once(slot: &mut Option<String>, value: &str) {
    if slot.is_none() && !value.is_empty() {
        *slot = Some(value.to_owned());
    }
}

fn note_unsupported(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|existing| existing == name) {
        list.push(name.to_owned());
    }
}

/// Whether a host matches a block: one positive pattern must match, and no negated one may.
fn matches(patterns: &[Pattern], alias: &str) -> bool {
    let mut positive = false;
    for pattern in patterns {
        if pattern.negated {
            if glob_match(&pattern.text, alias) {
                return false;
            }
        } else if glob_match(&pattern.text, alias) {
            positive = true;
        }
    }
    positive
}

/// OpenSSH's host patterns, which are `*` and `?`, case-insensitively.
fn glob_match(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let value: Vec<char> = value.to_ascii_lowercase().chars().collect();
    let mut p = 0;
    let mut v = 0;
    let mut star: Option<(usize, usize)> = None;

    while v < value.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, v));
            p += 1;
        } else if let Some((star_p, star_v)) = star {
            p = star_p + 1;
            v = star_v + 1;
            star = Some((star_p, star_v + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

fn expand_file(path: &Path, base: &Path, home: &Path, depth: usize, seen: &mut HashSet<PathBuf>) -> String {
    if depth > MAX_INCLUDE_DEPTH {
        return String::new();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canonical.clone()) {
        return String::new();
    }

    let mut expanded = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some((key, value)) = split_directive(trimmed) else {
            expanded.push_str(line);
            expanded.push('\n');
            continue;
        };
        if key.eq_ignore_ascii_case("include") {
            for pattern in value.split_whitespace() {
                for included in include_paths(pattern, base, home) {
                    expanded.push_str(&expand_file(&included, base, home, depth + 1, seen));
                }
            }
        } else {
            expanded.push_str(line);
            expanded.push('\n');
        }
    }

    seen.remove(&canonical);
    expanded
}

/// The files one `Include` pattern names, in the order OpenSSH would read them.
fn include_paths(pattern: &str, base: &Path, home: &Path) -> Vec<PathBuf> {
    let expanded = expand_home(pattern, home);
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    };

    let Some(name) = candidate.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let directory = candidate.parent().unwrap_or(base);

    if !name.contains(['*', '?']) {
        return candidate.is_file().then_some(candidate).into_iter().collect();
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut matches: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(|file| glob_match(name, file)))
        .map(|entry| entry.path())
        .collect();
    matches.sort();
    matches
}

/// Replaces the leading `~` or a `%d` with the home directory. Other OpenSSH tokens are left
/// alone: a path containing one is not silently replaced with a guess.
pub fn expand_home(value: &str, home: &Path) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/").or_else(|| value.strip_prefix("~\\")) {
        return home.join(rest);
    }
    if value == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = value.strip_prefix("%d/").or_else(|| value.strip_prefix("%d\\")) {
        return home.join(rest);
    }
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn a_host_block_fills_the_address_and_the_account() {
        let config = parse("Host web\n  HostName 10.0.0.5\n  User deploy\n  Port 2222\n");
        let resolved = config.resolve("web");

        assert_eq!(resolved.address("web"), "10.0.0.5");
        assert_eq!(resolved.user.as_deref(), Some("deploy"));
        assert_eq!(resolved.port, Some(2222));
    }

    #[test]
    fn a_host_without_host_name_is_dialled_by_its_alias() {
        let config = parse("Host web\n  User deploy\n");
        assert_eq!(config.resolve("web").address("web"), "web");
    }

    #[test]
    fn the_first_value_wins_in_file_order() {
        let config = parse("Host *\n  User fallback\n  Port 2200\nHost web\n  User specific\n  Port 2022\n");
        let resolved = config.resolve("web");
        assert_eq!(
            resolved.user.as_deref(),
            Some("fallback"),
            "the first block already answered"
        );
        assert_eq!(resolved.port, Some(2200), "and the second value does not replace it");
    }

    #[test]
    fn a_negated_pattern_keeps_a_block_from_matching() {
        let config = parse("Host web !web.internal\n  User deploy\n");
        assert_eq!(config.resolve("web").user.as_deref(), Some("deploy"));
        assert_eq!(config.resolve("web.internal").user, None);
    }

    #[test]
    fn a_wildcard_block_applies_after_the_specific_one() {
        let config = parse("Host db-*\n  Port 5432\nHost *\n  User ops\n");
        let db = config.resolve("db-1");
        assert_eq!(db.port, Some(5432));
        assert_eq!(db.user.as_deref(), Some("ops"));
    }

    #[test]
    fn identity_files_accumulate_in_the_order_they_were_written() {
        let config = parse("Host web\n  IdentityFile ~/.ssh/id_work\nHost *\n  IdentityFile ~/.ssh/id_fallback\n");
        let resolved = config.resolve("web");
        assert_eq!(resolved.identity_files, ["~/.ssh/id_work", "~/.ssh/id_fallback"]);
        assert!(!resolved.identities_only);
    }

    #[test]
    fn identities_only_is_read_as_a_switch_and_keeps_the_first_answer() {
        let config = parse("Host web\n  IdentitiesOnly yes\nHost *\n  IdentitiesOnly no\n");
        assert!(config.resolve("web").identities_only);
    }

    #[test]
    fn proxy_directives_are_carried_so_the_caller_can_refuse_them() {
        let config = parse("Host web\n  ProxyJump bastion\n  ProxyCommand ssh -W %h:%p bastion\n");
        let resolved = config.resolve("web");
        assert_eq!(resolved.proxy_jump.as_deref(), Some("bastion"));
        assert_eq!(resolved.proxy_command.as_deref(), Some("ssh -W %h:%p bastion"));
    }

    #[test]
    fn an_unreadable_port_is_named_instead_of_guessed_at() {
        let config = parse("Host web\n  Port not-a-number\n");
        let resolved = config.resolve("web");
        assert_eq!(resolved.port, None);
        assert!(resolved.unsupported.iter().any(|name| name == "Port"));
    }

    #[test]
    fn a_match_block_is_reported_as_unsupported() {
        let config = parse("Match host web\n  User deploy\nHost *\n  User ops\n");
        let resolved = config.resolve("web");
        assert_eq!(resolved.user.as_deref(), Some("ops"), "the Match body is not applied");
        assert!(resolved.unsupported.iter().any(|name| name == "Match"));
    }

    #[test]
    fn comments_blank_lines_and_crlf_do_not_confuse_the_parser() {
        let config = parse("# a comment\r\n\r\nHost web\r\n  # inside\r\n  HostName 10.0.0.5\r\n  User deploy\r\n");
        let resolved = config.resolve("web");
        assert_eq!(resolved.address("web"), "10.0.0.5");
        assert_eq!(resolved.user.as_deref(), Some("deploy"));
    }

    #[test]
    fn key_equals_value_is_accepted_as_well_as_whitespace() {
        let config = parse("Host web\n  HostName=10.0.0.5\n  User=deploy\n");
        let resolved = config.resolve("web");
        assert_eq!(resolved.address("web"), "10.0.0.5");
        assert_eq!(resolved.user.as_deref(), Some("deploy"));
    }

    #[test]
    fn the_alias_list_contains_concrete_names_and_no_patterns() {
        let config = parse("Host web db-* !internal *.example.com\n  User ops\nHost *\n  Port 22\n");
        assert_eq!(config.aliases(), ["web"]);
    }

    #[test]
    fn an_include_is_followed_when_the_file_tree_is_read() {
        let directory = tempdir().expect("a temporary directory");
        let ssh = directory.path().join(".ssh");
        let fragments = ssh.join("config.d");
        fs::create_dir_all(&fragments).expect("the fragment directory");
        fs::write(ssh.join("config"), "Include config.d/*\nHost web\n  User deploy\n").expect("the main config");
        fs::write(
            fragments.join("10-work"),
            "Host web\n  HostName 10.0.0.5\n  IdentityFile ~/.ssh/id_work\n",
        )
        .expect("the fragment");

        let config = load(&ssh.join("config"), directory.path());
        let resolved = config.resolve("web");
        assert_eq!(resolved.address("web"), "10.0.0.5");
        assert_eq!(resolved.user.as_deref(), Some("deploy"));
        assert_eq!(resolved.identity_files, ["~/.ssh/id_work"]);
    }

    #[test]
    fn an_include_that_matches_nothing_leaves_the_rest_alone() {
        let directory = tempdir().expect("a temporary directory");
        let config_path = directory.path().join("config");
        fs::write(&config_path, "Include missing.d/*\nHost web\n  User deploy\n").expect("the config");

        let config = load(&config_path, directory.path());
        assert_eq!(config.resolve("web").user.as_deref(), Some("deploy"));
    }

    #[test]
    fn a_missing_config_file_is_an_empty_configuration() {
        let directory = tempdir().expect("a temporary directory");
        let config = load(&directory.path().join("not-there"), directory.path());
        assert!(config.aliases().is_empty());
        assert_eq!(config.resolve("anything").address("anything"), "anything");
    }

    #[test]
    fn a_proxy_jump_value_is_read_as_its_hops_in_order() {
        assert_eq!(parse_proxy_jump("none"), Ok(Vec::new()));
        assert_eq!(parse_proxy_jump(" NONE "), Ok(Vec::new()));
        assert_eq!(
            parse_proxy_jump("bastion,deploy@10.0.0.9:2222"),
            Ok(vec![
                JumpStep {
                    user: None,
                    host: "bastion".to_owned(),
                    port: None,
                },
                JumpStep {
                    user: Some("deploy".to_owned()),
                    host: "10.0.0.9".to_owned(),
                    port: Some(2222),
                },
            ])
        );
    }

    #[test]
    fn an_ipv6_hop_is_read_with_and_without_a_port() {
        assert_eq!(
            parse_proxy_jump("[2001:db8::1]:2200"),
            Ok(vec![JumpStep {
                user: None,
                host: "2001:db8::1".to_owned(),
                port: Some(2200),
            }])
        );
        assert_eq!(
            parse_proxy_jump("ops@[2001:db8::1]"),
            Ok(vec![JumpStep {
                user: Some("ops".to_owned()),
                host: "2001:db8::1".to_owned(),
                port: None,
            }])
        );
        // Unbracketed, the colons belong to the address rather than to a port.
        assert_eq!(
            parse_proxy_jump("2001:db8::1"),
            Ok(vec![JumpStep {
                user: None,
                host: "2001:db8::1".to_owned(),
                port: None,
            }])
        );
    }

    #[test]
    fn a_malformed_hop_is_returned_as_written() {
        assert_eq!(parse_proxy_jump("bastion:"), Err("bastion:".to_owned()));
        assert_eq!(parse_proxy_jump(":22"), Err(":22".to_owned()));
        assert_eq!(parse_proxy_jump("@bastion"), Err("@bastion".to_owned()));
        assert_eq!(parse_proxy_jump("bastion, ,db"), Err(" ".to_owned()));
        assert_eq!(parse_proxy_jump("bastion:0"), Err("bastion:0".to_owned()));
        assert_eq!(parse_proxy_jump("[2001:db8::1"), Err("[2001:db8::1".to_owned()));
    }

    #[test]
    fn the_glob_matches_the_way_openssh_documents_it() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("web-?", "web-1"));
        assert!(glob_match("WEB-*", "web-1"));
        assert!(!glob_match("web-?", "web-10"));
        assert!(!glob_match("db-*", "web-1"));
    }

    fn a_key() -> russh::keys::PrivateKey {
        let mut rng = russh::keys::key::safe_rng();
        russh::keys::PrivateKey::random(&mut rng, russh::keys::Algorithm::Ed25519).expect("a generated key")
    }

    #[test]
    fn a_key_recorded_in_the_system_known_hosts_is_trusted() {
        use russh::keys::PublicKeyBase64 as _;

        let directory = tempdir().expect("a temporary directory");
        let path = directory.path().join("known_hosts");
        let key = a_key();
        let public = key.public_key();
        fs::write(
            &path,
            format!("web.example {}\n", public.to_openssh().expect("a public line")),
        )
        .expect("the known_hosts file");

        assert_eq!(
            check_system_known_host(
                &path,
                "web.example",
                22,
                public.algorithm().as_str(),
                &public.public_key_base64()
            ),
            SystemHostKey::Trusted
        );
        assert_eq!(
            check_system_known_host(
                &path,
                "other.example",
                22,
                public.algorithm().as_str(),
                &public.public_key_base64()
            ),
            SystemHostKey::Unknown
        );
    }

    #[test]
    fn a_different_key_in_the_system_file_is_a_stop() {
        let directory = tempdir().expect("a temporary directory");
        let path = directory.path().join("known_hosts");
        let recorded_key = a_key();
        let recorded = recorded_key.public_key();
        fs::write(
            &path,
            format!("web.example {}\n", recorded.to_openssh().expect("a public line")),
        )
        .expect("the known_hosts file");

        let presented_key = a_key();
        let presented = presented_key.public_key();
        use russh::keys::PublicKeyBase64 as _;
        assert_eq!(
            check_system_known_host(
                &path,
                "web.example",
                22,
                presented.algorithm().as_str(),
                &presented.public_key_base64()
            ),
            SystemHostKey::Changed
        );
    }

    #[test]
    fn a_matching_record_is_trusted_whatever_other_records_of_the_same_algorithm_say() {
        use russh::keys::PublicKeyBase64 as _;

        let directory = tempdir().expect("a temporary directory");
        let path = directory.path().join("known_hosts");
        let current_key = a_key();
        let stale_key = a_key();
        let current = current_key.public_key();
        let stale = stale_key.public_key();
        let current_line = format!("web.example {}", current.to_openssh().expect("a public line"));
        let stale_line = format!("web.example {}", stale.to_openssh().expect("a public line"));

        // The stale record first is the order in which `ssh` itself stops, and the one that made
        // 0.4.0 refuse a host the person had already accepted.
        fs::write(&path, format!("{stale_line}\n{current_line}\n")).expect("the known_hosts file");
        assert_eq!(
            check_system_known_host(&path, "web.example", 22, "ssh-ed25519", &current.public_key_base64()),
            SystemHostKey::Trusted
        );

        // The order the other way round is what `ssh` already accepts; both are the same answer.
        fs::write(&path, format!("{current_line}\n{stale_line}\n")).expect("the known_hosts file");
        assert_eq!(
            check_system_known_host(&path, "web.example", 22, "ssh-ed25519", &current.public_key_base64()),
            SystemHostKey::Trusted
        );
    }

    #[test]
    fn a_record_of_another_algorithm_is_not_a_change() {
        use russh::keys::PublicKeyBase64 as _;

        let directory = tempdir().expect("a temporary directory");
        let path = directory.path().join("known_hosts");
        let mut rng = russh::keys::key::safe_rng();
        let recorded_key = russh::keys::PrivateKey::random(
            &mut rng,
            russh::keys::Algorithm::Ecdsa {
                curve: russh::keys::EcdsaCurve::NistP256,
            },
        )
        .expect("a generated key");
        let recorded = recorded_key.public_key();
        fs::write(
            &path,
            format!("web.example {}\n", recorded.to_openssh().expect("a public line")),
        )
        .expect("the known_hosts file");

        let presented_key = a_key();
        let presented = presented_key.public_key();
        assert_eq!(
            check_system_known_host(
                &path,
                "web.example",
                22,
                presented.algorithm().as_str(),
                &presented.public_key_base64()
            ),
            SystemHostKey::Unknown,
            "a record of another algorithm says nothing about this key, so the person is asked"
        );
    }

    #[test]
    fn a_missing_or_unreadable_file_asks_rather_than_trusting() {
        let directory = tempdir().expect("a temporary directory");
        let key = a_key();
        let public = key.public_key();
        use russh::keys::PublicKeyBase64 as _;

        assert_eq!(
            check_system_known_host(
                &directory.path().join("not-there"),
                "web.example",
                22,
                public.algorithm().as_str(),
                &public.public_key_base64()
            ),
            SystemHostKey::Unknown
        );
        assert_eq!(
            check_system_known_host(
                &directory.path().join("not-there"),
                "web.example",
                22,
                "not-a-key",
                "garbage"
            ),
            SystemHostKey::Unknown
        );
    }
}

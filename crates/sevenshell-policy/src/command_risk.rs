//! How dangerous a command is, before anybody runs it.
//!
//! Three decisions shape this module, and each of them is a rule about not crying wolf:
//!
//! 1. **It reads tokens, not text.** `echo "rm -rf /"` deletes nothing, so it is not destructive;
//!    a classifier that greps the raw string says it is, people learn to ignore it, and the one
//!    message that mattered gets ignored with it.
//! 2. **It explains itself.** Every verdict carries the rules that produced it with a message key,
//!    so the interface shows a reason and a person can say "that rule is wrong" about something
//!    specific.
//! 3. **It errs towards caution, and says what it does not know.** A command it cannot parse is
//!    `Caution`, never `Safe`: the verdict is a hint for a human, not a security boundary, and
//!    pretending otherwise would be worse than saying so.

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskReason {
    pub rule_id: &'static str,
    pub label_key: &'static str,
    pub level: RiskLevel,
}

/// What the classifier concluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskAssessment {
    pub level: RiskLevel,
    pub reasons: Vec<RiskReason>,
}

impl RiskAssessment {
    fn from_reasons(reasons: Vec<RiskReason>) -> Self {
        let level = reasons
            .iter()
            .map(|reason| reason.level)
            .max()
            .unwrap_or(RiskLevel::Safe);
        Self { level, reasons }
    }

    /// Whether an unattended caller should stop and ask.
    pub fn needs_approval(&self) -> bool {
        self.level.needs_approval()
    }
}

/// The parts of a command the rules look at.
struct Parsed {
    /// Words that are not inside quotes, which is what actually runs.
    words: Vec<String>,
    /// The first word of each pipe's destination: `curl … | sudo bash` puts `sudo` here, and the
    /// `bash` that runs is a word of that segment even though it is not the command's program.
    piped: Vec<String>,
    /// Everything outside quotes, lower-cased, for the phrases that are not single words.
    loose: String,
}

/// Classifies one command line.
pub fn classify(command: &str) -> RiskAssessment {
    let parsed = parse(command);
    if parsed.words.is_empty() {
        return RiskAssessment {
            level: RiskLevel::Safe,
            reasons: Vec::new(),
        };
    }

    let mut reasons: Vec<RiskReason> = RULES
        .iter()
        .filter(|rule| (rule.when)(&parsed))
        .map(RiskReason::from)
        .collect();

    // A command whose program is a shell cannot be read as a list of words any more: what runs is
    // whatever the string contains, and the words after the shell name are arguments to it rather
    // than the program. This is a fact worth stating next to the other reasons, not instead of
    // them — `bash -c "rm -rf /"` is still a recursive forced delete.
    if program_word(&parsed).is_some_and(is_shell) {
        reasons.push(reason(RULE_UNREADABLE));
    }

    RiskAssessment::from_reasons(reasons)
}

fn reason(rule: Rule) -> RiskReason {
    RiskReason {
        rule_id: rule.id,
        label_key: rule.label_key,
        level: rule.level,
    }
}

impl From<&Rule> for RiskReason {
    fn from(rule: &Rule) -> Self {
        Self {
            rule_id: rule.id,
            label_key: rule.label_key,
            level: rule.level,
        }
    }
}

/// Splits a command line into the words that would run, dropping those inside quotes.
///
/// Quotes are removed rather than kept: a word inside quotes is data, and this is asking what will
/// *happen*, not what will be printed. Escapes are handled to the extent that a shell handles them
/// outside quotes, which is enough for the shapes a rule cares about.
fn parse(command: &str) -> Parsed {
    let mut words = Vec::new();
    let mut piped = Vec::new();
    let mut loose = String::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut starts_piped_segment = false;

    for character in command.chars() {
        if escaped {
            // Outside quotes an escape makes the next character literal; inside quotes the shell
            // keeps both, and either way it is not a boundary.
            current.push(character);
            escaped = false;
            continue;
        }

        match character {
            '\\' => escaped = true,
            '"' | '\'' => match quote {
                Some(open) if open == character => quote = None,
                Some(_) => current.push(character),
                None => quote = Some(character),
            },
            _ if quote.is_some() => current.push(character),
            // A separator outside quotes ends a word.
            ' ' | '\t' | '\n' | ';' | '|' | '&' | '(' | ')' | '`' => {
                if !current.is_empty() {
                    let word = std::mem::take(&mut current);
                    if starts_piped_segment {
                        piped.push(word.clone());
                    }
                    words.push(word);
                }
                // A word after a pipe is what the pipe's output is handed to, which is not always
                // one of the command's own words: `curl … | bash` runs bash, but not by name here.
                starts_piped_segment |= character == '|';
                loose.push(' ');
            }
            _ => {
                current.push(character);
                loose.push(character.to_ascii_lowercase());
            }
        }
    }

    if !current.is_empty() {
        if starts_piped_segment {
            piped.push(current.clone());
        }
        words.push(current);
    }

    Parsed { words, piped, loose }
}

fn is_shell(word: &str) -> bool {
    matches!(
        basename(word),
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "pwsh" | "powershell" | "cmd" | "eval" | "source"
    )
}

/// One rule: what it is called, how bad it is, and when it fires.
struct Rule {
    id: &'static str,
    label_key: &'static str,
    level: RiskLevel,
    when: fn(&Parsed) -> bool,
}

const RULE_UNREADABLE: Rule = Rule {
    id: "unreadable-shell",
    label_key: "risk.unreadableShell",
    level: RiskLevel::Caution,
    when: |_| false,
};

/// The rules, in no particular order: the level of an assessment is the worst of them.
const RULES: &[Rule] = &[
    Rule {
        id: "recursive-force-delete",
        label_key: "risk.recursiveForceDelete",
        level: RiskLevel::Destructive,
        when: |parsed| runs(parsed, "rm") && has_short_flag(parsed, 'r') && has_short_flag(parsed, 'f'),
    },
    Rule {
        id: "delete-everything",
        label_key: "risk.deleteEverything",
        level: RiskLevel::Catastrophic,
        when: |parsed| {
            runs(parsed, "rm") && has_short_flag(parsed, 'r') && parsed.words.iter().any(|word| is_broad_path(word))
        },
    },
    Rule {
        id: "filesystem-format",
        label_key: "risk.filesystemFormat",
        level: RiskLevel::Catastrophic,
        when: |parsed| {
            runs_any(parsed, &["mkfs", "mkfs.ext4", "mkfs.xfs", "fdisk", "parted", "sgdisk"])
                || parsed.loose.contains("mkfs.")
        },
    },
    Rule {
        id: "write-to-device",
        label_key: "risk.writeToDevice",
        level: RiskLevel::Catastrophic,
        when: |parsed| {
            parsed
                .words
                .iter()
                .any(|word| word.starts_with("of=/dev/") || word.starts_with(">/dev/sd"))
        },
    },
    Rule {
        id: "network-code-as-root",
        label_key: "risk.networkCodeAsRoot",
        level: RiskLevel::Catastrophic,
        when: |parsed| {
            // Downloading a script and handing it straight to a shell: whatever is on the other end
            // runs here, and no word of this command line says what it will be.
            (runs_any(parsed, &["curl", "wget"]) || parsed.loose.contains("curl ") || parsed.loose.contains("wget "))
                && parsed.piped.iter().any(|word| is_shell(word))
        },
    },
    Rule {
        id: "power-state",
        label_key: "risk.powerState",
        level: RiskLevel::Destructive,
        when: |parsed| runs_any(parsed, &["shutdown", "reboot", "halt", "poweroff", "init"]),
    },
    Rule {
        id: "firewall-flush",
        label_key: "risk.firewallFlush",
        level: RiskLevel::Destructive,
        when: |parsed| {
            (runs(parsed, "iptables") && parsed.words.iter().any(|word| word == "-F" || word == "--flush"))
                || parsed.loose.contains("nft flush ruleset")
                || (runs(parsed, "ufw") && parsed.words.iter().any(|word| word == "disable"))
        },
    },
    Rule {
        id: "database-drop",
        label_key: "risk.databaseDrop",
        level: RiskLevel::Destructive,
        when: |parsed| {
            parsed.loose.contains("drop database")
                || parsed.loose.contains("drop table")
                || runs(parsed, "dropdb")
                || parsed.loose.contains("flushall")
        },
    },
    Rule {
        id: "delete-files-by-find",
        label_key: "risk.deleteByFind",
        level: RiskLevel::Destructive,
        when: |parsed| runs(parsed, "find") && parsed.words.iter().any(|word| word == "-delete" || word == "-exec"),
    },
    Rule {
        id: "erase-logs",
        label_key: "risk.eraseLogs",
        level: RiskLevel::Destructive,
        when: |parsed| {
            parsed
                .words
                .iter()
                .any(|word| word.starts_with("/var/log") && !word.starts_with("-"))
                && (runs_any(parsed, &["rm", "truncate", "shred"]) || parsed.loose.contains("> /var/log"))
        },
    },
    Rule {
        id: "kill-everything",
        label_key: "risk.killEverything",
        level: RiskLevel::Destructive,
        when: |parsed| runs(parsed, "kill") && parsed.words.iter().any(|word| word == "-1") || runs(parsed, "killall5"),
    },
    Rule {
        id: "recursive-permissions",
        label_key: "risk.recursivePermissions",
        level: RiskLevel::Destructive,
        when: |parsed| {
            (runs(parsed, "chmod") || runs(parsed, "chown"))
                && parsed.words.iter().any(|word| word == "-R" || word == "--recursive")
                && parsed.words.iter().any(|word| is_broad_path(word))
        },
    },
    Rule {
        id: "permissions-wide-open",
        label_key: "risk.permissionsWideOpen",
        level: RiskLevel::Caution,
        when: |parsed| runs(parsed, "chmod") && parsed.words.iter().any(|word| word == "777" || word.ends_with("777")),
    },
    Rule {
        id: "privilege-elevation",
        label_key: "risk.privilegeElevation",
        level: RiskLevel::Caution,
        when: |parsed| mentions_any(parsed, &["sudo", "su", "doas", "runas"]),
    },
    Rule {
        id: "service-control",
        label_key: "risk.serviceControl",
        level: RiskLevel::Caution,
        when: |parsed| {
            (runs(parsed, "systemctl")
                && parsed
                    .words
                    .iter()
                    .any(|word| matches!(word.as_str(), "stop" | "disable" | "mask" | "kill")))
                || (runs(parsed, "service") && parsed.words.iter().any(|word| word == "stop"))
        },
    },
    Rule {
        id: "package-removal",
        label_key: "risk.packageRemoval",
        level: RiskLevel::Caution,
        when: |parsed| {
            (runs_any(parsed, &["apt", "apt-get", "yum", "dnf", "zypper", "pacman"])
                && parsed
                    .words
                    .iter()
                    .any(|word| matches!(word.as_str(), "remove" | "purge" | "autoremove")))
                || (runs(parsed, "rpm") && parsed.words.iter().any(|word| word == "-e"))
        },
    },
    Rule {
        id: "user-change",
        label_key: "risk.userChange",
        level: RiskLevel::Caution,
        when: |parsed| runs_any(parsed, &["userdel", "groupdel", "usermod", "chpasswd", "passwd"]),
    },
    Rule {
        id: "history-rewrite",
        label_key: "risk.historyRewrite",
        level: RiskLevel::Caution,
        when: |parsed| {
            (runs(parsed, "history") && parsed.words.iter().any(|word| word == "-c")) || runs(parsed, "shred")
        },
    },
    Rule {
        id: "scheduled-work-removal",
        label_key: "risk.scheduledWorkRemoval",
        level: RiskLevel::Caution,
        when: |parsed| runs(parsed, "crontab") && parsed.words.iter().any(|word| word == "-r"),
    },
    Rule {
        id: "version-control-destructive",
        label_key: "risk.versionControlDestructive",
        level: RiskLevel::Caution,
        when: |parsed| {
            (runs(parsed, "git")
                && parsed
                    .words
                    .iter()
                    .any(|word| matches!(word.as_str(), "clean" | "reset")))
                || parsed.loose.contains("push --force")
                || parsed.loose.contains("push -f ")
        },
    },
    Rule {
        id: "overwrite-file",
        label_key: "risk.overwriteFile",
        level: RiskLevel::Caution,
        when: |parsed| parsed.loose.contains("> ") && !parsed.loose.contains("> /dev/null"),
    },
];

/// Programs whose whole job is to run another program.
///
/// A rule about the program that will actually do the work has to look past these: `sudo rm -rf /`
/// deletes the same files as `rm -rf /`.
const WRAPPERS: &[&str] = &[
    "sudo", "doas", "su", "env", "nohup", "nice", "ionice", "stdbuf", "time", "timeout", "setsid", "command",
    "builtin", "exec",
];

/// The flags of one wrapper that take the next word as their value, so it is not read as the program.
///
/// Per wrapper rather than one shared list: `-p` takes a value for `sudo` (a prompt) but not for
/// `time` (POSIX output format), and guessing would either hide a real program or invent one.
fn wrapper_value_flags(wrapper: &str) -> &'static [&'static str] {
    match wrapper {
        "sudo" => &[
            "-u", "-g", "-p", "-C", "-h", "-r", "-t", "-U", "--user", "--group", "--host", "--prompt",
        ],
        "su" => &["-c", "-s", "--command", "--shell"],
        "doas" => &["-u", "-C"],
        "env" => &["-u", "-C", "-S", "--unset", "--chdir", "--split-string"],
        "nice" => &["-n", "--adjustment"],
        "ionice" => &["-c", "-n", "-p"],
        "stdbuf" => &["-i", "-o", "-e", "--input", "--output", "--error"],
        "time" => &["-o", "-f", "--output", "--format"],
        "timeout" => &["-s", "-k", "--signal", "--kill-after"],
        _ => &[],
    }
}

/// The program the command will run, with any wrappers and their arguments skipped.
///
/// `sudo -u root systemctl stop nginx` runs `systemctl` and `env APP_ENV=prod /opt/app/run` runs
/// `/opt/app/run`, so wrapper flags, their values and leading `NAME=value` assignments are stepped
/// over. A flag at the very start belongs to the program itself (`ls -la`, `awk -F: …`) — there is
/// no wrapper for it to belong to, so it is left alone.
fn program_word(parsed: &Parsed) -> Option<&str> {
    let mut index = 0;
    let mut wrapper: Option<&str> = None;

    loop {
        let word = parsed.words.get(index)?;

        if WRAPPERS.contains(&word.as_str()) {
            wrapper = Some(word);
            index += 1;
            continue;
        }

        if let Some(name) = wrapper {
            if word.starts_with('-') {
                // `--user=root` carries its own value, `-u root` does not.
                let takes_value = !word.contains('=') && wrapper_value_flags(name).contains(&word.as_str());
                index += if takes_value { 2 } else { 1 };
                continue;
            }
            if word.contains('=') && !word.starts_with('=') {
                index += 1;
                continue;
            }
        }

        return Some(word);
    }
}

/// True when the command's own program is this one.
///
/// The comparison is on the last path segment, so `/usr/bin/rm` is the same rule as `rm`.
fn runs(parsed: &Parsed, program: &str) -> bool {
    program_word(parsed).is_some_and(|word| basename(word) == program)
}

/// True when any word of the command is this program.
///
/// Used for rules about something being *present* rather than about what runs — `sudo` elevates no
/// matter which program follows it.
fn mentions(parsed: &Parsed, program: &str) -> bool {
    parsed.words.iter().any(|word| basename(word) == program)
}

fn mentions_any(parsed: &Parsed, programs: &[&str]) -> bool {
    programs.iter().any(|program| mentions(parsed, program))
}

fn basename(word: &str) -> &str {
    word.rsplit(['/', '\\']).next().unwrap_or(word)
}

fn runs_any(parsed: &Parsed, programs: &[&str]) -> bool {
    programs.iter().any(|program| runs(parsed, program))
}

/// True when a short flag was given, in any of the ways a shell accepts: `-rf`, `-r -f`, `-Rf`.
fn has_short_flag(parsed: &Parsed, flag: char) -> bool {
    let flag = flag.to_ascii_lowercase();
    parsed.words.iter().skip(1).any(|word| {
        word.starts_with('-')
            && !word.starts_with("--")
            && word.chars().skip(1).any(|letter| letter.to_ascii_lowercase() == flag)
    })
}

/// Whether a word names a place where "recursive" means "everything".
fn is_broad_path(word: &str) -> bool {
    matches!(word, "/" | "/*" | "/." | "~" | "~/." | "~/*" | "." | ".." | "./*") || word.ends_with("/*")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_read_is_safe() {
        let assessment = classify("ls -la /var/log");
        assert_eq!(assessment.level, RiskLevel::Safe);
        assert!(assessment.reasons.is_empty());
    }

    #[test]
    fn a_command_inside_quotes_is_not_that_command() {
        for command in [
            "echo \"rm -rf /\"",
            "printf 'mkfs.ext4 /dev/sda'",
            "grep -r 'drop database' notes.txt",
        ] {
            let assessment = classify(command);
            assert_eq!(assessment.level, RiskLevel::Safe, "{command} was read as if it ran");
        }
    }

    #[test]
    fn deleting_everything_is_the_worst_case() {
        let assessment = classify("rm -rf /");

        assert_eq!(assessment.level, RiskLevel::Catastrophic);
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.rule_id == "delete-everything")
        );
    }

    #[test]
    fn the_reasons_name_the_rules_that_fired() {
        let assessment = classify("sudo systemctl stop nginx");

        assert_eq!(assessment.level, RiskLevel::Caution);
        let ids: Vec<&str> = assessment.reasons.iter().map(|reason| reason.rule_id).collect();
        assert!(ids.contains(&"privilege-elevation"));
        assert!(ids.contains(&"service-control"));
        assert!(
            assessment
                .reasons
                .iter()
                .all(|reason| reason.label_key.starts_with("risk."))
        );
    }

    #[test]
    fn an_unreadable_shell_string_is_caution_rather_than_safe() {
        let assessment = classify("bash -c \"$MAINTENANCE_COMMAND\"");

        assert_eq!(assessment.level, RiskLevel::Caution);
        assert_eq!(assessment.reasons[0].rule_id, "unreadable-shell");
    }

    #[test]
    fn the_worst_rule_decides_the_level() {
        let assessment = classify("rm -rf /etc && sudo systemctl restart nginx");

        assert_eq!(assessment.level, RiskLevel::Destructive);
        assert!(assessment.needs_approval());
    }

    #[test]
    fn paths_are_read_to_the_last_segment() {
        assert_eq!(classify("/usr/bin/rm -rf /").level, RiskLevel::Catastrophic);
    }

    #[test]
    fn an_empty_command_is_nothing_at_all() {
        assert_eq!(classify("   ").level, RiskLevel::Safe);
    }
}

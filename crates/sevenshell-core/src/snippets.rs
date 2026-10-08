//! Saved commands, and sending one to the terminals that are open.
//!
//! Two things here are decisions rather than plumbing.
//!
//! **What "send" means.** A command is written into a *shell that is already running* — the bytes
//! are the same bytes somebody would type — which is why there is no connection to open, no exit
//! status to read, and no session needed: only an open terminal has a shell to type into. Running
//! a command with no terminal, and reading each host's answer, is the batch path and stays there
//! ([`Core::run_across`](crate::Core::run_across)).
//!
//! **Where the gate is.** Sending to *one* terminal is deliberately not gated: a person is looking
//! at the shell they are typing into, which is the same reason the terminal itself is not gated.
//! Sending to several at once is different — nobody reads ten shells at the same time — so a
//! command whose level needs approval waits for one, in the core, where a caller that never draws
//! a button (an agent, later) meets it too.

use sevenshell_error::ShellError;
use sevenshell_types::{
    CommandHistoryEntry, RiskAssessment, SendFailure, SendOutcome, SendReport, SendTarget, Snippet, SnippetGroup,
};

use crate::Core;
use crate::sessions::now_seconds;

impl Core {
    /// The saved commands, grouped, in the order the interface lists them.
    pub fn snippet_groups(&self) -> Result<Vec<SnippetGroup>, ShellError> {
        self.vault.snippet_groups()
    }

    pub fn save_snippet_group(&self, group: SnippetGroup) -> Result<SnippetGroup, ShellError> {
        self.vault.save_snippet_group(group)
    }

    pub fn delete_snippet_group(&self, id: &str) -> Result<(), ShellError> {
        self.vault.delete_snippet_group(id)
    }

    /// Adds or updates a saved command, and answers with what was stored.
    ///
    /// The answer matters: a command saved with an empty id comes back with the id it was given,
    /// and that is the only handle the interface will have for it.
    pub fn save_snippet(&self, snippet: Snippet) -> Result<Snippet, ShellError> {
        self.vault.save_snippet(snippet)
    }

    pub fn delete_snippet(&self, id: &str) -> Result<(), ShellError> {
        self.vault.delete_snippet(id)
    }

    /// What was sent, newest first.
    pub fn command_history(&self) -> Result<Vec<CommandHistoryEntry>, ShellError> {
        self.vault.command_history()
    }

    pub fn clear_command_history(&self) -> Result<(), ShellError> {
        self.vault.clear_command_history()
    }

    /// Writes a line into the terminals in the target, pressing Enter afterwards when asked.
    ///
    /// A terminal that cannot be written to is one entry in the report rather than a failed send:
    /// "it reached the other three" is the useful answer when one tab has already ended.
    pub async fn send_command(
        &self,
        target: SendTarget,
        command: &str,
        append_cr: bool,
        acknowledged: bool,
    ) -> Result<SendOutcome, ShellError> {
        let SendTarget::Terminals { handles } = target;

        let risk = sevenshell_policy::classify(command);
        if handles.len() > 1 && risk.needs_approval() && !acknowledged {
            return Ok(SendOutcome::ApprovalRequired { risk });
        }

        // A pty expects the carriage return a keyboard would send, not a line feed: the terminal
        // driver translates it, and a bare `\n` shows up as a line that never runs.
        let mut line = command.to_owned();
        if append_cr {
            line.push('\r');
        }

        let mut sent = Vec::new();
        let mut failures = Vec::new();
        for handle in &handles {
            match self.registry.write(handle, line.as_bytes()).await {
                Ok(()) => sent.push(handle.clone()),
                Err(error) => failures.push(send_failure(handle, &error)),
            }
        }

        // Only what actually reached a shell is remembered: a command that was refused everywhere
        // was not sent, and offering it back as history would say it was.
        if !sent.is_empty() {
            self.vault.remember_command(command, now_seconds())?;
        }

        Ok(SendOutcome::Sent {
            report: SendReport { sent, failures },
        })
    }
}

/// One terminal a command could not be sent to.
///
/// The same subset of the envelope the batch rows carry, and for the same reason: the wire types
/// cannot name the error type, so the parts a message needs travel as themselves.
fn send_failure(handle: &str, error: &ShellError) -> SendFailure {
    SendFailure {
        handle: handle.to_owned(),
        code: error.code().as_str().to_owned(),
        message_key: error.message_key().to_owned(),
        message_params: error.message_params().clone(),
        detail: error.detail().map(str::to_owned),
    }
}

/// The verdict for a command, without sending it.
///
/// The interface asks for this as somebody types, so the warning is on screen before the button is
/// pressed rather than after — the same answer the send itself would give.
pub fn classify_send(command: &str) -> RiskAssessment {
    sevenshell_policy::classify(command)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sevenshell_types::DEFAULT_GROUP_ID;
    use sevenshell_vault::Vault;
    use tempfile::{TempDir, tempdir};

    use super::*;

    fn vault() -> (TempDir, Arc<Vault>) {
        let directory = tempdir().expect("temp dir");
        let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
        vault.create("master").expect("create");
        (directory, vault)
    }

    #[tokio::test]
    async fn one_terminal_is_never_asked_about() {
        let (_directory, vault) = vault();
        let (core, _events) = Core::new(vault);

        // A handle that does not exist: the answer is a report with a failure, not a verdict —
        // because a person sending to the one shell they are looking at is the person deciding.
        let outcome = core
            .send_command(
                SendTarget::Terminals {
                    handles: vec!["terminal-that-never-existed".to_owned()],
                },
                "rm -rf /srv/app",
                true,
                false,
            )
            .await
            .expect("a send is not an error");

        let SendOutcome::Sent { report } = outcome else {
            panic!("one terminal must not raise the approval gate, got {outcome:?}");
        };
        assert!(report.sent.is_empty());
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].code, "SESSION-NOT-FOUND");
    }

    #[tokio::test]
    async fn several_terminals_wait_for_a_person_when_the_command_needs_one() {
        let (_directory, vault) = vault();
        let (core, _events) = Core::new(vault);

        let outcome = core
            .send_command(
                SendTarget::Terminals {
                    handles: vec!["one".to_owned(), "two".to_owned()],
                },
                "rm -rf /srv/app",
                true,
                false,
            )
            .await
            .expect("waiting for approval is not an error");

        match outcome {
            SendOutcome::ApprovalRequired { risk } => {
                assert!(risk.needs_approval());
                assert!(
                    risk.reasons
                        .iter()
                        .any(|reason| reason.rule_id == "recursive-force-delete")
                );
            }
            other => panic!("expected the gate to hold several terminals, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_command_that_reached_no_shell_is_not_remembered() {
        let (_directory, vault) = vault();
        let (core, _events) = Core::new(vault);

        core.send_command(
            SendTarget::Terminals {
                handles: vec!["terminal-that-never-existed".to_owned()],
            },
            "uptime",
            true,
            true,
        )
        .await
        .expect("a send is not an error");

        assert!(
            core.command_history().expect("read").is_empty(),
            "offering back a command that was never typed anywhere would say it was"
        );
    }

    #[test]
    fn the_library_round_trips_through_the_core() {
        let (_directory, vault) = vault();
        let (core, _events) = Core::new(vault);

        let saved = core
            .save_snippet(Snippet {
                id: String::new(),
                group_id: DEFAULT_GROUP_ID.to_owned(),
                name: "uptime".to_owned(),
                prefix: String::new(),
                command: "uptime".to_owned(),
                append_cr: true,
                order: 0,
                slot_history: Vec::new(),
            })
            .expect("save");

        let groups = core.snippet_groups().expect("read");
        let default = groups
            .iter()
            .find(|group| group.id == DEFAULT_GROUP_ID)
            .expect("default");
        assert_eq!(default.commands.len(), 1);
        assert_eq!(default.commands[0].id, saved.id);

        core.delete_snippet(&saved.id).expect("delete");
        let groups = core.snippet_groups().expect("read");
        assert!(groups.iter().all(|group| group.commands.is_empty()));
    }
}

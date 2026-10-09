//! The decrypted content of a vault, and the rules it holds itself to.
//!
//! A `Document` is the plaintext behind the envelope: the sessions, the groups that order them,
//! the imported keys, the per-session secrets and the trusted host keys. Three rules live here
//! rather than in the caller:
//!
//! * **A stored secret is wiped when it is dropped.** Key material and session secrets are held
//!   in [`SecretString`], so locking the vault really does erase them instead of leaving them
//!   for the allocator to hand to the next allocation.
//! * **A secret is never printed.** Nothing here derives `Debug`; the manual implementations
//!   count what is stored, they never show it.
//! * **Absent means unchanged.** Saving a session without a secret keeps whatever secret that
//!   session already had — the interface never has to send a password back to leave it alone.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::{
    AuthMethod, CommandHistoryEntry, CompletionWord, DEFAULT_GROUP_ID, KeyInfo, KnownHostEntry, Session, SessionGroup,
    Snippet, SnippetGroup,
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::credential::{KeyEntry, SecretEntry, SecretKind, ServicePassword};

/// Bumped only when the stored layout changes in a way older builds cannot read.
///
/// Version 2 added the command library and its history. Version 3 added a forward's mode: a build
/// that predates it can parse the document, ignore the field and write the forward back as a
/// *local* one — a tunnel to the wrong place rather than a missing line. Version 4 added a session's
/// agent forwarding, which such a build would drop on its next write — a setting changed without
/// anybody saying so, in the direction of "off", which is the safe way round but still not a thing
/// to do quietly. Version 5 added session recording, for the same reason. Version 6 added the words
/// somebody adds to what follows a command: a build that predates it would parse the document, drop
/// the list and write it back empty. Version 7 added the password for a sync server — a value an
/// older build would drop on its next write, which would look exactly like "the password was
/// forgotten". Version 8 added the access tokens for code-hosting accounts, which an older build
/// would drop in exactly the same way: the token that was kept would simply be gone.
pub const VAULT_SCHEMA_VERSION: u32 = 8;

/// How many sent commands are remembered. Enough to cover a working session's worth of "what was
/// that line", and bounded so the vault does not grow with every command ever typed.
pub const COMMAND_HISTORY_LIMIT: usize = 500;

/// Everything a vault holds, as it is serialized into the encrypted payload.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub schema_version: u32,
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub groups: Vec<SessionGroup>,
    #[serde(default)]
    pub keys: Vec<StoredKey>,
    #[serde(default)]
    pub secrets: Vec<StoredSecret>,
    #[serde(default)]
    pub known_hosts: Vec<KnownHostEntry>,
    /// Commands kept for later, grouped. The default group exists from the first write, because a
    /// command needs somewhere to live.
    #[serde(default)]
    pub snippet_groups: Vec<SnippetGroup>,
    /// What was sent, newest first.
    #[serde(default)]
    pub command_history: Vec<CommandHistoryEntry>,
    /// The words somebody added to what follows a command, in the order they added them.
    #[serde(default)]
    pub completion_words: Vec<CompletionWord>,
    /// The password for the sync server, when somebody asked for it to be kept.
    ///
    /// Here rather than in the settings file because it is a password, and the settings file is
    /// read by anything that can read the configuration directory. The server's *address* and user
    /// name are not secrets and live there instead — this is the one part of a sync configuration
    /// that has to be behind the vault's own lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync_password: Option<SecretString>,
    /// The passphrase an uploaded snapshot is sealed under, when somebody asked for it to be kept.
    ///
    /// Kept because a timed upload cannot ask: a timer that stops to prompt is a timer nobody has
    /// set. Optional, and independent of the password above — a snapshot can be sealed without the
    /// server needing anything but a login, and a server can need a login without anything being
    /// sealed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync_passphrase: Option<SecretString>,
    /// The access token kept for each code-hosting account, by account name.
    ///
    /// A table rather than a field per service, because the accounts are keyed by more than the
    /// service: two GitLab installations are two accounts with two tokens, and a build that has to
    /// learn a third service should not need the vault to change shape for it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    snippet_tokens: BTreeMap<String, SecretString>,
}

impl Document {
    /// An empty document at the current schema version.
    pub(crate) fn empty() -> Self {
        Self {
            schema_version: VAULT_SCHEMA_VERSION,
            sessions: Vec::new(),
            groups: Vec::new(),
            keys: Vec::new(),
            secrets: Vec::new(),
            known_hosts: Vec::new(),
            snippet_groups: vec![default_group()],
            command_history: Vec::new(),
            completion_words: Vec::new(),
            sync_password: None,
            sync_passphrase: None,
            snippet_tokens: BTreeMap::new(),
        }
    }

    /// The sessions, with `has_secret` set from the secrets table rather than from whatever
    /// the caller last wrote. The table is the authority; the flag is a report about it.
    pub(crate) fn sessions(&self) -> Vec<Session> {
        let mut sessions = self.sessions.clone();
        for session in &mut sessions {
            session.auth.has_secret = self.stores_secret_for(&session.id);
        }
        sessions
    }

    /// Adds or replaces a session, and its secret when one is given.
    ///
    /// `secret: None` leaves an existing secret alone; there is no way to say "delete the
    /// secret" here, because the interface cannot tell the difference between "the user did
    /// not change it" and "the user cleared it" — and of the two readings, keeping the
    /// password is the one that cannot lose data.
    pub(crate) fn save_session(&mut self, mut session: Session, secret: Option<SessionSecret>) -> Session {
        if let Some(secret) = &secret {
            let value = SecretString::new(secret.expose().to_owned());
            match self.secrets.iter_mut().find(|stored| stored.session_id == session.id) {
                Some(stored) => stored.secret = value,
                None => self.secrets.push(StoredSecret {
                    session_id: session.id.clone(),
                    secret: value,
                }),
            }
        }

        session.auth.has_secret = self.stores_secret_for(&session.id);
        match self.sessions.iter_mut().find(|existing| existing.id == session.id) {
            Some(existing) => *existing = session.clone(),
            None => self.sessions.push(session.clone()),
        }
        session
    }

    /// Removes a session and the secret that belonged to it. Removing something already absent
    /// is not an error: the world ends up in the state the caller asked for either way.
    pub(crate) fn delete_session(&mut self, id: &str) {
        self.sessions.retain(|session| session.id != id);
        self.secrets.retain(|secret| secret.session_id != id);
    }

    pub(crate) fn save_group(&mut self, group: SessionGroup) -> SessionGroup {
        match self.groups.iter_mut().find(|existing| existing.id == group.id) {
            Some(existing) => *existing = group.clone(),
            None => self.groups.push(group.clone()),
        }
        group
    }

    /// Removes a group. Sessions and child groups that pointed at it fall back to the root of
    /// the tree: deleting a folder must never delete what was inside it.
    pub(crate) fn delete_group(&mut self, id: &str) {
        self.groups.retain(|group| group.id != id);
        for session in &mut self.sessions {
            if session.group_id.as_deref() == Some(id) {
                session.group_id = None;
            }
        }
        for group in &mut self.groups {
            if group.parent_id.as_deref() == Some(id) {
                group.parent_id = None;
            }
        }
    }

    /// The trusted key for exactly this endpoint, if one has been recorded.
    pub(crate) fn find_known_host(&self, host: &str, port: u16) -> Option<KnownHostEntry> {
        self.known_hosts
            .iter()
            .find(|entry| entry.host == host && entry.port == port)
            .cloned()
    }

    /// Records a trusted key for an endpoint.
    ///
    /// One entry per host and port: the comparison the SSH layer makes is against *the* key it
    /// expected, and a rotated key is recorded by replacing the old one. This is also the only
    /// way a user accepts a legitimately changed key.
    pub(crate) fn trust_host(&mut self, entry: KnownHostEntry) {
        self.known_hosts
            .retain(|existing| !(existing.host == entry.host && existing.port == entry.port));
        self.known_hosts.push(entry);
        self.known_hosts
            .sort_by(|a, b| a.host.cmp(&b.host).then(a.port.cmp(&b.port)));
    }

    /// Forgets every key recorded for an endpoint.
    pub(crate) fn forget_host(&mut self, host: &str, port: u16) {
        self.known_hosts
            .retain(|entry| !(entry.host == host && entry.port == port));
    }

    /// The command library, in the order the interface lists it.
    pub(crate) fn snippet_groups(&self) -> Vec<SnippetGroup> {
        let mut groups = self.snippet_groups.clone();
        groups.sort_by(|a, b| a.order.cmp(&b.order).then(a.name.cmp(&b.name)));
        for group in &mut groups {
            group
                .commands
                .sort_by(|a, b| a.order.cmp(&b.order).then(a.name.cmp(&b.name)));
        }
        groups
    }

    /// Adds or renames a group, and answers with what was stored.
    ///
    /// A group with no id is a new one: it gets an id and lands at the end of the list.
    pub(crate) fn save_snippet_group(&mut self, mut group: SnippetGroup) -> SnippetGroup {
        if group.id.is_empty() {
            group.id = format!("snippet-group-{}", Uuid::new_v4());
        }
        if group.id == DEFAULT_GROUP_ID {
            // The default group is found rather than made, and its label is a translation rather
            // than a name — so a rename here would be overwritten by the next interface that reads
            // it, and storing one would be storing a lie.
            group.name = String::new();
        }

        match self.snippet_groups.iter_mut().find(|existing| existing.id == group.id) {
            Some(existing) => {
                // The commands inside a group are not part of a rename: they are saved one at a
                // time, and taking them from here would let a stale list delete the newest ones.
                existing.name = group.name.clone();
                existing.order = group.order;
                existing.clone()
            }
            None => {
                group.order = self.next_group_order();
                group.commands.clear();
                self.snippet_groups.push(group.clone());
                group
            }
        }
    }

    /// Removes a group and the commands in it.
    ///
    /// Unlike a session group, what is inside goes with it: a command belongs to its group the way
    /// a line belongs to the file it was written in, and "the folder is gone but its contents are
    /// somewhere" would be a folder that only looks empty.
    pub(crate) fn delete_snippet_group(&mut self, id: &str) -> Result<(), ShellError> {
        if id == DEFAULT_GROUP_ID {
            return Err(ShellError::app_unsupported("deleting the default command group"));
        }
        self.snippet_groups.retain(|group| group.id != id);
        Ok(())
    }

    /// Adds or updates a saved command.
    ///
    /// A command whose group is gone lands in the default group rather than vanishing with it: the
    /// alternative is a save that silently does nothing.
    pub(crate) fn save_snippet(&mut self, mut snippet: Snippet) -> Snippet {
        if snippet.id.is_empty() {
            snippet.id = format!("snippet-{}", Uuid::new_v4());
        }

        let group_id = if self.snippet_groups.iter().any(|group| group.id == snippet.group_id) {
            snippet.group_id.clone()
        } else {
            DEFAULT_GROUP_ID.to_owned()
        };
        snippet.group_id = group_id.clone();

        let group = self
            .snippet_groups
            .iter_mut()
            .find(|group| group.id == group_id)
            .expect("the default group is always there");

        match group.commands.iter_mut().find(|existing| existing.id == snippet.id) {
            Some(existing) => {
                // The whole command, taken as it arrived: a field copied one at a time is a field that
                // can be forgotten here, and a forgotten one is a field nobody can edit — the trigger
                // and the switch were both exactly that until an edit of each was tested.
                //
                // The order is the one exception, because it is not part of an edit: zero means "leave
                // it where it is", so a caller changing a command's text has no opinion about its place
                // in the list, and moving one is the only caller that passes a real order.
                let order = existing.order;
                *existing = snippet;
                if existing.order == 0 {
                    existing.order = order;
                }
                existing.clone()
            }
            None => {
                snippet.order = group.commands.iter().map(|command| command.order).max().unwrap_or(0) + 1;
                group.commands.push(snippet.clone());
                snippet
            }
        }
    }

    pub(crate) fn delete_snippet(&mut self, id: &str) {
        for group in &mut self.snippet_groups {
            group.commands.retain(|command| command.id != id);
        }
    }

    /// The words somebody added to what follows a command.
    ///
    /// In the order they were added, because that is the only order there is: nothing drags these
    /// around, and all the order decides is which of two equally good words is offered first.
    pub(crate) fn completion_words(&self) -> Vec<CompletionWord> {
        self.completion_words.clone()
    }

    /// Adds or updates one word. A word with no id is a new one.
    ///
    /// The whole word is taken as it arrived: a field copied one at a time is a field that can be
    /// forgotten here, and a forgotten one is a field nobody can edit.
    ///
    /// The pair of command and word is what a row *is*, so saving a pair that is already there
    /// changes that row rather than adding a second one — two rows saying `kubectl apply` would be
    /// two rows somebody has to work out between.
    pub(crate) fn save_completion_word(&mut self, mut word: CompletionWord) -> CompletionWord {
        if word.id.is_empty() {
            word.id = format!("word-{}", Uuid::new_v4());
        }

        let existing = self
            .completion_words
            .iter()
            .position(|entry| entry.id == word.id || (entry.command == word.command && entry.word == word.word));

        match existing {
            Some(index) => {
                // The row keeps the id the interface is already holding, and its place in the list:
                // changing what a word says is not the same as moving it to the end.
                word.id = self.completion_words[index].id.clone();
                self.completion_words[index] = word.clone();
                word
            }
            None => {
                self.completion_words.push(word.clone());
                word
            }
        }
    }

    pub(crate) fn delete_completion_word(&mut self, id: &str) {
        self.completion_words.retain(|word| word.id != id);
    }

    /// Forgets every word somebody added, which is what restoring the defaults means for this list:
    /// what this build ships is not stored anywhere to put back.
    pub(crate) fn clear_completion_words(&mut self) {
        self.completion_words.clear();
    }

    /// What was sent, newest first.
    pub(crate) fn command_history(&self) -> Vec<CommandHistoryEntry> {
        self.command_history.clone()
    }

    /// Remembers a command that was sent.
    ///
    /// Sending the same line again moves it to the front rather than adding a second copy: the list
    /// is read by scanning for a line, and five identical rows push the lines that were actually
    /// different off the end.
    pub(crate) fn remember_command(&mut self, command: &str, at: u64) {
        let command = command.trim();
        if command.is_empty() {
            return;
        }

        self.command_history.retain(|entry| entry.command != command);
        self.command_history.insert(
            0,
            CommandHistoryEntry {
                command: command.to_owned(),
                at,
            },
        );
        self.command_history.truncate(COMMAND_HISTORY_LIMIT);
    }

    pub(crate) fn clear_command_history(&mut self) {
        self.command_history.clear();
    }

    /// Adds the default group when a document predates it.
    pub(crate) fn ensure_default_group(&mut self) {
        if self.snippet_groups.iter().all(|group| group.id != DEFAULT_GROUP_ID) {
            self.snippet_groups.insert(0, default_group());
        }
    }

    fn next_group_order(&self) -> u32 {
        self.snippet_groups.iter().map(|group| group.order).max().unwrap_or(0) + 1
    }

    /// The imported keys, as the interface may know them.
    ///
    /// [`KeyInfo`] and nothing else. What a `StoredKey` keeps private is the material and the
    /// passphrase; a list that carried either would be a secret leaving the vault in a shape
    /// nobody expects to have to check.
    pub(crate) fn keys(&self) -> Vec<KeyInfo> {
        self.keys.iter().map(|stored| stored.info.clone()).collect()
    }

    /// Stores key material and returns the `KeyInfo` the interface should remember: the handle
    /// the vault assigned, and the truth about whether a passphrase is held for it.
    pub(crate) fn import_key(
        &mut self,
        contents: String,
        passphrase: Option<String>,
        info: KeyInfo,
    ) -> Result<KeyInfo, ShellError> {
        self.store_key(format!("key-{}", Uuid::new_v4()), contents, passphrase, info)
    }

    /// Stores key material under a handle it was given rather than a fresh one.
    ///
    /// The import path makes a handle because nobody has seen this key before. A file that carries
    /// its own sessions is the other case: they point at the key by the handle it arrived with, and
    /// a fresh one would leave every one of them pointing at nothing.
    pub(crate) fn import_key_as(
        &mut self,
        key_ref: String,
        contents: String,
        passphrase: Option<String>,
        info: KeyInfo,
    ) -> Result<KeyInfo, ShellError> {
        self.store_key(key_ref, contents, passphrase, info)
    }

    /// The one place a key is stored, so the two entry points cannot come to differ about what
    /// storing one means.
    fn store_key(
        &mut self,
        key_ref: String,
        contents: String,
        passphrase: Option<String>,
        mut info: KeyInfo,
    ) -> Result<KeyInfo, ShellError> {
        if contents.trim().is_empty() {
            return Err(ShellError::ssh_key_invalid("the key file is empty"));
        }

        info.key_ref = Some(key_ref.clone());
        info.has_passphrase = passphrase.is_some();
        self.keys.push(StoredKey {
            key_ref,
            info: info.clone(),
            contents: SecretString::new(contents),
            passphrase: passphrase.map(SecretString::new),
        });
        Ok(info)
    }

    pub(crate) fn key_material(&self, key_ref: &str) -> Option<KeyMaterial> {
        let stored = self.keys.iter().find(|key| key.key_ref == key_ref)?;
        Some(KeyMaterial {
            contents: stored.contents.expose().to_owned(),
            passphrase: stored
                .passphrase
                .as_ref()
                .map(|passphrase| passphrase.expose().to_owned()),
        })
    }

    /// Removes imported key material. Removing something already absent is not an error, for
    /// the same reason deleting a session is not.
    pub(crate) fn forget_key(&mut self, key_ref: &str) {
        self.keys.retain(|key| key.key_ref != key_ref);
    }

    /// The secret stored for a session.
    ///
    /// The kind is not stored next to the value — the format keeps a secret and its session id
    /// and nothing else, so that a stray file listing cannot say what a value is for. It is
    /// derived from the session's authentication method instead: a key-based session holds a
    /// passphrase, anything else holds a password.
    pub(crate) fn secret_for_session(&self, id: &str) -> Option<SessionSecret> {
        let stored = self.secrets.iter().find(|secret| secret.session_id == id)?;
        let method = self
            .sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.auth.method);

        Some(match method {
            Some(AuthMethod::PublicKey) => SessionSecret::KeyPassphrase(stored.secret.expose().to_owned()),
            _ => SessionSecret::Password(stored.secret.expose().to_owned()),
        })
    }

    fn stores_secret_for(&self, session_id: &str) -> bool {
        self.secrets.iter().any(|secret| secret.session_id == session_id)
    }

    /// Every stored secret, in the shape a file takes.
    ///
    /// The kind comes from the session's authentication method, and not from the stored row, for
    /// the reason [`Document::secret_for_session`] gives: the row holds an id and a value, and
    /// nothing that says what the value is for. A secret whose session is gone is still listed —
    /// it is a value somebody stored, and dropping it here would be this listing deciding to lose
    /// something.
    pub(crate) fn secret_entries(&self) -> Vec<SecretEntry> {
        self.secrets
            .iter()
            .map(|stored| SecretEntry {
                session_id: stored.session_id.clone(),
                kind: match self
                    .sessions
                    .iter()
                    .find(|session| session.id == stored.session_id)
                    .map(|session| session.auth.method)
                {
                    Some(AuthMethod::PublicKey) => SecretKind::KeyPassphrase,
                    _ => SecretKind::Password,
                },
                value: stored.secret.expose().to_owned(),
            })
            .collect()
    }

    /// Every imported key, material and all, in the shape a file takes.
    pub(crate) fn key_entries(&self) -> Vec<KeyEntry> {
        self.keys
            .iter()
            .map(|stored| KeyEntry {
                key_ref: stored.key_ref.clone(),
                info: stored.info.clone(),
                contents: stored.contents.expose().to_owned(),
                passphrase: stored
                    .passphrase
                    .as_ref()
                    .map(|passphrase| passphrase.expose().to_owned()),
            })
            .collect()
    }

    /// The password kept for the sync server, if one was.
    pub(crate) fn sync_password(&self) -> Option<ServicePassword> {
        self.sync_password
            .as_ref()
            .map(|password| ServicePassword::new(password.expose().to_owned()))
    }

    /// Keeps a password for the sync server, or forgets the one that was kept.
    ///
    /// `None` and an empty string are the same answer — nobody wants to keep an empty password —
    /// so forgetting is one call rather than a separate path that could disagree with this one.
    pub(crate) fn set_sync_password(&mut self, password: Option<String>) {
        self.sync_password = password.filter(|value| !value.is_empty()).map(SecretString::new);
    }

    /// The passphrase an uploaded snapshot is sealed under, if one was kept.
    ///
    /// Not the same thing as the server's password, and kept apart on purpose: the server never
    /// sees this one, and it is the only thing standing between a file on somebody else's disk and
    /// the logins in it. A password that opens the door and a passphrase that seals the parcel are
    /// two decisions, and one of them may well be shared with a colleague.
    pub(crate) fn sync_passphrase(&self) -> Option<ServicePassword> {
        self.sync_passphrase
            .as_ref()
            .map(|passphrase| ServicePassword::new(passphrase.expose().to_owned()))
    }

    pub(crate) fn set_sync_passphrase(&mut self, passphrase: Option<String>) {
        self.sync_passphrase = passphrase.filter(|value| !value.is_empty()).map(SecretString::new);
    }

    /// The token kept for one code-hosting account, if one was.
    ///
    /// A token per account rather than one for the feature, because the accounts are not
    /// interchangeable: a GitHub token means nothing to Gitee, and somebody who has both should not
    /// have to type the one they are not using back in. The key is the account — what names it is
    /// the caller's business, not the vault's, since this is a table of values it never interprets.
    pub(crate) fn snippet_token(&self, account: &str) -> Option<ServicePassword> {
        self.snippet_tokens
            .get(account)
            .map(|token| ServicePassword::new(token.expose().to_owned()))
    }

    /// Keeps a token for one account, or forgets the one that was kept.
    ///
    /// `None` and an empty string are the same answer, like the sync password above: nobody wants to
    /// keep an empty token, and one path for forgetting cannot disagree with itself.
    pub(crate) fn set_snippet_token(&mut self, account: &str, token: Option<String>) {
        match token.filter(|value| !value.trim().is_empty()) {
            Some(token) => {
                self.snippet_tokens.insert(account.to_owned(), SecretString::new(token));
            }
            None => {
                self.snippet_tokens.remove(account);
            }
        }
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Counts only. A document is where every secret lives; a derived `Debug` here would put
        // them one `{:?}` away from a log line. The accounts with tokens are named rather than
        // counted, because which services have one is the whole of what a reader could want from
        // this line and none of it is a secret — an account name is a provider and an address.
        f.debug_struct("Document")
            .field("schema_version", &self.schema_version)
            .field("sessions", &self.sessions.len())
            .field("groups", &self.groups.len())
            .field("keys", &self.keys.len())
            .field("secrets", &self.secrets.len())
            .field("known_hosts", &self.known_hosts.len())
            .field("sync_password", &self.sync_password.is_some())
            .field("sync_passphrase", &self.sync_passphrase.is_some())
            .field("snippet_tokens", &self.snippet_tokens.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// An imported key as it is stored: its handle, what the interface knows about it, and the
/// material itself.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKey {
    pub key_ref: String,
    pub info: KeyInfo,
    contents: SecretString,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passphrase: Option<SecretString>,
}

/// A secret stored for one session, keyed by the session it belongs to.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSecret {
    pub session_id: String,
    secret: SecretString,
}

/// Key material on its way out of the vault.
///
/// Wiped when it is dropped, and never printed. The fields are public because the caller owns
/// this value and reads it once, but note that the `Drop` implementation is what keeps it from
/// lingering: formatting one is impossible, by construction.
#[derive(Clone)]
pub struct KeyMaterial {
    pub contents: String,
    pub passphrase: Option<String>,
}

impl KeyMaterial {
    /// Takes the material apart so it can be handed to a transport, which zeroizes it when done.
    ///
    /// Consuming `self` keeps one owner for the secret. Cloning it would leave a second copy in
    /// memory for the allocator to hand to whoever asks next.
    pub fn into_parts(mut self) -> (String, Option<String>) {
        (std::mem::take(&mut self.contents), self.passphrase.take())
    }
}

impl Drop for KeyMaterial {
    fn drop(&mut self) {
        self.contents.zeroize();
        if let Some(passphrase) = &mut self.passphrase {
            passphrase.zeroize();
        }
    }
}

impl fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyMaterial(<redacted>)")
    }
}

/// What a session needs in order to authenticate: a login password, or the passphrase of a key
/// file it points at.
///
/// Wiped when it is dropped, and never printed.
#[derive(Clone)]
pub enum SessionSecret {
    Password(String),
    KeyPassphrase(String),
}

impl SessionSecret {
    /// The value, for the one caller that has to hand it to a connection.
    pub(crate) fn expose(&self) -> &str {
        match self {
            Self::Password(value) | Self::KeyPassphrase(value) => value,
        }
    }

    /// Moves the value out so a connection can own it, leaving nothing behind to wipe here.
    ///
    /// The caller becomes responsible for the secret: whatever receives it must zeroize it, which
    /// `sevenshell_ssh::Credential` does.
    pub fn into_value(mut self) -> String {
        match &mut self {
            Self::Password(value) | Self::KeyPassphrase(value) => std::mem::take(value),
        }
    }
}

impl Drop for SessionSecret {
    fn drop(&mut self) {
        match self {
            Self::Password(value) | Self::KeyPassphrase(value) => value.zeroize(),
        }
    }
}

impl fmt::Debug for SessionSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Which kind of secret it is is safe to say; what it is, is not.
        let kind = match self {
            Self::Password(_) => "Password",
            Self::KeyPassphrase(_) => "KeyPassphrase",
        };
        write!(f, "SessionSecret::{kind}(<redacted>)")
    }
}

/// A string inside a document that is wiped on drop and can never be printed.
#[derive(Clone, Default)]
struct SecretString(Zeroizing<String>);

impl SecretString {
    fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Serialize for SecretString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // The deserialized allocation is moved straight into the zeroizing wrapper, so no
        // un-wiped copy of a secret is left behind for the allocator to hand out.
        String::deserialize(deserializer).map(Self::new)
    }
}

/// Brings a document written by an older build up to [`VAULT_SCHEMA_VERSION`].
///
/// Brings a stored document up to the shape this build reads.
///
/// A document from a *newer* build is refused instead of guessed at: silently dropping fields a
/// future version needs is how a vault loses data.
pub(crate) fn migrate(mut document: Document) -> Result<Document, ShellError> {
    if document.schema_version > VAULT_SCHEMA_VERSION {
        return Err(ShellError::store_schema_newer(
            document.schema_version,
            VAULT_SCHEMA_VERSION,
        ));
    }

    // 1 → 2: the command library arrived. An older document simply has none, and gets the group
    // the first saved command will need.
    if document.schema_version == 1 {
        document.ensure_default_group();
    }

    // 2 → 3: forwards gained a mode. 3 → 4: a session gained agent forwarding. 4 → 5: a session
    // gained recording. 5 → 6: a document gained the list of words somebody added, which an older one
    // simply has none of. None of them has anything to rewrite — the defaults are what every document
    // meant — so the steps are the version number alone.
    document.schema_version = VAULT_SCHEMA_VERSION;

    Ok(document)
}

/// The group every command has somewhere to land in.
fn default_group() -> SnippetGroup {
    SnippetGroup {
        id: DEFAULT_GROUP_ID.to_owned(),
        // Empty on purpose: this is the one group whose label is a translation, and a stored name
        // would be frozen in whatever language was active when the vault was made.
        name: String::new(),
        order: 0,
        commands: Vec::new(),
    }
}

/// Parses a decrypted payload. A payload that decrypted correctly but does not parse was
/// written by something that is not this format, and is reported as an unlock failure so the
/// two cannot be told apart from outside.
pub(crate) fn from_slice(payload: &[u8]) -> Result<Document, ShellError> {
    let document = serde_json::from_slice::<Document>(payload).map_err(|_| ShellError::vault_unlock_failed())?;
    migrate(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sevenshell_types::{SessionAuth, TerminalSettings};

    fn session(id: &str, name: &str) -> Session {
        Session {
            id: id.to_owned(),
            name: name.to_owned(),
            group_id: None,
            protocol: sevenshell_types::Protocol::Ssh,
            host: "host.example".to_owned(),
            port: 22,
            user: "root".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 1,
        }
    }

    #[test]
    fn saving_without_a_secret_keeps_the_one_already_stored() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        document.save_session(session("a", "web renamed"), None);

        assert_eq!(
            document
                .secret_for_session("a")
                .map(|secret| secret.expose().to_owned()),
            Some("hunter2".to_owned())
        );
        assert!(document.sessions()[0].auth.has_secret);
    }

    #[test]
    fn saving_with_a_secret_replaces_the_old_one() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("first".to_owned())));
        document.save_session(
            session("a", "web"),
            Some(SessionSecret::KeyPassphrase("second".to_owned())),
        );

        let stored = document.secret_for_session("a").expect("a secret");
        assert_eq!(stored.expose(), "second");
        assert!(
            matches!(stored, SessionSecret::Password(_)),
            "the method decides the kind"
        );
    }

    #[test]
    fn deleting_a_session_takes_its_secret_with_it() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        document.delete_session("a");

        assert!(document.sessions.is_empty());
        assert!(document.secrets.is_empty());
    }

    #[test]
    fn deleting_a_group_keeps_what_was_inside_it() {
        let mut document = Document::empty();
        document.save_group(SessionGroup {
            id: "g1".to_owned(),
            name: "servers".to_owned(),
            parent_id: None,
            order: 0,
            pinned: false,
        });
        document.save_group(SessionGroup {
            id: "g2".to_owned(),
            name: "nested".to_owned(),
            parent_id: Some("g1".to_owned()),
            order: 0,
            pinned: false,
        });
        let mut nested = session("a", "web");
        nested.group_id = Some("g1".to_owned());
        document.save_session(nested, None);

        document.delete_group("g1");

        assert_eq!(document.groups.len(), 1);
        assert_eq!(document.groups[0].parent_id, None);
        assert_eq!(document.sessions[0].group_id, None);
    }

    #[test]
    fn a_newer_document_is_refused_with_the_schema_code() {
        // One past this build's shape rather than a number written down: the point is that a
        // *newer* document is refused, and a test naming a version has to be edited every time the
        // current one moves — which is how such a test stops testing anything.
        let found = VAULT_SCHEMA_VERSION + 1;
        let mut future = Document::empty();
        future.schema_version = found;

        let error = migrate(future).expect_err("a document from the future is not this build's shape");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::StoreSchemaNewer);
        assert_eq!(
            error.message_params().get("found").map(String::as_str),
            Some(found.to_string().as_str())
        );
        assert_eq!(
            error.message_params().get("supported").map(String::as_str),
            Some(VAULT_SCHEMA_VERSION.to_string().as_str())
        );
    }

    #[test]
    fn a_document_without_a_library_gets_the_default_group() {
        // What an older build wrote: version 1, no command library. Upgrading has to leave
        // somewhere for the first saved command to go.
        let raw = br#"{"schemaVersion":1,"sessions":[],"groups":[],"keys":[],"secrets":[],"knownHosts":[]}"#;

        let document = from_slice(raw).expect("a version 1 document is readable");

        assert_eq!(document.schema_version, VAULT_SCHEMA_VERSION);
        assert_eq!(document.snippet_groups.len(), 1);
        assert_eq!(document.snippet_groups[0].id, DEFAULT_GROUP_ID);
        assert!(
            document.snippet_groups[0].name.is_empty(),
            "the default label is a translation"
        );
        assert!(document.command_history.is_empty());
    }

    #[test]
    fn a_document_written_before_the_words_existed_reads_as_having_none() {
        // Version 5 is the shape from before somebody could add their own words to what follows a
        // command. An absent list has to read as none rather than as a document that will not open.
        let raw = br#"{"schemaVersion":5,"sessions":[],"groups":[],"keys":[],"secrets":[],"knownHosts":[]}"#;

        let document = from_slice(raw).expect("a version 5 document is readable");

        assert_eq!(document.schema_version, VAULT_SCHEMA_VERSION);
        assert!(document.completion_words.is_empty());
    }

    #[test]
    fn a_document_without_a_schema_version_does_not_parse() {
        let error = from_slice(br#"{"sessions":[]}"#).expect_err("schemaVersion is required");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::VaultUnlockFailed);
    }

    #[test]
    fn a_document_written_before_the_sync_password_reads_as_having_none() {
        // Version 6 is the shape from before this application could keep a password for a sync
        // server. An absent one has to read as "nobody asked to keep one" rather than as a document
        // that will not open — and it must not be confused later with a password somebody forgot.
        let raw = br#"{"schemaVersion":6,"sessions":[],"groups":[],"keys":[],"secrets":[],"knownHosts":[]}"#;

        let mut document = from_slice(raw).expect("a version 6 document is readable");

        assert_eq!(document.schema_version, VAULT_SCHEMA_VERSION);
        assert!(document.sync_password().is_none());

        // Keeping one and forgetting it again leaves nothing behind — the two are the same field,
        // so there is no state where "forgotten" and "never kept" could differ.
        document.set_sync_password(Some("hunter2".to_owned()));
        assert_eq!(document.sync_password().expect("kept").expose(), "hunter2");

        document.set_sync_password(None);
        assert!(document.sync_password().is_none());
        document.set_sync_password(Some(String::new()));
        assert!(
            document.sync_password().is_none(),
            "an empty password is nobody keeping one"
        );

        // The passphrase that seals an upload is a second field, and keeping one is not keeping the
        // other: a server can need a login with nothing sealed, and a file can be sealed for a
        // server that takes anything from this address.
        document.set_sync_passphrase(Some("correct horse".to_owned()));
        assert_eq!(document.sync_passphrase().expect("kept").expose(), "correct horse");
        assert!(document.sync_password().is_none(), "and the other one stayed empty");

        document.set_sync_passphrase(None);
        assert!(document.sync_passphrase().is_none());
    }

    #[test]
    fn a_sync_password_is_never_printed() {
        let mut document = Document::empty();
        document.set_sync_password(Some("hunter2".to_owned()));
        document.set_sync_passphrase(Some("correct horse".to_owned()));
        document.set_snippet_token("github", Some("ghp_secret".to_owned()));

        let rendered = format!(
            "{document:?} {:?} {:?}",
            document.sync_password(),
            document.sync_passphrase()
        );

        assert!(
            !rendered.contains("hunter2"),
            "a password reached Debug output: {rendered}"
        );
    }

    #[test]
    fn a_document_written_before_the_tokens_reads_as_having_none() {
        // Version 7 is the shape from before this application could keep an access token for a code
        // hosting account. An absent table has to read as "nothing kept" rather than as a document
        // that will not open, and the two sync secrets have to survive the trip either way.
        let raw = br#"{"schemaVersion":7,"sessions":[],"groups":[],"keys":[],"secrets":[],"knownHosts":[]}"#;

        let mut document = from_slice(raw).expect("a version 7 document is readable");

        assert_eq!(document.schema_version, VAULT_SCHEMA_VERSION);
        assert!(document.snippet_token("github").is_none());

        document.set_snippet_token("github", Some("ghp_one".to_owned()));
        assert_eq!(document.snippet_token("github").expect("kept").expose(), "ghp_one");
        assert!(
            document.snippet_token("gitee").is_none(),
            "one account's token is not another's"
        );

        // Two GitLab installations are two accounts, which is the reason this is a table rather than
        // one field per service.
        document.set_snippet_token("gitlab:https://one.example", Some("glpat-one".to_owned()));
        document.set_snippet_token("gitlab:https://two.example", Some("glpat-two".to_owned()));
        assert_eq!(
            document
                .snippet_token("gitlab:https://one.example")
                .expect("kept")
                .expose(),
            "glpat-one"
        );
        assert_eq!(
            document
                .snippet_token("gitlab:https://two.example")
                .expect("kept")
                .expose(),
            "glpat-two"
        );

        document.set_snippet_token("github", None);
        assert!(
            document.snippet_token("github").is_none(),
            "forgetting one leaves the rest"
        );
        document.set_snippet_token("gitee", Some("   ".to_owned()));
        assert!(
            document.snippet_token("gitee").is_none(),
            "a token of nothing is nobody keeping one"
        );
        assert!(document.snippet_token("gitlab:https://one.example").is_some());
    }

    #[test]
    fn a_token_is_never_printed() {
        let mut document = Document::empty();
        document.set_snippet_token("github", Some("ghp_secret".to_owned()));

        let rendered = format!("{document:?} {:?}", document.snippet_token("github"));

        assert!(
            !rendered.contains("ghp_secret"),
            "a token reached Debug output: {rendered}"
        );
    }

    #[test]
    fn a_secret_is_never_printed() {
        let mut document = Document::empty();
        document.save_session(session("a", "web"), Some(SessionSecret::Password("hunter2".to_owned())));

        let rendered = format!("{document:?} {:?}", document.secret_for_session("a"));

        assert!(
            !rendered.contains("hunter2"),
            "a secret reached Debug output: {rendered}"
        );
        assert!(rendered.contains("redacted"));
    }
}

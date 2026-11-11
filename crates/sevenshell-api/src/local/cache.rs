//! What a call may reuse from the call before it.
//!
//! An agent asks a series of small questions about one machine: list this directory, read that
//! file, show the containers. Every one of them needs the same three things — an authenticated
//! connection, a protocol channel on top of it, and somebody to close them — and opening them again
//! per call would mean one authentication per question. So the host keeps one cache, shared by every
//! call, and this is it.
//!
//! Two rules shape the whole module:
//!
//! 1. **The cache holds handles, never credentials.** A file handle, a container connection id, a
//!    cluster connection id: things this process names, not things that open a door. The secrets are
//!    inside the core's connections, where they already were, and locking the vault is answered by
//!    [`ConnectionCache::close_all`].
//! 2. **A dead entry is reopened once, then reported.** Connections do go away — a server reboots, a
//!    terminal is closed, a runtime is uninstalled — and an agent that gets "not connected" for a
//!    cache it never knew about has no way to recover. One silent reopen is the difference between a
//!    stale cache and a broken tool.
//!
//! The core is held here rather than borrowed per call because one operation needs the shared
//! handle: a command run (`Core::run_across`) takes `&Arc<Core>` so its per-host tasks can outlive
//! the call.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sevenshell_core::Core;
use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_types::{ContainerSummary, K8sMethod, K8sRequest, K8sResponse, Session, SftpListing, SftpOutcome};
use tokio::sync::Mutex;

/// How long an entry nobody has touched survives. Long enough that a conversation is one connection,
/// short enough that a laptop which slept overnight does not wake up holding a socket that is gone.
const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The three kinds of thing that are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Container,
    Cluster,
}

/// An open file channel, and where it starts.
///
/// The starting path is remembered because it is the only way to ask "list the home directory" that
/// works for every backend: SFTP resolves it to a canonical path, WebDAV and FTP to the folder the
/// session names, and none of them accept an empty string.
struct FilesEntry {
    handle: String,
    start: String,
    last_used: Instant,
}

/// One container or cluster connection, by the id the core gave it.
struct ConnectionEntry {
    id: String,
    last_used: Instant,
}

#[derive(Default)]
struct Inner {
    files: HashMap<String, FilesEntry>,
    containers: HashMap<String, ConnectionEntry>,
    clusters: HashMap<String, ConnectionEntry>,
}

impl Inner {
    fn connections(&mut self, kind: Kind) -> &mut HashMap<String, ConnectionEntry> {
        match kind {
            Kind::Container => &mut self.containers,
            Kind::Cluster => &mut self.clusters,
        }
    }
}

/// The handles this installation is holding open for the calls that follow.
pub struct ConnectionCache {
    core: Arc<Core>,
    idle: Duration,
    inner: Mutex<Inner>,
}

impl ConnectionCache {
    pub fn new(core: Arc<Core>) -> Self {
        Self {
            core,
            idle: IDLE_TIMEOUT,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// How long an unused entry is kept. Only tests set this.
    pub fn with_idle_timeout(mut self, idle: Duration) -> Self {
        self.idle = idle;
        self
    }

    /// The core these entries belong to, which is also the shared handle a command run needs.
    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// Reads one directory, opening — or reopening once — the channel it needs.
    ///
    /// `path` of `None` means the place the channel opened at, which is the session's home for SFTP
    /// and the folder the session names for WebDAV and FTP.
    pub async fn files(&self, session_id: &str, path: Option<&str>) -> Result<SftpListing, ShellError> {
        let (handle, target) = self.files_target(session_id, path).await?;
        match self.core.list_files(&handle, &target).await {
            Ok(listing) => Ok(listing),
            Err(error) if gone(&error) => {
                self.forget_files(session_id).await;
                // The starting path is asked for again rather than reused: a reopened channel may
                // resolve its home somewhere else, and the old path was relative to the old one.
                let (handle, target) = self.files_target(session_id, path).await?;
                self.core.list_files(&handle, &target).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn read_text(&self, session_id: &str, path: &str) -> Result<String, ShellError> {
        let (handle, _) = self.files_handle(session_id).await?;
        match self.core.read_text_file(&handle, path).await {
            Ok(text) => Ok(text),
            Err(error) if gone(&error) => {
                self.forget_files(session_id).await;
                let (handle, _) = self.files_handle(session_id).await?;
                self.core.read_text_file(&handle, path).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn write_text(&self, session_id: &str, path: &str, text: &str) -> Result<(), ShellError> {
        let (handle, _) = self.files_handle(session_id).await?;
        match self.core.write_text_file(&handle, path, text).await {
            Ok(()) => Ok(()),
            Err(error) if gone(&error) => {
                self.forget_files(session_id).await;
                let (handle, _) = self.files_handle(session_id).await?;
                self.core.write_text_file(&handle, path, text).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn containers(&self, session_id: &str) -> Result<Vec<ContainerSummary>, ShellError> {
        let connection = self.connection(Kind::Container, session_id).await?;
        match self.core.container_list(&connection).await {
            Ok(containers) => Ok(containers),
            Err(error) if gone(&error) => {
                self.forget(Kind::Container, session_id).await;
                let connection = self.connection(Kind::Container, session_id).await?;
                self.core.container_list(&connection).await
            }
            Err(error) => Err(error),
        }
    }

    /// Runs one container action on the cached connection. The verdict has already been taken by the
    /// caller; this only carries out what was decided.
    pub async fn container_action(
        &self,
        session_id: &str,
        container_id: &str,
        action: &str,
        acknowledged: bool,
    ) -> Result<(), ShellError> {
        let connection = self.connection(Kind::Container, session_id).await?;
        match self
            .core
            .container_action(&connection, container_id, action, acknowledged)
            .await
        {
            Ok(()) => Ok(()),
            Err(error) if gone(&error) => {
                self.forget(Kind::Container, session_id).await;
                let connection = self.connection(Kind::Container, session_id).await?;
                self.core
                    .container_action(&connection, container_id, action, acknowledged)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    /// One GET against the cluster's API server.
    pub async fn k8s_get(&self, session_id: &str, path: &str) -> Result<K8sResponse, ShellError> {
        let connection = self.connection(Kind::Cluster, session_id).await?;
        match self.core.k8s_request(&connection, get_request(path)).await {
            Ok(response) => Ok(response),
            Err(error) if gone(&error) => {
                self.forget(Kind::Cluster, session_id).await;
                let connection = self.connection(Kind::Cluster, session_id).await?;
                self.core.k8s_request(&connection, get_request(path)).await
            }
            Err(error) => Err(error),
        }
    }

    /// Closes everything. What the host calls when the vault locks: every handle here rides on a
    /// credential that is no longer available, and holding a socket open for a locked vault would be
    /// the one piece of the session that outlived the lock.
    pub async fn close_all(&self) {
        let (files, containers, clusters) = {
            let mut inner = self.inner.lock().await;
            (
                std::mem::take(&mut inner.files),
                std::mem::take(&mut inner.containers),
                std::mem::take(&mut inner.clusters),
            )
        };

        for entry in files.values() {
            let _ = self.core.close_files(&entry.handle).await;
        }
        for entry in containers.values() {
            self.core.container_disconnect(&entry.id).await;
        }
        for entry in clusters.values() {
            self.core.k8s_disconnect(&entry.id);
        }
    }

    /// Closes and forgets everything cached for one session. Used when a terminal or a container tab
    /// it borrowed from goes away.
    pub async fn forget_session(&self, session_id: &str) {
        if let Some(entry) = self.inner.lock().await.files.remove(session_id) {
            let _ = self.core.close_files(&entry.handle).await;
        }
        self.forget(Kind::Container, session_id).await;
        self.forget(Kind::Cluster, session_id).await;
    }

    /// The handle and the path a read should use.
    async fn files_target(&self, session_id: &str, path: Option<&str>) -> Result<(String, String), ShellError> {
        let (handle, start) = self.files_handle(session_id).await?;
        let path = match path {
            Some(path) if !path.trim().is_empty() => path.to_owned(),
            _ => start,
        };
        Ok((handle, path))
    }

    async fn files_handle(&self, session_id: &str) -> Result<(String, String), ShellError> {
        let expired = {
            let mut inner = self.inner.lock().await;
            match inner.files.get_mut(session_id) {
                Some(entry) if entry.last_used.elapsed() < self.idle => {
                    entry.last_used = Instant::now();
                    return Ok((entry.handle.clone(), entry.start.clone()));
                }
                // Idle past the budget: the entry goes now, and the channel behind it is closed
                // before a new one is opened so a reopened session never doubles the channels.
                Some(_) => inner.files.remove(session_id),
                None => None,
            }
        };
        if let Some(expired) = expired {
            let _ = self.core.close_files(&expired.handle).await;
        }

        match self.core.open_files(session_id).await? {
            SftpOutcome::Opened { handle, listing, .. } => {
                let start = if listing.path.trim().is_empty() {
                    ".".to_owned()
                } else {
                    listing.path
                };

                let mut inner = self.inner.lock().await;
                // Two calls can miss at the same moment. The entry that got there first wins, and
                // this one closes the channel it opened: one wasted authentication either way, but a
                // channel nobody owns would outlive the cache and never be closed.
                if let Some(winner) = inner.files.get(session_id) {
                    let (kept, kept_start) = (winner.handle.clone(), winner.start.clone());
                    drop(inner);
                    let _ = self.core.close_files(&handle).await;
                    return Ok((kept, kept_start));
                }

                inner.files.insert(
                    session_id.to_owned(),
                    FilesEntry {
                        handle: handle.clone(),
                        start: start.clone(),
                        last_used: Instant::now(),
                    },
                );
                Ok((handle, start))
            }
            // A file channel has no prompt of its own in this interface: the decision belongs to
            // opening the session, and the answer here is the one the container and cluster tools
            // also give — the host's identity, so a person can go and open that session once.
            SftpOutcome::HostKeyUnknown { key } => {
                Err(ShellError::ssh_host_key_unknown(&key.host, key.port, &key.key_type))
            }
            SftpOutcome::HostKeyMismatch { key, .. } => {
                Err(ShellError::ssh_host_key_mismatch(&key.host, key.port, &key.key_type))
            }
        }
    }

    async fn forget_files(&self, session_id: &str) {
        if let Some(entry) = self.inner.lock().await.files.remove(session_id) {
            let _ = self.core.close_files(&entry.handle).await;
        }
    }

    async fn connection(&self, kind: Kind, session_id: &str) -> Result<String, ShellError> {
        let expired = {
            let mut inner = self.inner.lock().await;
            match inner.connections(kind).get_mut(session_id) {
                Some(entry) if entry.last_used.elapsed() < self.idle => {
                    entry.last_used = Instant::now();
                    return Ok(entry.id.clone());
                }
                Some(_) => inner.connections(kind).remove(session_id),
                None => None,
            }
        };
        if let Some(expired) = expired {
            self.close(kind, &expired.id).await;
        }

        let id = match kind {
            Kind::Container => self.core.container_connect(session_id).await?,
            Kind::Cluster => self.core.k8s_connect(session_id).await?,
        };

        let mut inner = self.inner.lock().await;
        // The same race as the file channel: the first connection to arrive is kept, and the second
        // is closed rather than left holding a runtime or a cluster with nothing to name it.
        if let Some(winner) = inner.connections(kind).get(session_id) {
            let kept = winner.id.clone();
            drop(inner);
            self.close(kind, &id).await;
            return Ok(kept);
        }

        inner.connections(kind).insert(
            session_id.to_owned(),
            ConnectionEntry {
                id: id.clone(),
                last_used: Instant::now(),
            },
        );
        Ok(id)
    }

    async fn forget(&self, kind: Kind, session_id: &str) {
        if let Some(entry) = self.inner.lock().await.connections(kind).remove(session_id) {
            self.close(kind, &entry.id).await;
        }
    }

    async fn close(&self, kind: Kind, id: &str) {
        match kind {
            Kind::Container => self.core.container_disconnect(id).await,
            Kind::Cluster => self.core.k8s_disconnect(id),
        }
    }
}

fn get_request(path: &str) -> K8sRequest {
    K8sRequest {
        method: K8sMethod::Get,
        path: path.to_owned(),
        body: None,
        content_type: None,
        // A read never needed an acknowledgement, and this tool has no way to carry one.
        acknowledged: false,
    }
}

/// Whether a failure means the cached entry is dead rather than the operation being wrong.
///
/// Deliberately narrow. A permission error, a missing directory and a timeout are all things a
/// second attempt would do again — and a timeout would do again *slowly* — so none of them reopen
/// anything. What is here is the vocabulary the core uses for "this handle names nothing now".
pub(crate) fn gone(error: &ShellError) -> bool {
    match error.code() {
        ErrorCode::ContainerNotConnected | ErrorCode::K8sNotConnected | ErrorCode::SshChannelFailed => true,
        // A file browser that has been closed answers with an internal error whose detail is the
        // only place this is said; there is no code for it, and inventing one here would mean
        // changing the catalog for a case that is a cache miss from this side.
        ErrorCode::AppInternal => error.detail().is_some_and(|detail| detail.contains("no longer open")),
        _ => false,
    }
}

/// A saved session, or nothing. Used by the tools for the words a prompt needs; a locked vault
/// answers `None` rather than an error, because the prompt is not the place to report a lock.
pub(crate) fn saved_session(core: &Core, session_id: &str) -> Option<Session> {
    core.vault()
        .sessions()
        .ok()?
        .into_iter()
        .find(|session| session.id == session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_channel_is_reopened_and_a_real_failure_is_not() {
        let dead = [
            ShellError::container_not_connected("c1"),
            ShellError::k8s_not_connected("k1"),
            ShellError::ssh_channel_failed("10.0.0.7", 22, "a channel closed"),
            ShellError::app_internal("the file browser f1 is no longer open"),
        ];
        for error in dead {
            assert!(
                gone(&error),
                "`{}` names a handle that is gone: {}",
                error.code().as_str(),
                error
            );
        }

        let live = [
            ShellError::sftp_not_found("/nope"),
            ShellError::app_internal("a pod terminal was registered twice"),
            ShellError::ssh_timeout("10.0.0.7", 22, 30),
        ];
        for error in live {
            assert!(
                !gone(&error),
                "`{}` would be answered the same way by a second attempt",
                error.code().as_str()
            );
        }
    }

    #[test]
    fn the_idle_budget_is_fifteen_minutes() {
        assert_eq!(IDLE_TIMEOUT, Duration::from_secs(900));
    }
}

//! ZMODEM on the browser host.
//!
//! The detector and both state machines live in `sevenshell-term`; this module is the host half.
//! A received file lands in the staging area and reaches the page through the same download token
//! an ordinary transfer uses. A file to send cannot be chosen by the Rust side here — there is no
//! native dialog in a browser tab — so the page is asked, stages the bytes through `/upload`, and
//! hands the token back. That is the same opaque-token rule as every other browser file path.
//!
//! The detector is push-driven: it advances only when it is fed. While a send is waiting on a
//! file read or on the far side's next frame, no terminal output arrives to feed it, so a ticker
//! keeps an active session moving. Without that tick, a real `rz` receives the ZDATA header and
//! then waits forever for the file it was told about.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use sevenshell_core::Core;
use sevenshell_error::ShellError;
use sevenshell_term::zmodem::{Detector, DetectorEvent, Direction, EndReason, FileInfo, ReceiveEvent};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, broadcast};

use crate::StagedFile;

const TICK: Duration = Duration::from_millis(25);

struct Session {
    detector: Detector,
    incoming: Option<Incoming>,
    sending_name: Option<String>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            detector: Detector::new(),
            incoming: None,
            sending_name: None,
        }
    }
}

struct Incoming {
    file: tokio::fs::File,
    name: String,
    path: PathBuf,
}

/// The ZMODEM half of every terminal the browser host is carrying.
pub struct Zmodem {
    core: Arc<Core>,
    events: broadcast::Sender<String>,
    staging: Arc<Mutex<HashMap<String, StagedFile>>>,
    sessions: Mutex<HashMap<String, Session>>,
}

impl Zmodem {
    pub fn new(
        core: Arc<Core>,
        events: broadcast::Sender<String>,
        staging: Arc<Mutex<HashMap<String, StagedFile>>>,
    ) -> Self {
        Self {
            core,
            events,
            staging,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Keeps every active session moving between terminal chunks.
    pub fn spawn_ticker(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(TICK).await;
                this.tick().await;
            }
        });
    }

    /// Feeds one chunk of terminal output through the detector and does what it asks.
    pub async fn feed(&self, handle: &str, bytes: Vec<u8>) {
        let core = self.core.clone();
        let events_channel = self.events.clone();
        let staging = self.staging.clone();
        let mut sessions = self.sessions.lock().await;
        let session = sessions.entry(handle.to_owned()).or_default();

        let events = match session.detector.feed(&bytes) {
            Ok(events) => events,
            Err(error) => {
                Self::notice(
                    &events_channel,
                    handle,
                    "failed",
                    None,
                    error.detail().map(str::to_owned),
                    None,
                );
                let abort = session.detector.abort();
                Self::write_replies(&core, handle, abort).await;
                session.incoming = None;
                session.sending_name = None;
                return;
            }
        };
        Self::handle_events(&core, &events_channel, &staging, handle, session, events).await;
    }

    /// Starts sending a file the page staged for a waiting `rz`.
    pub async fn send_file(&self, handle: &str, path: PathBuf) -> Result<(), ShellError> {
        let core = self.core.clone();
        let events_channel = self.events.clone();
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(handle)
            .ok_or_else(|| ShellError::terminal_invalid("that terminal is no longer carrying a ZMODEM session"))?;
        if session.detector.active() != Some(Direction::Send) {
            return Err(ShellError::terminal_invalid(
                "that terminal is not waiting for a ZMODEM file",
            ));
        }

        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|error| ShellError::app_internal(format!("the chosen file could not be read: {error}")))?;
        let size = metadata.len();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| ShellError::app_internal("the chosen file has no name"))?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|age| age.as_secs());
        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|error| ShellError::app_internal(format!("the chosen file could not be opened: {error}")))?;

        let events = match session.detector.start_sending(
            FileInfo {
                name: name.clone(),
                size,
                modified,
                // The protocol's mode field is for the far side; this side's file keeps its own.
                mode: Some(0o100644),
            },
            file,
        ) {
            Ok(events) => events,
            Err(error) => {
                let detail = error.detail().unwrap_or("the ZMODEM sender could not start").to_owned();
                Self::fail(&core, &events_channel, handle, session, None, detail.clone()).await;
                return Err(ShellError::terminal_invalid(detail));
            }
        };
        session.sending_name = Some(name.clone());
        Self::notice(&events_channel, handle, "sending", Some(name), None, None);
        Self::write_replies(&core, handle, events).await;
        Ok(())
    }

    /// Cancels whatever the terminal's ZMODEM session is doing, if anything.
    pub async fn cancel(&self, handle: &str) {
        let core = self.core.clone();
        let events_channel = self.events.clone();
        let mut sessions = self.sessions.lock().await;
        let Some(session) = sessions.get_mut(handle) else {
            return;
        };
        if session.detector.active().is_none() {
            return;
        }
        let abort = session.detector.abort();
        session.incoming = None;
        session.sending_name = None;
        Self::write_replies(&core, handle, abort).await;
        Self::notice(&events_channel, handle, "cancelled", None, None, None);
    }

    pub async fn forget(&self, handle: &str) {
        self.sessions.lock().await.remove(handle);
    }

    async fn tick(&self) {
        let handles: Vec<String> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .filter(|(_, session)| session.detector.active().is_some())
                .map(|(handle, _)| handle.clone())
                .collect()
        };
        for handle in handles {
            self.feed(&handle, Vec::new()).await;
        }
    }

    async fn handle_events(
        core: &Core,
        events_channel: &broadcast::Sender<String>,
        staging: &Arc<Mutex<HashMap<String, StagedFile>>>,
        handle: &str,
        session: &mut Session,
        detected: Vec<DetectorEvent>,
    ) {
        let mut finish_sending = false;
        for event in detected {
            match event {
                DetectorEvent::Output(bytes) => Self::emit_data(events_channel, handle, bytes),
                DetectorEvent::Reply(bytes) => {
                    if let Err(error) = core.write(handle, &bytes).await {
                        log::warn!("a ZMODEM reply could not be written to {handle}: {error}");
                    }
                }
                DetectorEvent::Started(direction) => {
                    Self::notice(events_channel, handle, "started", None, None, None);
                    if direction == Direction::Send {
                        // The page owns the picker. The session stays waiting until either the
                        // staged token arrives or the person cancels.
                        Self::notice(events_channel, handle, "choose", None, None, None);
                    }
                }
                DetectorEvent::Received(ReceiveEvent::Offer(info)) => {
                    let name = safe_name(&info.name);
                    let directory = staging_root().join(format!("zmodem-{}", uuid::Uuid::new_v4().simple()));
                    if let Err(error) = tokio::fs::create_dir_all(&directory).await {
                        Self::fail(
                            core,
                            events_channel,
                            handle,
                            session,
                            Some(name),
                            format!("the ZMODEM folder could not be created: {error}"),
                        )
                        .await;
                        return;
                    }
                    let path = directory.join(&name);
                    match tokio::fs::File::create(&path).await {
                        Ok(file) => {
                            session.incoming = Some(Incoming {
                                file,
                                name: name.clone(),
                                path,
                            });
                            Self::notice(events_channel, handle, "receiving", Some(name), None, None);
                        }
                        Err(error) => {
                            Self::fail(
                                core,
                                events_channel,
                                handle,
                                session,
                                Some(name),
                                format!("the received file could not be created: {error}"),
                            )
                            .await;
                            return;
                        }
                    }
                }
                DetectorEvent::Received(ReceiveEvent::Data(data)) => {
                    let Some(incoming) = session.incoming.as_mut() else {
                        continue;
                    };
                    if let Err(error) = incoming.file.write_all(&data).await {
                        let name = incoming.name.clone();
                        session.incoming = None;
                        Self::fail(
                            core,
                            events_channel,
                            handle,
                            session,
                            Some(name),
                            format!("the received file could not be written: {error}"),
                        )
                        .await;
                        return;
                    }
                }
                DetectorEvent::Received(ReceiveEvent::Complete(_info)) => {
                    let Some(mut incoming) = session.incoming.take() else {
                        continue;
                    };
                    let _ = incoming.file.flush().await;
                    let token = uuid::Uuid::new_v4().simple().to_string();
                    staging.lock().await.insert(
                        token.clone(),
                        StagedFile {
                            path: incoming.path,
                            name: incoming.name.clone(),
                        },
                    );
                    Self::notice(events_channel, handle, "done", Some(incoming.name), None, Some(token));
                }
                DetectorEvent::Received(ReceiveEvent::Skipped { name, reason }) => {
                    Self::notice(
                        events_channel,
                        handle,
                        "skipped",
                        Some(safe_name(&name)),
                        Some(reason),
                        None,
                    );
                }
                DetectorEvent::Sent { bytes } => {
                    Self::notice(
                        events_channel,
                        handle,
                        "sent",
                        session.sending_name.clone(),
                        Some(bytes.to_string()),
                        None,
                    );
                    finish_sending = true;
                }
                DetectorEvent::Ended(reason) => {
                    session.incoming = None;
                    let sent_name = session.sending_name.take();
                    if reason == EndReason::Complete && sent_name.is_some() {
                        Self::notice(events_channel, handle, "finished", sent_name, None, None);
                    }
                }
            }
        }

        if finish_sending && matches!(session.detector.active(), Some(Direction::Send)) {
            match session.detector.finish_sending() {
                Ok(events) => Self::write_replies(core, handle, events).await,
                Err(error) => Self::notice(
                    events_channel,
                    handle,
                    "failed",
                    None,
                    error.detail().map(str::to_owned),
                    None,
                ),
            }
        }
    }

    async fn fail(
        core: &Core,
        events: &broadcast::Sender<String>,
        handle: &str,
        session: &mut Session,
        name: Option<String>,
        detail: String,
    ) {
        Self::notice(events, handle, "failed", name, Some(detail), None);
        let abort = session.detector.abort();
        session.incoming = None;
        session.sending_name = None;
        Self::write_replies(core, handle, abort).await;
    }

    async fn write_replies(core: &Core, handle: &str, events: Vec<DetectorEvent>) {
        for event in events {
            if let DetectorEvent::Reply(bytes) = event
                && let Err(error) = core.write(handle, &bytes).await
            {
                log::warn!("a ZMODEM reply could not be written to {handle}: {error}");
            }
        }
    }

    fn emit_data(events: &broadcast::Sender<String>, handle: &str, bytes: Vec<u8>) {
        let payload = json!({ "handle": handle, "base64": BASE64.encode(bytes) });
        let _ = events.send(json!({ "event": "terminal:data", "payload": payload }).to_string());
    }

    fn notice(
        events: &broadcast::Sender<String>,
        handle: &str,
        kind: &str,
        name: Option<String>,
        detail: Option<String>,
        token: Option<String>,
    ) {
        let mut payload = json!({ "handle": handle, "kind": kind });
        if let Some(name) = name {
            payload["name"] = Value::String(name);
        }
        if let Some(detail) = detail {
            payload["detail"] = Value::String(detail);
        }
        if let Some(token) = token {
            payload["token"] = Value::String(token);
        }
        let _ = events.send(json!({ "event": "zmodem:event", "payload": payload }).to_string());
    }
}

/// Keeps a remote name from choosing a directory outside the staging folder.
fn safe_name(name: &str) -> String {
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim_matches('\0')
        .trim();
    if name.is_empty() || name == "." || name == ".." {
        "zmodem-download".to_owned()
    } else {
        name.to_owned()
    }
}

fn staging_root() -> PathBuf {
    crate::staging_root()
}

#[cfg(test)]
mod tests {
    use super::safe_name;

    #[test]
    fn a_received_name_cannot_choose_a_directory() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("a\\b\\file.txt"), "file.txt");
        assert_eq!(safe_name(".."), "zmodem-download");
        assert_eq!(safe_name(""), "zmodem-download");
    }
}

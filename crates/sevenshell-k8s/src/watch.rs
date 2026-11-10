//! A watch: one long-lived GET whose answer is a stream of events.
//!
//! The shape that matters here is what happens when the stream breaks. An API server restarts, a
//! proxy reaps an idle connection, and both look like a read error rather than a decision — so a
//! broken stream is asked for again, with a backoff, and from the last resource version that was
//! seen rather than from the beginning. The one answer that must *not* be retried is `410 Gone`: the
//! server has forgotten the version, and asking again in a loop would spin forever without ever
//! delivering the events that were missed. That is reported as a relist, and the loop stops.

use std::time::Duration;

use sevenshell_types::{K8sStreamEnd, K8sWatchEvent};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch as cancel_channel;

use crate::client::K8sClient;
use crate::rest;

/// How long the first retry waits, doubling from there.
const FIRST_BACKOFF: Duration = Duration::from_secs(1);

/// The longest a retry waits. A watch that has been down for a while should still come back once
/// the cluster does, so there is no giving up — but it should not hammer an API server that is
/// already having a bad day.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The longest one watch event may be.
///
/// A watch line is one object, and the largest object anybody watches is a few hundred kilobytes of
/// CRD. Four mebibytes is far above that and far below what would exhaust a machine that has twenty
/// watches open.
const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

/// How much of an error answer is quoted back.
const MAX_DETAIL_BYTES: usize = 300;

/// Follows one list until it ends, and reports the end exactly once.
pub(crate) async fn run(
    client: std::sync::Arc<K8sClient>,
    path: &str,
    stream_id: &str,
    events: UnboundedSender<K8sWatchEvent>,
    end: UnboundedSender<K8sStreamEnd>,
    mut cancel: cancel_channel::Receiver<bool>,
) {
    let mut backoff = FIRST_BACKOFF;
    let mut resource_version: Option<String> = None;
    let mut ended: Option<String> = None;

    loop {
        // The first attempt keeps the caller's own query — including a resource version it took
        // from a list it already read. Every later attempt starts from the last version seen here,
        // which is what makes a reconnect lose nothing.
        let target = match &resource_version {
            Some(version) => rest::with_resource_version(path, version),
            None => rest::with_watch_params(path),
        };

        match attempt(&client, &target, stream_id, &events, &mut resource_version, &mut cancel).await {
            Attempt::Cancelled => break,
            // The server closed the stream on its own terms. That is an end, not a failure: the
            // interface relists and watches again when it wants to.
            Attempt::Closed => break,
            Attempt::Relist(detail) => {
                ended = Some(detail);
                break;
            }
            Attempt::Broken(detail) => {
                ended = Some(detail);
                break;
            }
            Attempt::Retry { established, detail } => {
                if established {
                    backoff = FIRST_BACKOFF;
                }
                log::warn!("the watch on `{path}` was interrupted ({detail}); asking again in {backoff:?}");
                if !pause(backoff, &mut cancel).await {
                    break;
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }

    // One message, one place: a stop that races the stream ending must not produce two.
    let _ = end.send(K8sStreamEnd {
        stream_id: stream_id.to_owned(),
        error: ended,
    });
}

/// What one attempt at the stream came to.
enum Attempt {
    Cancelled,
    /// The server ended the stream cleanly.
    Closed,
    /// The server no longer holds the version this watch was following.
    Relist(String),
    /// Asking again would fail the same way.
    Broken(String),
    /// Worth asking again.
    Retry {
        established: bool,
        detail: String,
    },
}

impl Attempt {
    fn retry(established: bool, detail: impl Into<String>) -> Self {
        Self::Retry {
            established,
            detail: detail.into(),
        }
    }
}

async fn attempt(
    client: &K8sClient,
    target: &str,
    stream_id: &str,
    events: &UnboundedSender<K8sWatchEvent>,
    resource_version: &mut Option<String>,
    cancel: &mut cancel_channel::Receiver<bool>,
) -> Attempt {
    let url = match rest::url(client.base(), target) {
        Ok(url) => url,
        Err(error) => return Attempt::Broken(error.to_string()),
    };
    let request = match client.prepare("GET", &url) {
        // No request deadline: a watch is supposed to stay open, and only the connect timeout (on
        // the client) bounds how long it may take to establish one.
        Ok(request) => request,
        Err(error) => return Attempt::Broken(error.to_string()),
    };

    let mut response = tokio::select! {
        _ = cancel.changed() => return Attempt::Cancelled,
        response = request.send() => match response {
            Ok(response) => response,
            Err(error) => return Attempt::retry(false, error.to_string()),
        },
    };

    let status = response.status();
    if status != reqwest::StatusCode::OK {
        let detail = match rest::read_capped(response, MAX_DETAIL_BYTES).await {
            Ok(body) => format!("the server answered {status}: {}", summarize(&body)),
            Err(_) => format!("the server answered {status}"),
        };
        return match status.as_u16() {
            // The one answer that means "read the list again": the version is gone, and a watch
            // from it can never be resumed.
            410 => Attempt::Relist(format!(
                "the server no longer holds the resource version this watch was following; the list has to be read again ({detail})"
            )),
            // An answer that will not change by asking again.
            401 | 403 | 404 => Attempt::Broken(detail),
            // Everything else — a proxy in front of a restarting server, a rate limit — is asked
            // again after a pause.
            _ => Attempt::retry(false, detail),
        };
    }

    let mut buffer: Vec<u8> = Vec::new();
    let mut progressed = false;
    loop {
        let chunk = tokio::select! {
            _ = cancel.changed() => return Attempt::Cancelled,
            chunk = response.chunk() => match chunk {
                Ok(Some(chunk)) => chunk,
                // Clean end of the response body.
                Ok(None) => return Attempt::Closed,
                Err(error) => return Attempt::retry(progressed, error.to_string()),
            },
        };

        buffer.extend_from_slice(&chunk);
        while let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=index).collect();
            let line = &line[..line.len() - 1];
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match reading(line) {
                Reading::Skip => {}
                Reading::Stop { kind, object, verdict } => {
                    // The event itself is delivered first: the interface shows the server's own
                    // words before the stream goes away.
                    emit(
                        events,
                        &K8sWatchEvent {
                            stream_id: stream_id.to_owned(),
                            kind,
                            object,
                        },
                    );
                    return match verdict {
                        Verdict::Relist(detail) => Attempt::Relist(detail),
                        Verdict::Broken(detail) => Attempt::Broken(detail),
                    };
                }
                Reading::Event {
                    kind,
                    object,
                    resource_version: seen,
                } => {
                    if let Some(seen) = seen {
                        *resource_version = Some(seen);
                    }
                    if !emit(
                        events,
                        &K8sWatchEvent {
                            stream_id: stream_id.to_owned(),
                            kind,
                            object,
                        },
                    ) {
                        // Nothing is listening any more, so there is no reason to keep the socket.
                        return Attempt::Cancelled;
                    }
                    progressed = true;
                }
            }
        }

        if buffer.len() > MAX_EVENT_BYTES {
            return Attempt::Broken(format!(
                "one watch event was larger than {} MiB",
                MAX_EVENT_BYTES / (1024 * 1024)
            ));
        }
    }
}

/// Sends one event. False means nobody is listening any more.
fn emit(events: &UnboundedSender<K8sWatchEvent>, event: &K8sWatchEvent) -> bool {
    events.send(event.clone()).is_ok()
}

/// What one line of the stream came to.
enum Reading {
    /// Not an event this build understands; the stream continues.
    Skip,
    Event {
        kind: String,
        object: String,
        resource_version: Option<String>,
    },
    /// The server said the stream cannot continue.
    Stop {
        kind: String,
        object: String,
        verdict: Verdict,
    },
}

enum Verdict {
    Relist(String),
    Broken(String),
}

fn reading(line: &[u8]) -> Reading {
    #[derive(serde::Deserialize)]
    struct Raw {
        #[serde(rename = "type", default)]
        kind: String,
        #[serde(default)]
        object: serde_json::Value,
    }

    let Ok(raw) = serde_json::from_slice::<Raw>(line) else {
        // A watch stream carries one JSON object per line, and a line this side cannot read is
        // worth a note rather than the end of the stream: the object after it is still good.
        log::warn!("a watch event could not be read and was skipped");
        return Reading::Skip;
    };
    if raw.kind.is_empty() {
        return Reading::Skip;
    }

    let resource_version = raw
        .object
        .get("metadata")
        .and_then(|metadata| metadata.get("resourceVersion"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let object = raw.object.to_string();

    if raw.kind.eq_ignore_ascii_case("ERROR") {
        let code = raw.object.get("code").and_then(serde_json::Value::as_u64);
        let reason = raw.object.get("reason").and_then(serde_json::Value::as_str);
        let message = raw
            .object
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(summarize)
            .unwrap_or_else(|| "the server ended the watch".to_owned());
        let verdict = if code == Some(410) || reason == Some("Expired") {
            Verdict::Relist(format!(
                "the server no longer holds the resource version this watch was following; the list has to be read again ({message})"
            ))
        } else {
            Verdict::Broken(message)
        };
        return Reading::Stop {
            kind: raw.kind,
            object,
            verdict,
        };
    }

    Reading::Event {
        kind: raw.kind,
        object,
        resource_version,
    }
}

/// One line of a server's explanation, short enough to sit in an event.
fn summarize(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = flat.trim();
    if flat.is_empty() {
        return "no detail".to_owned();
    }
    match flat.char_indices().nth(MAX_DETAIL_BYTES) {
        Some((index, _)) => format!("{}…", &flat[..index]),
        None => flat.to_owned(),
    }
}

/// Waits, unless somebody stops the watch first. False means it was stopped.
async fn pause(delay: Duration, cancel: &mut cancel_channel::Receiver<bool>) -> bool {
    tokio::select! {
        _ = cancel.changed() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

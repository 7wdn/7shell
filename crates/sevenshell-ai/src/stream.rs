//! Turning one response into a stream of events.
//!
//! Two layers, kept apart:
//!
//! * [`events`] turns an HTTP body into [`SseEvent`]s. It owns the two deadlines that belong to
//!   reading — the first byte and the quiet gap — because they are the same for both adapters.
//! * [`provider_stream`] drives a [`SseParser`] over those events. The parser is the adapter's own
//!   vocabulary (`message_delta`, `choices[].delta`); everything it answers with is
//!   [`StreamEvent`]s, and the queue in the bridge is what lets one wire event become several — a
//!   `message_delta` carries both usage and a stop reason.
//!
//! Cancellation is dropping the stream. Nothing here spawns a task, so a dropped stream drops the
//! response with it and the connection closes; there is no background reader left writing into a
//! channel nobody will read. That is also why a failure inside the stream ends it: an adapter that
//! cannot read its own protocol must not keep reading bytes it will misinterpret.

use std::collections::VecDeque;
use std::pin::Pin;

use futures_util::stream::{Stream, StreamExt};
use sevenshell_error::scrub_bounded;

use crate::http::Limits;
use crate::provider::ProviderStream;
use crate::sse::{SseBuffer, SseEvent};
use crate::types::{ProviderError, StreamEvent};

/// The longest transport detail an event carries.
const TRANSPORT_DETAIL_LIMIT: usize = 512;

/// One adapter's reading of the wire.
///
/// `on_event` answers with zero or more events: a `thinking` block start is nothing the interface
/// draws on its own, while a tool call's `content_block_stop` is the one moment its arguments can be
/// parsed. `on_end` is the last chance to report something the stream never said — a tool call whose
/// arguments arrived but whose closing event did not, or a message that ended without a stop.
pub(crate) trait SseParser: Send + 'static {
    fn on_event(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, ProviderError>;

    fn on_end(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        Ok(Vec::new())
    }
}

/// The body of one response, as complete events.
///
/// The first read runs under [`Limits::first_token`] and every later one under [`Limits::idle`]: a
/// provider that has begun answering has already proved it is there, and should be allowed to think.
/// A half event left in the buffer when the body ends is dropped here; what that means is the
/// adapter's judgement, and it reports it when [`SseParser::on_end`] runs.
pub(crate) fn events(
    response: reqwest::Response,
    limits: Limits,
) -> impl Stream<Item = Result<SseEvent, ProviderError>> + Send + 'static {
    let state = BodyState {
        body: response,
        buffer: SseBuffer::new(),
        ready: VecDeque::new(),
        first_read: true,
        finished: false,
        limits,
    };

    futures_util::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.ready.pop_front() {
                return Some((Ok(event), state));
            }
            if state.finished {
                return None;
            }

            let patience = if state.first_read {
                state.limits.first_token
            } else {
                state.limits.idle
            };

            match tokio::time::timeout(patience, state.body.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    state.first_read = false;
                    state.ready.extend(state.buffer.push(&chunk));
                }
                Ok(Ok(None)) => state.finished = true,
                Ok(Err(error)) => {
                    state.finished = true;
                    return Some((Err(ProviderError::Network(transport_detail(&error))), state));
                }
                Err(_) => {
                    state.finished = true;
                    return Some((Err(ProviderError::Timeout), state));
                }
            }
        }
    })
}

struct BodyState {
    body: reqwest::Response,
    buffer: SseBuffer,
    ready: VecDeque<SseEvent>,
    first_read: bool,
    finished: bool,
    limits: Limits,
}

/// The adapter's stream, with its parser driven over the events.
pub(crate) fn provider_stream<P>(
    source: impl Stream<Item = Result<SseEvent, ProviderError>> + Send + 'static,
    parser: P,
) -> ProviderStream
where
    P: SseParser,
{
    let state = Bridge {
        source: Box::pin(source),
        parser,
        ready: VecDeque::new(),
        finished: false,
    };

    Box::pin(futures_util::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.ready.pop_front() {
                return Some((item, state));
            }
            if state.finished {
                return None;
            }

            match state.source.next().await {
                Some(Ok(event)) => match state.parser.on_event(&event) {
                    Ok(events) => state.ready.extend(events.into_iter().map(Ok)),
                    Err(error) => {
                        state.finished = true;
                        state.ready.push_back(Err(error));
                    }
                },
                Some(Err(error)) => {
                    state.finished = true;
                    state.ready.push_back(Err(error));
                }
                None => {
                    state.finished = true;
                    match state.parser.on_end() {
                        Ok(events) => state.ready.extend(events.into_iter().map(Ok)),
                        Err(error) => state.ready.push_back(Err(error)),
                    }
                }
            }
        }
    }))
}

struct Bridge<P> {
    source: Pin<Box<dyn Stream<Item = Result<SseEvent, ProviderError>> + Send>>,
    parser: P,
    ready: VecDeque<Result<StreamEvent, ProviderError>>,
    finished: bool,
}

fn transport_detail(error: &reqwest::Error) -> String {
    scrub_bounded(&error.to_string(), TRANSPORT_DETAIL_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// A server that answers with `head` and then behaves as the test says.
    async fn server(head: &'static str, body: &'static str, then: Behaviour) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
        let address = listener.local_addr().expect("an address");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("a connection");
            read_head(&mut stream).await;
            stream.write_all(head.as_bytes()).await.expect("the head");
            stream.write_all(body.as_bytes()).await.expect("the body");
            stream.flush().await.expect("a flush");
            match then {
                Behaviour::Break => {
                    // An answer that stops without its terminating chunk: the connection went away
                    // in the middle of the body, which is what a dropped provider looks like.
                    let _ = stream.shutdown().await;
                }
                Behaviour::Hold => {
                    // Keep the socket open and unread, so the only thing that can end the test is
                    // the client giving up on the body.
                    std::future::pending::<()>().await;
                }
            }
        });

        address
    }

    enum Behaviour {
        Hold,
        Break,
    }

    async fn read_head(stream: &mut TcpStream) {
        let mut taken = Vec::new();
        let mut byte = [0u8; 1];
        while !taken.ends_with(b"\r\n\r\n") {
            let read = stream.read(&mut byte).await.expect("a read");
            if read == 0 {
                break;
            }
            taken.push(byte[0]);
        }
        // The request carried a body; reading its head only leaves it in the socket, which is fine
        // for a server that writes its answer regardless.
    }

    fn streaming_head() -> &'static str {
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
    }

    #[tokio::test]
    async fn an_idle_stream_times_out_instead_of_hanging() {
        let address = server(streaming_head(), "", Behaviour::Hold).await;
        let response = reqwest::Client::new()
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("a response");

        let limits = Limits {
            first_token: Duration::from_millis(500),
            idle: Duration::from_millis(100),
        };
        let mut stream = Box::pin(events(response, limits));

        let item = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("the reader must not wait forever")
            .expect("one item");

        assert_eq!(item, Err(ProviderError::Timeout));
        assert!(stream.next().await.is_none(), "a timed-out stream ends");
    }

    #[tokio::test]
    async fn a_body_that_breaks_off_carries_the_transport_failure() {
        // A well-formed chunked frame cut off before its terminating chunk: the client sees a body
        // that ended in the middle, which is how a dropped connection mid-answer looks.
        let address = server(streaming_head(), "12\r\ndata: {\"a\": 1}\n\n", Behaviour::Break).await;
        let response = reqwest::Client::new()
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("a response");

        let mut stream = Box::pin(events(response, Limits::default()));
        let mut seen_error = false;
        while let Some(item) = stream.next().await {
            if matches!(item, Err(ProviderError::Network(_))) {
                seen_error = true;
            }
        }
        assert!(
            seen_error,
            "a body that stopped early is a transport failure, not a short answer"
        );
    }

    #[tokio::test]
    async fn a_parser_failure_ends_the_stream_after_the_events_before_it() {
        struct Failing;

        impl SseParser for Failing {
            fn on_event(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
                if event.data == "bad" {
                    return Err(ProviderError::Protocol("no".to_owned()));
                }
                Ok(vec![StreamEvent::TextDelta {
                    index: 0,
                    text: event.data.clone(),
                }])
            }
        }

        let source = futures_util::stream::iter([
            Ok(SseEvent {
                event: None,
                data: "first".to_owned(),
            }),
            Ok(SseEvent {
                event: None,
                data: "bad".to_owned(),
            }),
            Ok(SseEvent {
                event: None,
                data: "never".to_owned(),
            }),
        ]);
        let mut stream = provider_stream(source, Failing);

        assert!(matches!(stream.next().await, Some(Ok(StreamEvent::TextDelta { .. }))));
        assert!(matches!(stream.next().await, Some(Err(ProviderError::Protocol(_)))));
        assert!(
            stream.next().await.is_none(),
            "nothing after a failure is worth reading"
        );
    }
}

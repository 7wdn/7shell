//! Following a list: what arrives, when it stops, and what a broken stream is told to do.
//!
//! There is no real cluster on the machine these tests run on, so the server here is a socket the
//! test scripts line by line. What it proves is the half of a watch that is this side's own
//! behaviour: the query it asks, the events it hands on, that a stop ends the stream once, and that
//! a stream the server forgot is reported rather than asked for again in a loop.

mod common;

use std::time::Duration;

use sevenshell_error::ErrorCode;
use sevenshell_k8s::{K8sManager, Kubeconfig};
use sevenshell_types::{K8sStreamEnd, K8sWatchEvent};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use common::{Answer, Ending, document, fake};

struct Watching {
    events: UnboundedReceiver<K8sWatchEvent>,
    end: UnboundedReceiver<K8sStreamEnd>,
}

impl Watching {
    async fn next_event(&mut self) -> K8sWatchEvent {
        tokio::time::timeout(Duration::from_secs(5), self.events.recv())
            .await
            .expect("an event before the deadline")
            .expect("an event, not a closed channel")
    }

    async fn finished(&mut self) -> K8sStreamEnd {
        tokio::time::timeout(Duration::from_secs(10), self.end.recv())
            .await
            .expect("the stream to end before the deadline")
            .expect("an end, not a closed channel")
    }
}

async fn connected(base: &str) -> (K8sManager, String) {
    let config = Kubeconfig::parse(document(base, "t0ken").as_bytes()).expect("a kubeconfig");
    let manager = K8sManager::new();
    let id = manager.connect(&config, None).await.expect("a connection");
    (manager, id)
}

#[tokio::test]
async fn a_watch_hands_on_what_the_server_sends_and_asks_for_bookmarks() {
    let server = fake(|_, _| Answer::Stream {
        status: 200,
        lines: vec![
            r#"{"type":"ADDED","object":{"kind":"Pod","metadata":{"name":"web","resourceVersion":"10"}}}"#.to_owned(),
            r#"{"type":"MODIFIED","object":{"kind":"Pod","metadata":{"name":"web","resourceVersion":"11"}}}"#
                .to_owned(),
            r#"{"type":"DELETED","object":{"kind":"Pod","metadata":{"name":"web","resourceVersion":"12"}}}"#.to_owned(),
            r#"{"type":"BOOKMARK","object":{"kind":"Pod","metadata":{"resourceVersion":"13"}}}"#.to_owned(),
        ],
        then: Ending::Close,
    })
    .await;
    let (manager, connection) = connected(&server.base).await;
    let (events, events_in) = unbounded_channel();
    let (end, end_in) = unbounded_channel();
    let mut watching = Watching {
        events: events_in,
        end: end_in,
    };

    let stream_id = manager
        .watch_start(&connection, "/api/v1/pods?labelSelector=app%3Dweb", events, end)
        .expect("a watch");

    for expected in ["ADDED", "MODIFIED", "DELETED", "BOOKMARK"] {
        let event = watching.next_event().await;
        assert_eq!(event.kind, expected);
        assert_eq!(event.stream_id, stream_id, "every event names the stream it came from");
        assert!(
            event.object.contains("\"kind\""),
            "the object travels as the server sent it"
        );
    }

    let ending = watching.finished().await;
    assert_eq!(ending.stream_id, stream_id);
    assert_eq!(ending.error, None, "a stream the server closed is a clean end");

    let seen = server.last();
    assert_eq!(seen.method, "GET");
    assert!(
        seen.path.starts_with("/api/v1/pods?labelSelector=app%3Dweb"),
        "the caller's selector has to survive: {}",
        seen.path
    );
    assert!(
        seen.path.contains("watch=true"),
        "the path should ask for a watch: {}",
        seen.path
    );
    assert!(
        seen.path.contains("allowWatchBookmarks=true"),
        "bookmarks keep the resource version fresh: {}",
        seen.path
    );
    assert_eq!(
        manager.open_streams(),
        0,
        "a stream that ended is not still counted as open"
    );
}

#[tokio::test]
async fn a_watch_needs_a_connection_that_is_open() {
    let (events, _events) = unbounded_channel();
    let (end, _end) = unbounded_channel();
    let manager = K8sManager::new();

    let error = manager
        .watch_start("no-such-connection", "/api/v1/pods", events, end)
        .expect_err("a watch needs a connection");

    assert_eq!(error.code(), ErrorCode::K8sNotConnected);
}

#[tokio::test]
async fn stopping_a_watch_ends_it_exactly_once() {
    let server = fake(|_, _| Answer::Stream {
        status: 200,
        lines: Vec::new(),
        then: Ending::Hold,
    })
    .await;
    let (manager, connection) = connected(&server.base).await;
    let (events, events_in) = unbounded_channel();
    let (end, end_in) = unbounded_channel();
    let mut watching = Watching {
        events: events_in,
        end: end_in,
    };

    let stream_id = manager
        .watch_start(&connection, "/api/v1/pods", events, end)
        .expect("a watch");
    server.wait_for(1).await;
    assert_eq!(manager.open_streams(), 1);

    assert!(manager.stop_stream(&stream_id), "the stream was open");
    let ending = watching.finished().await;
    assert_eq!(ending.stream_id, stream_id);
    assert_eq!(ending.error, None, "a stop the person asked for is not a failure");

    // The second stop has nothing left to stop, and it must not produce a second end.
    assert!(!manager.stop_stream(&stream_id), "the stream had already ended");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(watching.end.try_recv().is_err(), "a stop must not be reported twice");
    assert!(watching.events.try_recv().is_err(), "a held stream sends nothing");
    assert_eq!(manager.open_streams(), 0);
}

#[tokio::test]
async fn a_stream_registered_by_another_kind_of_reader_is_stopped_the_same_way() {
    // The stop command the interface has is one command for every stream. A log stream registers
    // itself here, and the same stop has to reach it.
    let server = fake(|_, _| Answer::Once {
        status: 200,
        body: "{}".to_owned(),
    })
    .await;
    let (manager, connection) = connected(&server.base).await;
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&stopped);

    manager.register_stream("log-stream", &connection, move || {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    assert_eq!(manager.open_streams(), 1);

    assert!(manager.stop_stream("log-stream"), "the stream was registered");
    assert!(
        stopped.load(std::sync::atomic::Ordering::SeqCst),
        "the stop has to reach the stream that registered itself"
    );

    // A stream that ends on its own leaves nothing to stop.
    manager.register_stream("log-stream-2", &connection, || {});
    manager.forget_stream("log-stream-2");
    assert!(!manager.stop_stream("log-stream-2"));
    assert!(!manager.stop_stream("never-registered"));
}

#[tokio::test]
async fn disconnecting_stops_the_watches_it_opened() {
    let server = fake(|_, _| Answer::Stream {
        status: 200,
        lines: Vec::new(),
        then: Ending::Hold,
    })
    .await;
    let (manager, connection) = connected(&server.base).await;
    let (events, events_in) = unbounded_channel();
    let (end, end_in) = unbounded_channel();
    let mut watching = Watching {
        events: events_in,
        end: end_in,
    };

    manager
        .watch_start(&connection, "/api/v1/pods", events, end)
        .expect("a watch");
    server.wait_for(1).await;

    manager.disconnect(&connection);

    // The stream ends by itself once its connection is gone: nothing else has to remember it.
    let ending = watching.finished().await;
    assert_eq!(ending.error, None);
    assert_eq!(manager.open_streams(), 0);
}

#[tokio::test]
async fn a_gone_answer_is_reported_as_a_relist_and_not_asked_for_again() {
    let server = fake(|_, _| Answer::Status {
        status: 410,
        reason: "Expired",
        message: "too old resource version: 10 (13)",
        code: 410,
    })
    .await;
    let (manager, connection) = connected(&server.base).await;
    let (events, events_in) = unbounded_channel();
    let (end, end_in) = unbounded_channel();
    let mut watching = Watching {
        events: events_in,
        end: end_in,
    };

    manager
        .watch_start(&connection, "/api/v1/pods", events, end)
        .expect("a watch");

    let ending = watching.finished().await;
    let error = ending.error.expect("a relist is something the interface has to act on");
    assert!(
        error.contains("read again"),
        "the end should say a relist is needed, not just that it failed: {error}"
    );

    // A hot loop would ask again immediately; a watch that respects a 410 does not ask at all.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(server.count(), 1, "410 must not be retried in a loop");
}

#[tokio::test]
async fn a_stream_the_server_closes_is_an_end_and_not_a_retry() {
    let server = fake(|_, _| Answer::Stream {
        status: 200,
        lines: Vec::new(),
        then: Ending::Close,
    })
    .await;
    let (manager, connection) = connected(&server.base).await;
    let (events, events_in) = unbounded_channel();
    let (end, end_in) = unbounded_channel();
    let mut watching = Watching {
        events: events_in,
        end: end_in,
    };

    manager
        .watch_start(&connection, "/api/v1/pods", events, end)
        .expect("a watch");

    let ending = watching.finished().await;
    assert_eq!(ending.error, None);

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(server.count(), 1, "a stream that ended is not reconnected");
}

#[tokio::test]
async fn a_broken_stream_is_asked_again_from_the_version_it_reached() {
    // The first connection delivers one event and then breaks; the second one carries on from the
    // version that event named, so nothing that happened in between is lost.
    let server = fake(|_, connection| match connection {
        1 => Answer::Stream {
            status: 200,
            lines: vec![
                r#"{"type":"ADDED","object":{"kind":"Pod","metadata":{"name":"web","resourceVersion":"10"}}}"#
                    .to_owned(),
            ],
            then: Ending::Abort,
        },
        _ => Answer::Stream {
            status: 200,
            lines: vec![
                r#"{"type":"MODIFIED","object":{"kind":"Pod","metadata":{"name":"web","resourceVersion":"11"}}}"#
                    .to_owned(),
            ],
            then: Ending::Close,
        },
    })
    .await;
    let (manager, connection) = connected(&server.base).await;
    let (events, events_in) = unbounded_channel();
    let (end, end_in) = unbounded_channel();
    let mut watching = Watching {
        events: events_in,
        end: end_in,
    };

    let stream_id = manager
        .watch_start(&connection, "/api/v1/pods", events, end)
        .expect("a watch");

    let first = watching.next_event().await;
    assert_eq!(first.kind, "ADDED");
    assert_eq!(first.stream_id, stream_id);

    let second = watching.next_event().await;
    assert_eq!(second.kind, "MODIFIED");
    assert_eq!(
        second.stream_id, stream_id,
        "a reconnect continues the same stream, rather than starting another one"
    );

    let ending = watching.finished().await;
    assert_eq!(ending.error, None, "the second attempt ended cleanly");

    assert_eq!(server.count(), 2, "a broken stream is asked for again");
    let requests = server.requests();
    assert!(
        requests[1].path.contains("resourceVersion=10"),
        "the retry has to continue from the last version seen: {}",
        requests[1].path
    );
}

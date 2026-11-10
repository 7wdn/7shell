//! The request path: what goes out, what comes back, and what is refused before either.
//!
//! The fake server here is a socket the test controls line by line, because the interesting
//! questions are the ones a real API server would hide: is the token on this request, did the
//! kubeconfig's path prefix survive, is a non-2xx answer returned rather than thrown, and does a
//! body that never ends stop somewhere.

mod common;

use sevenshell_error::ErrorCode;
use sevenshell_k8s::{K8sManager, Kubeconfig};

use common::{Answer, Ending, Scratch, document, fake};

async fn connected(base: &str) -> (K8sManager, String) {
    let config = Kubeconfig::parse(document(base, "t0ken").as_bytes()).expect("a kubeconfig");
    let manager = K8sManager::new();
    let id = manager.connect(&config, None).await.expect("a connection");
    (manager, id)
}

#[tokio::test]
async fn a_request_carries_the_bearer_token_and_asks_for_json() {
    let server = fake(|_, _| Answer::Once {
        status: 200,
        body: "{}".to_owned(),
    })
    .await;
    let (manager, id) = connected(&server.base).await;

    let answer = manager
        .request(&id, "GET", "/api/v1/pods", None, None)
        .await
        .expect("an answer");

    assert_eq!(answer.status, 200);
    let seen = server.last();
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.path, "/api/v1/pods");
    assert_eq!(seen.header("authorization"), Some("Bearer t0ken"));
    assert_eq!(seen.header("accept"), Some("application/json"));
}

#[tokio::test]
async fn a_request_passes_the_method_body_and_content_type_through_and_returns_the_status_as_it_came() {
    // A conflict is an answer, not a failure: the interface decides what a 409 means for the object
    // it just tried to change.
    let server = fake(|_, _| Answer::Once {
        status: 409,
        body: r#"{"kind":"Status","code":409}"#.to_owned(),
    })
    .await;
    let (manager, id) = connected(&server.base).await;

    let patch = br#"{"spec":{"replicas":3}}"#;
    let answer = manager
        .request(
            &id,
            "PATCH",
            "/apis/apps/v1/namespaces/default/deployments/web",
            Some(patch),
            Some("application/merge-patch+json"),
        )
        .await
        .expect("a 409 is still an answer");

    assert_eq!(answer.status, 409);
    assert!(answer.body.contains("Status"), "the body should be the server's own");
    let seen = server.last();
    assert_eq!(seen.method, "PATCH");
    assert_eq!(seen.path, "/apis/apps/v1/namespaces/default/deployments/web");
    assert_eq!(seen.header("content-type"), Some("application/merge-patch+json"));
    assert_eq!(seen.body, patch.to_vec());
}

#[tokio::test]
async fn the_path_a_kubeconfig_names_stays_in_front_of_every_request() {
    // An API server behind a proxy is at `https://host/clusters/lab`, and a client that dropped the
    // prefix would answer 404 for every object on it.
    let server = fake(|_, _| Answer::Once {
        status: 200,
        body: "{}".to_owned(),
    })
    .await;
    let prefixed = format!("{}/clusters/lab", server.base);
    let (manager, id) = connected(&prefixed).await;

    manager
        .request(&id, "GET", "/api/v1/pods", None, None)
        .await
        .expect("an answer");

    assert_eq!(server.last().path, "/clusters/lab/api/v1/pods");
}

#[tokio::test]
async fn a_response_body_over_the_cap_is_refused_instead_of_buffered() {
    // 64 MiB is the cap; one byte more is a server this side will not follow into memory.
    const CHUNK: usize = 64 * 1024;
    let limit = 64 * 1024 * 1024 + 1;
    let server = fake(move |_, _| Answer::Endless {
        chunk: vec![b'x'; CHUNK],
        limit,
    })
    .await;
    let (manager, id) = connected(&server.base).await;

    let error = manager
        .request(&id, "GET", "/api/v1/pods", None, None)
        .await
        .expect_err("an endless body is refused");

    assert_eq!(error.code(), ErrorCode::K8sRequestFailed);
    assert!(
        format!("{error}").contains("64 MiB"),
        "the refusal should say which limit was hit: {error}"
    );
}

#[tokio::test]
async fn a_path_that_is_not_absolute_is_refused_before_anything_is_sent() {
    let server = fake(|_, _| Answer::Once {
        status: 200,
        body: "{}".to_owned(),
    })
    .await;
    let (manager, id) = connected(&server.base).await;

    let error = manager
        .request(&id, "GET", "api/v1/pods", None, None)
        .await
        .expect_err("a relative path is refused");

    assert_eq!(error.code(), ErrorCode::AppInternal);
    assert_eq!(server.count(), 0, "nothing should have been sent");
}

#[tokio::test]
async fn a_request_after_a_disconnect_reports_not_connected() {
    let server = fake(|_, _| Answer::Once {
        status: 200,
        body: "{}".to_owned(),
    })
    .await;
    let (manager, id) = connected(&server.base).await;

    assert!(manager.disconnect(&id), "the connection was open");
    let error = manager
        .request(&id, "GET", "/api/v1/pods", None, None)
        .await
        .expect_err("a closed connection cannot send");

    assert_eq!(error.code(), ErrorCode::K8sNotConnected);
    assert!(!manager.disconnect(&id), "a second disconnect has nothing to close");
}

#[tokio::test]
async fn an_api_server_that_is_not_listening_is_reported_as_unavailable() {
    // A port that was bound and released: nothing is listening on it any more.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("an address").port();
    drop(listener);

    let (manager, id) = connected(&format!("http://127.0.0.1:{port}")).await;
    let error = manager
        .request(&id, "GET", "/api/v1/pods", None, None)
        .await
        .expect_err("nothing is listening");

    assert_eq!(error.code(), ErrorCode::K8sUnavailable);
}

#[tokio::test]
async fn a_kubeconfig_read_from_disk_resolves_its_certificate_paths_beside_itself() {
    // The shape every generated kubeconfig has: `certificate-authority: ca.crt` next to the file.
    let scratch = Scratch::new();
    scratch.write("ca.crt", &common::pem("ca1.pem"));
    let path = scratch.write(
        "config",
        r#"apiVersion: v1
kind: Config
current-context: test
clusters:
  - name: test-cluster
    cluster:
      server: https://127.0.0.1:6443
      certificate-authority: ca.crt
contexts:
  - name: test
    context: {cluster: test-cluster, user: test-user}
users:
  - name: test-user
    user: {token: t0ken}
"#
        .as_bytes(),
    );

    let config = Kubeconfig::load(&path).expect("a kubeconfig with a relative CA");
    assert_eq!(config.contexts().len(), 1);

    // The certificate is only read because a path was resolved; a client built from this document
    // would otherwise have no authority at all.
    let client = sevenshell_k8s::K8sClient::connect(&config, None)
        .await
        .expect("the address and the CA are both usable");
    drop(client);
}

#[tokio::test]
async fn a_streamed_answer_is_read_to_its_end() {
    // The same code path a watch uses for one line: chunked rather than sized.
    let server = fake(|_, _| Answer::Stream {
        status: 200,
        lines: vec![r#"{"kind":"PodList","items":[]}"#.to_owned()],
        then: Ending::Close,
    })
    .await;
    let (manager, id) = connected(&server.base).await;

    let answer = manager
        .request(&id, "GET", "/api/v1/pods", None, None)
        .await
        .expect("an answer");

    assert_eq!(answer.status, 200);
    assert!(answer.body.contains("PodList"));
}

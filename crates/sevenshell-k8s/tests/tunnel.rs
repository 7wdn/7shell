//! A cluster reached through something that is not this machine's network.
//!
//! The dialer stands in for the SSH channel: what is under test here is the contract between the
//! client and whatever carries it — the URL keeps the cluster's host, the listener gets every
//! connection, and the dialer is asked for exactly the server the kubeconfig named.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use sevenshell_error::{ErrorCode, ShellError};
use sevenshell_k8s::{ApiDialer, BoxedApiStream, DialFuture, K8sManager, Kubeconfig, TunneledCluster};
use tokio::net::TcpStream;

use common::{pem, tls_server};

/// A dialer that answers with a socket to a server on this machine, recording what it was asked.
struct LocalDialer {
    address: SocketAddr,
    asked: Mutex<Vec<(String, u16)>>,
}

impl LocalDialer {
    fn new(address: SocketAddr) -> Arc<Self> {
        Arc::new(Self {
            address,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<(String, u16)> {
        self.asked.lock().expect("the asked lock").clone()
    }
}

impl ApiDialer for LocalDialer {
    fn dial(&self, host: String, port: u16) -> DialFuture {
        self.asked.lock().expect("the asked lock").push((host, port));
        let address = self.address;
        Box::pin(async move {
            let stream = TcpStream::connect(address)
                .await
                .map_err(|error| ShellError::k8s_unavailable(format!("the test tunnel could not connect: {error}")))?;
            let stream: BoxedApiStream = Box::new(stream);
            Ok(stream)
        })
    }
}

/// A kubeconfig for a one-context cluster at `server`, trusting the first test authority.
fn document(server: &str) -> String {
    use base64::Engine as _;
    let authority = base64::engine::general_purpose::STANDARD.encode(pem("ca1.pem"));
    format!(
        r#"apiVersion: v1
kind: Config
current-context: test
clusters:
  - name: test-cluster
    cluster:
      server: {server}
      certificate-authority-data: {authority}
contexts:
  - name: test
    context:
      cluster: test-cluster
      user: test-user
users:
  - name: test-user
    user:
      token: not-a-real-token
"#
    )
}

/// The address of the fake API server, out of the base URL it reports.
fn address_of(base: &str) -> SocketAddr {
    base.trim_start_matches("https://")
        .parse()
        .expect("the fake server's address")
}

#[tokio::test]
async fn a_cluster_reached_through_a_dialer_sends_its_requests_through_it() {
    let far = tls_server(&pem("server1.pem"), &pem("server1.key"));

    // Port 1 is what the kubeconfig names: nothing listens there, so a request that skipped the
    // tunnel could not succeed. The certificate the handshake is checked against belongs to the
    // test authority, which means TLS went to the far side of the tunnel, not to the listener.
    let dialer = LocalDialer::new(address_of(&far.base));
    let config = Kubeconfig::parse(document("https://127.0.0.1:1").as_bytes()).expect("a kubeconfig");
    let prepared = TunneledCluster::prepare(&config, None).expect("a tunneled cluster");

    let manager = K8sManager::new();
    let shared: Arc<dyn ApiDialer> = dialer.clone();
    let connection = manager.connect_via(prepared, shared).await.expect("connect");

    let answer = manager
        .request(&connection, "GET", "/api/v1/pods", None, None)
        .await
        .expect("the request should be carried");
    assert_eq!(answer.status, 200, "the fake API server answered {}", answer.status);
    assert_eq!(answer.body, "{}", "the answer came from the far side of the tunnel");
    assert_eq!(
        dialer.asked(),
        vec![("127.0.0.1".to_owned(), 1)],
        "the dialer was not asked for the server the kubeconfig named"
    );

    assert!(manager.disconnect(&connection), "the connection was open");
    let error = manager
        .request(&connection, "GET", "/api/v1/pods", None, None)
        .await
        .expect_err("a disconnected cluster is gone");
    assert_eq!(error.code(), ErrorCode::K8sNotConnected);
}

#[tokio::test]
async fn a_named_server_is_asked_for_by_name_so_the_certificate_still_matches_it() {
    let far = tls_server(&pem("server1.pem"), &pem("server1.key"));
    let dialer = LocalDialer::new(address_of(&far.base));
    let config = Kubeconfig::parse(document("https://localhost:1").as_bytes()).expect("a kubeconfig");
    let prepared = TunneledCluster::prepare(&config, None).expect("a tunneled cluster");

    let manager = K8sManager::new();
    let shared: Arc<dyn ApiDialer> = dialer.clone();
    let connection = manager.connect_via(prepared, shared).await.expect("connect");

    let answer = manager
        .request(&connection, "GET", "/api/v1/pods", None, None)
        .await
        .expect("the request should be carried");
    assert_eq!(
        answer.status, 200,
        "the certificate was checked against the name the kubeconfig gave"
    );
    assert_eq!(
        dialer.asked(),
        vec![("localhost".to_owned(), 1)],
        "the tunnel was not asked for the name the kubeconfig gave"
    );

    manager.disconnect(&connection);
}

//! Certificates: which authority is trusted, and which identity is presented.
//!
//! These tests speak real TLS against a real handshake, because the question they answer cannot be
//! answered by inspecting a configuration: a kubeconfig that names a cluster's own authority must
//! connect to *that* cluster, and must not connect to one holding a certificate from somewhere else.
//! The certificates are committed beside this file and signed by two authorities, neither of which
//! any machine trusts.

mod common;

use sevenshell_error::ErrorCode;
use sevenshell_k8s::{K8sManager, Kubeconfig};

use common::{Scratch, pem, tls_server};

fn document(server: &str, cluster_authority: Option<&str>, user: &str) -> String {
    let mut yaml = format!(
        r#"apiVersion: v1
kind: Config
current-context: test
clusters:
  - name: test-cluster
    cluster:
      server: {server}
"#
    );
    if let Some(authority) = cluster_authority {
        yaml.push_str(&format!("      certificate-authority-data: {authority}\n"));
    }
    yaml.push_str(
        r#"contexts:
  - name: test
    context:
      cluster: test-cluster
      user: test-user
users:
  - name: test-user
    user:
"#,
    );
    yaml.push_str(user);
    yaml.push('\n');
    yaml
}

/// The base64 body of a committed certificate, which is how a kubeconfig carries one.
fn authority(name: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(pem(name))
}

async fn request_against(yaml: &str) -> Result<u16, sevenshell_error::ShellError> {
    let config = Kubeconfig::parse(yaml.as_bytes()).expect("a kubeconfig");
    let manager = K8sManager::new();
    let id = manager.connect(&config, None).await?;
    manager
        .request(&id, "GET", "/api/v1/pods", None, None)
        .await
        .map(|answer| answer.status)
}

#[tokio::test]
async fn the_clusters_own_authority_is_what_its_certificate_is_checked_against() {
    // The server's certificate is signed by an authority this machine has never heard of, so the
    // connection can only succeed because the kubeconfig named it.
    let server = tls_server(&pem("server1.pem"), &pem("server1.key"));
    let yaml = document(&server.base, Some(&authority("ca1.pem")), "      token: t0ken");

    let status = request_against(&yaml).await.expect("the named authority is trusted");
    assert_eq!(status, 200);
    assert_eq!(server.connections.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_certificate_from_another_authority_is_refused_even_when_it_is_valid() {
    // The same kubeconfig, but the server answers with a certificate signed by a second authority:
    // a valid certificate is not the same thing as the right one.
    let server = tls_server(&pem("server2.pem"), &pem("server2.key"));
    let yaml = document(&server.base, Some(&authority("ca1.pem")), "      token: t0ken");

    let error = request_against(&yaml).await.expect_err("the wrong cluster is refused");
    assert_eq!(error.code(), ErrorCode::K8sUnavailable);
}

#[tokio::test]
async fn a_kubeconfig_without_an_authority_falls_back_to_the_machines_own_roots() {
    // No authority named is the ordinary public-certificate case; a certificate nobody trusts is
    // still refused, which is what proves the fallback is the system store and not "anything goes".
    let server = tls_server(&pem("server1.pem"), &pem("server1.key"));
    let yaml = document(&server.base, None, "      token: t0ken");

    let error = request_against(&yaml)
        .await
        .expect_err("a certificate the machine does not trust is refused");
    assert_eq!(error.code(), ErrorCode::K8sUnavailable);
}

#[tokio::test]
async fn insecure_skip_tls_verify_accepts_what_no_authority_signed() {
    let server = tls_server(&pem("server2.pem"), &pem("server2.key"));
    let yaml = format!(
        r#"apiVersion: v1
kind: Config
current-context: test
clusters:
  - name: test-cluster
    cluster:
      server: {}
      insecure-skip-tls-verify: true
contexts:
  - name: test
    context: {{cluster: test-cluster, user: test-user}}
users:
  - name: test-user
    user: {{token: t0ken}}
"#,
        server.base
    );

    let status = request_against(&yaml).await.expect("the person asked for this");
    assert_eq!(status, 200);
}

#[tokio::test]
async fn a_client_identity_needs_both_halves_and_a_real_key() {
    let server = tls_server(&pem("server1.pem"), &pem("server1.key"));
    let authority = authority("ca1.pem");

    // Both halves: the connection is built with an identity attached.
    let scratch = Scratch::new();
    scratch.write("client.pem", &pem("client.pem"));
    scratch.write("client.key", &pem("client.key"));
    let complete = document(
        &server.base,
        Some(&authority),
        "      client-certificate-data: Y2xpZW50\n      client-key-data: a2V5",
    );
    // The PEM above is not a real certificate pair; what matters first is that the *shape* is read
    // as a pair, so this is refused for being unreadable rather than for being half an identity.
    let error = request_against(&complete).await.expect_err("not a real certificate");
    assert_eq!(error.code(), ErrorCode::K8sKubeconfigInvalid);

    // One half only: refused by the kubeconfig reader, by name, before any connection.
    let half = document(
        &server.base,
        Some(&authority),
        "      client-certificate-data: Y2xpZW50",
    );
    let config = Kubeconfig::parse(half.as_bytes()).expect_err("a certificate without its key");
    assert_eq!(config.code(), ErrorCode::K8sKubeconfigInvalid);

    // A key that is not a PEM document is refused by name, before any connection.
    let paths = format!(
        "      client-certificate: {}\n      client-key: {}",
        scratch.path().join("client.pem").display(),
        scratch.path().join("client-not-a-key.key").display()
    );
    scratch.write("client-not-a-key.key", b"not a key");
    let malformed = document(&server.base, Some(&authority), &paths);
    let error = request_against(&malformed)
        .await
        .expect_err("a key that is not PEM is not readable");
    assert_eq!(error.code(), ErrorCode::K8sKubeconfigInvalid);

    // A real PKCS#8 pair builds, and the handshake still works: the identity is attached rather
    // than the connection being refused for having one.
    let working = document(
        &server.base,
        Some(&authority),
        &format!(
            "      client-certificate: {}\n      client-key: {}",
            scratch.path().join("client.pem").display(),
            scratch.path().join("client.key").display()
        ),
    );
    let status = request_against(&working).await.expect("a readable identity");
    assert_eq!(status, 200);
}

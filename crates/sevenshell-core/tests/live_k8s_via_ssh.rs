//! A real cluster reached through a real sshd: the tunnel end to end.
//!
//! Ignored by default. It needs the SSH lab (`deploy/lab/openssh`) and the k3s lab
//! (`deploy/lab/kubernetes`), and the kubeconfig the latter writes:
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 SEVENSHELL_TEST_USER=labuser \
//!   SEVENSHELL_TEST_PASSWORD=labpassword \
//!   SEVENSHELL_TEST_KUBECONFIG=.scratch/k3s/kubeconfig-original.yaml \
//!   SEVENSHELL_TEST_K8S_VIA_SERVER=https://host.docker.internal:6443 \
//!   cargo test -p sevenshell-core --test live_k8s_via_ssh -- --ignored --test-threads=1
//! ```
//!
//! `SEVENSHELL_TEST_K8S_VIA_SERVER` is the address the **far side** reaches the API server by. In
//! the Docker Desktop lab that is `host.docker.internal:6443`: the k3s container publishes 6443 on
//! the host's loopback, and the only route from inside the sshd container to that port is the
//! host's own address. The name is not in the cluster's certificate — that was issued for the
//! loopback address the kubeconfig was generated with — so the rewritten kubeconfig skips server
//! verification. The TLS handshake itself still crosses the tunnel, which is what this test is
//! about.

use std::sync::Arc;
use std::time::Duration;

use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, K8sMethod, K8sRequest, KubeconfigSource, KubernetesSettings, Session, SessionAuth, TerminalSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};

struct Lab {
    /// Owned so the vault file outlives the test.
    _directory: TempDir,
    core: Arc<Core>,
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// The lab's coordinates, or a message saying why this test cannot run.
fn lab() -> Option<Lab> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return None;
    };
    let Ok(port) = port.parse::<u16>() else {
        eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
        return None;
    };
    let (Some(kubeconfig), Some(server)) = (
        environment("SEVENSHELL_TEST_KUBECONFIG"),
        environment("SEVENSHELL_TEST_K8S_VIA_SERVER"),
    ) else {
        eprintln!("the cluster lab is not configured; skipping (set SEVENSHELL_TEST_KUBECONFIG/..K8S_VIA_SERVER)");
        return None;
    };
    let document = std::fs::read(&kubeconfig).expect("read the kubeconfig");
    let document = rewritten(&String::from_utf8(document).expect("the kubeconfig is text"), &server);

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    vault
        .save_session(
            session("gateway", sevenshell_types::Protocol::Ssh, &host, port, &user),
            Some(SessionSecret::Password(password)),
        )
        .expect("save the ssh session");

    let mut cluster = session("cluster", sevenshell_types::Protocol::Kubernetes, "", 0, "");
    cluster.kubernetes = KubernetesSettings {
        source: KubeconfigSource::Imported,
        context: None,
        namespace: "sevenshell-lab".to_owned(),
        via_ssh: Some("gateway".to_owned()),
    };
    vault
        .save_session(cluster, Some(SessionSecret::Kubeconfig(document)))
        .expect("save the cluster session");

    let (core, _events) = Core::new(Arc::clone(&vault));
    Some(Lab {
        _directory: directory,
        core,
    })
}

fn session(id: &str, protocol: sevenshell_types::Protocol, host: &str, port: u16, user: &str) -> Session {
    Session {
        id: id.to_owned(),
        name: id.to_owned(),
        group_id: None,
        protocol,
        host: host.to_owned(),
        port,
        user: user.to_owned(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        desktop: sevenshell_types::DesktopSettings::default(),
        telnet: sevenshell_types::TelnetSettings::default(),
        serial: sevenshell_types::SerialSettings::default(),
        local: sevenshell_types::LocalSettings::default(),
        wsl: sevenshell_types::WslSettings::default(),
        container: sevenshell_types::ContainerSettings::default(),
        kubernetes: KubernetesSettings::default(),
        webdav: sevenshell_types::WebDavSettings::default(),
        ftp: sevenshell_types::FtpSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    }
}

/// The lab kubeconfig with its server address replaced and server verification relaxed.
///
/// The replacement is textual because the document is a fixture written by k3s, and re-encoding it
/// would be a second YAML reader to keep in step with the first. The certificate authority stays
/// in the document: a connection with `insecure-skip-tls-verify` does not consult it.
fn rewritten(document: &str, server: &str) -> String {
    let mut lines = Vec::new();
    let mut replaced = false;
    for line in document.lines() {
        if line.trim_start().starts_with("server:") {
            let indent = &line[..line.len() - line.trim_start().len()];
            lines.push(format!("{indent}server: {server}"));
            lines.push(format!("{indent}insecure-skip-tls-verify: true"));
            replaced = true;
        } else {
            lines.push(line.to_owned());
        }
    }
    assert!(replaced, "the kubeconfig names no server to replace");
    lines.join("\n") + "\n"
}

#[tokio::test]
#[ignore = "needs the SSH and Kubernetes labs; see the module docs"]
async fn a_real_cluster_is_reached_through_a_real_ssh_session() {
    let Some(lab) = lab() else {
        return;
    };

    // The probe sends nothing, which is why the key it reports is the key the connection will
    // see; trusting it is what a person who compared the fingerprint does.
    let host = environment("SEVENSHELL_TEST_HOST").expect("checked by lab()");
    let port = environment("SEVENSHELL_TEST_PORT")
        .and_then(|port| port.parse().ok())
        .expect("checked by lab()");
    let presented = sevenshell_ssh::probe_host_key(&host, port, Duration::from_secs(15))
        .await
        .expect("the lab presents a host key");
    lab.core.trust_host_key(&presented).expect("trust the lab");

    let connection = lab
        .core
        .k8s_connect("cluster")
        .await
        .expect("the cluster should be reached through the ssh session");

    let version = lab
        .core
        .k8s_request(
            &connection,
            K8sRequest {
                method: K8sMethod::Get,
                path: "/version".to_owned(),
                body: None,
                content_type: None,
                acknowledged: false,
            },
        )
        .await
        .expect("the request should be carried through the tunnel");
    assert_eq!(version.status, 200, "the API server answered {}", version.status);
    assert!(
        version.body.contains("gitVersion"),
        "the answer is not a version document: {}",
        &version.body[..version.body.len().min(200)]
    );

    // A watch is the stream-shaped half of the same connection: it stays open through the tunnel
    // and is stopped by an id, which is what a tab that goes away relies on.
    let (events, mut watch_events) = tokio::sync::mpsc::unbounded_channel();
    let (end, _end_events) = tokio::sync::mpsc::unbounded_channel();
    let stream = lab
        .core
        .k8s_watch_start(&connection, "/api/v1/namespaces/sevenshell-lab/pods", events, end)
        .expect("a watch on the lab namespace");
    let event = tokio::time::timeout(Duration::from_secs(20), watch_events.recv())
        .await
        .expect("the watch should report the pods that are there")
        .expect("the watch channel is open");
    assert!(
        !event.kind.is_empty() && !event.object.is_empty(),
        "the watch event carried neither a kind nor an object: {event:?}"
    );
    assert_eq!(event.stream_id, stream);
    lab.core.k8s_stream_stop(&stream);

    lab.core.k8s_disconnect(&connection);
    let error = lab
        .core
        .k8s_request(
            &connection,
            K8sRequest {
                method: K8sMethod::Get,
                path: "/version".to_owned(),
                body: None,
                content_type: None,
                acknowledged: false,
            },
        )
        .await
        .expect_err("a disconnected cluster is gone");
    assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sNotConnected);
}

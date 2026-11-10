//! A real Kubernetes API server: connection, list, logs and an exec terminal.
//!
//! Ignored by default. `deploy/lab/kubernetes/compose.yaml` starts a one-node k3s; the kubeconfig
//! is read from `SEVENSHELL_TEST_KUBECONFIG`.
//!
//! ```text
//! SEVENSHELL_TEST_KUBECONFIG=.scratch/k3s/kubeconfig-original.yaml \
//!   cargo test -p sevenshell-core --test live_k8s -- --ignored --test-threads=1
//! ```
//!
//! The k3s kubeconfig writes an SEC1 EC client key, which is exactly the shape the client has to
//! normalize before `native-tls` can use it. The pods named here are fixtures: `lab-shell` is a
//! busybox that sleeps, and `lab-logs` prints a line a second.

use std::sync::Arc;
use std::time::Duration;

use sevenshell_core::{Core, CoreEvent};
use sevenshell_session::TerminalEvent;
use sevenshell_types::{
    AuthMethod, K8sExecRequest, K8sLogRequest, K8sMethod, K8sRequest, KubeconfigSource, KubernetesSettings, Session,
    SessionAuth, TerminalSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;
use tokio::time::timeout;

const NAMESPACE: &str = "sevenshell-lab";

struct Cluster {
    _directory: TempDir,
    core: Arc<Core>,
    session_id: String,
    events: mpsc::Receiver<CoreEvent>,
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn cluster() -> Option<Cluster> {
    let Some(path) = environment("SEVENSHELL_TEST_KUBECONFIG") else {
        eprintln!("the Kubernetes lab is not configured; set SEVENSHELL_TEST_KUBECONFIG");
        return None;
    };
    let kubeconfig = std::fs::read(&path).expect("read the kubeconfig");

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    vault
        .save_session(
            Session {
                id: "lab-cluster".to_owned(),
                name: "lab cluster".to_owned(),
                group_id: None,
                protocol: sevenshell_types::Protocol::Kubernetes,
                host: String::new(),
                port: 0,
                user: String::new(),
                auth: SessionAuth::with_password(AuthMethod::Password),
                terminal: TerminalSettings::default(),
                desktop: sevenshell_types::DesktopSettings::default(),
                telnet: sevenshell_types::TelnetSettings::default(),
                serial: sevenshell_types::SerialSettings::default(),
                local: sevenshell_types::LocalSettings::default(),
                wsl: sevenshell_types::WslSettings::default(),
                container: sevenshell_types::ContainerSettings::default(),
                kubernetes: KubernetesSettings {
                    source: KubeconfigSource::Imported,
                    context: None,
                    namespace: NAMESPACE.to_owned(),
                    via_ssh: None,
                },
                webdav: sevenshell_types::WebDavSettings::default(),
                ftp: sevenshell_types::FtpSettings::default(),
                jump: Vec::new(),
                forwards: Vec::new(),
                updated_at: 0,
            },
            Some(SessionSecret::Kubeconfig(
                String::from_utf8(kubeconfig).expect("kubeconfig is text"),
            )),
        )
        .expect("save session");

    let (core, events) = Core::new(Arc::clone(&vault));
    Some(Cluster {
        _directory: directory,
        core,
        session_id: "lab-cluster".to_owned(),
        events,
    })
}

async fn read_until(events: &mut mpsc::Receiver<CoreEvent>, handle: &str, needle: &str) -> String {
    let deadline = Duration::from_secs(20);
    let mut collected = String::new();
    let _ = timeout(deadline, async {
        while let Some(event) = events.recv().await {
            if event.handle != handle {
                continue;
            }
            if let TerminalEvent::Data(bytes) = event.event {
                collected.push_str(&String::from_utf8_lossy(&bytes));
                if collected.contains(needle) {
                    return;
                }
            }
        }
    })
    .await;
    collected
}

#[tokio::test]
#[ignore = "needs a real cluster; set SEVENSHELL_TEST_KUBECONFIG"]
async fn a_real_cluster_lists_resources_follows_logs_and_runs_a_terminal() {
    let Some(mut lab) = cluster() else {
        return;
    };

    let connection = lab.core.k8s_connect(&lab.session_id).await.expect("connect");

    let nodes = lab
        .core
        .k8s_request(
            &connection,
            K8sRequest {
                method: K8sMethod::Get,
                path: "/api/v1/nodes".to_owned(),
                body: None,
                content_type: None,
                acknowledged: false,
            },
        )
        .await
        .expect("list nodes");
    assert_eq!(nodes.status, 200, "the API server answered {}", nodes.status);
    assert!(
        nodes.body.contains("\"items\""),
        "nodes are a list: {}",
        &nodes.body[..nodes.body.len().min(200)]
    );
    assert!(nodes.body.contains("k3s"), "the lab node is the k3s node");

    let pods = lab
        .core
        .k8s_request(
            &connection,
            K8sRequest {
                method: K8sMethod::Get,
                path: format!("/api/v1/namespaces/{NAMESPACE}/pods?limit=500"),
                body: None,
                content_type: None,
                acknowledged: false,
            },
        )
        .await
        .expect("list pods");
    assert_eq!(pods.status, 200);
    assert!(pods.body.contains("lab-shell"), "the exec fixture is listed");
    assert!(pods.body.contains("lab-logs"), "the log fixture is listed");

    let (lines, mut line_events) = mpsc::unbounded_channel();
    let (end, mut end_events) = mpsc::unbounded_channel();
    let stream = lab
        .core
        .k8s_logs_start(
            &connection,
            &K8sLogRequest {
                namespace: NAMESPACE.to_owned(),
                pod: "lab-logs".to_owned(),
                container: "lab-logs".to_owned(),
                tail_lines: 10,
                timestamps: false,
                previous: false,
            },
            lines,
            end,
        )
        .await
        .expect("follow logs");

    let mut seen = String::new();
    let mut ended = None;
    let _ = timeout(Duration::from_secs(20), async {
        loop {
            tokio::select! {
                Some(line) = line_events.recv() => {
                    seen.push_str(&line.line);
                    seen.push('\n');
                    if seen.contains("tick") {
                        return;
                    }
                }
                Some(end) = end_events.recv() => {
                    ended = end.error;
                    return;
                }
                else => return,
            }
        }
    })
    .await;
    assert!(
        seen.contains("tick"),
        "the followed log produced a line (end: {ended:?}): {seen}"
    );
    let _ = timeout(Duration::from_secs(5), end_events.recv()).await;
    lab.core.k8s_stream_stop(&stream);

    let handle = lab
        .core
        .k8s_exec_open(
            &connection,
            &K8sExecRequest {
                namespace: NAMESPACE.to_owned(),
                pod: "lab-shell".to_owned(),
                container: "lab-shell".to_owned(),
                command: String::new(),
            },
            80,
            24,
        )
        .await
        .expect("open a terminal");

    lab.core
        .write(&handle, b"echo k8s-exec-ok\n")
        .await
        .expect("write to the terminal");
    let echoed = read_until(&mut lab.events, &handle, "k8s-exec-ok").await;
    assert!(echoed.contains("k8s-exec-ok"), "the terminal answered: {echoed}");
    lab.core.close_terminal(&handle).await.expect("close the terminal");

    lab.core.k8s_disconnect(&connection);
}

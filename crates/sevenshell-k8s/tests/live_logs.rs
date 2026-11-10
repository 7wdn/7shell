//! A real pod log stream, without the core in between.
//!
//! Ignored by default; `SEVENSHELL_TEST_KUBECONFIG` points at a kubeconfig with permission to read
//! the `sevenshell-lab` namespace.

use std::time::Duration;

use sevenshell_k8s::{K8sClient, Kubeconfig, open_logs};
use sevenshell_types::K8sLogRequest;
use tokio::time::timeout;

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

#[tokio::test]
#[ignore = "needs a real cluster; set SEVENSHELL_TEST_KUBECONFIG"]
async fn a_real_pod_log_stream_delivers_lines() {
    let Some(path) = environment("SEVENSHELL_TEST_KUBECONFIG") else {
        eprintln!("the Kubernetes lab is not configured; skipping");
        return;
    };
    let bytes = std::fs::read(path).expect("read the kubeconfig");
    let config = Kubeconfig::parse(&bytes).expect("parse the kubeconfig");
    let client = K8sClient::connect(&config, None).await.expect("connect");

    let mut stream = open_logs(
        &client,
        &K8sLogRequest {
            namespace: "sevenshell-lab".to_owned(),
            pod: "lab-logs".to_owned(),
            container: "lab-logs".to_owned(),
            tail_lines: 10,
            timestamps: false,
            previous: false,
        },
    )
    .expect("follow the log");

    let mut seen = String::new();
    let _ = timeout(Duration::from_secs(20), async {
        while let Some(line) = stream.next_line().await {
            seen.push_str(&line);
            seen.push('\n');
            if seen.contains("tick") {
                return;
            }
        }
    })
    .await;

    assert!(
        seen.contains("tick"),
        "a line arrived (ended: {:?}): {seen}",
        stream.wait_ended().await
    );
}

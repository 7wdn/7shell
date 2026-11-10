//! Real Docker Desktop.
//!
//! Ignored by default: it needs an engine, an image, and the right to create containers. Run it
//! with `SEVENSHELL_TEST_DOCKER=1 cargo test -p sevenshell-container --test live_container --
//! --ignored`. It skips with a sentence, rather than failing, when the engine is not reachable or
//! the small image it uses cannot be pulled — a machine without Docker is not a defect in this
//! crate.

use std::time::Duration;

use sevenshell_container::{ContainerManager, Runner};
use sevenshell_types::{ContainerCreateRequest, ContainerImageTransfer, ContainerRuntime};

/// The image the lifecycle uses. Small enough to pull in seconds, and it has a shell.
const IMAGE: &str = "busybox:latest";
const NAME_PREFIX: &str = "7shell-live-";

fn engine_is_reachable() -> bool {
    if std::env::var("SEVENSHELL_TEST_DOCKER").as_deref() != Ok("1") {
        eprintln!("skipped: set SEVENSHELL_TEST_DOCKER=1 to run the live container test");
        return false;
    }
    let reachable = std::process::Command::new("docker")
        .arg("version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !reachable {
        eprintln!("skipped: the Docker engine is not reachable from this process");
    }
    reachable
}

#[tokio::test]
#[ignore = "needs Docker Desktop and SEVENSHELL_TEST_DOCKER=1"]
async fn a_real_engine_lists_images_runs_a_container_logs_into_it_and_removes_it() {
    if !engine_is_reachable() {
        return;
    }

    let manager = ContainerManager::new();
    let connection = manager.new_id();
    manager
        .connect(connection.clone(), ContainerRuntime::Docker, "", Runner::Local)
        .expect("the connection settings should be accepted");
    let provider = manager.provider(&connection).expect("the connection should be open");

    provider.list().await.expect("the container list should answer");
    let images = provider.images().await.expect("the image list should answer");
    let info = provider
        .runtime_info()
        .await
        .expect("the runtime should describe itself");
    assert!(
        !info.client_version.is_empty() || !info.server_version.is_empty(),
        "the runtime reported neither a client nor a server version: {info:?}"
    );

    let mut image_ready = images.iter().any(|row| row.repository == "busybox");
    if !image_ready {
        let request = ContainerImageTransfer {
            image: IMAGE.to_owned(),
            platform: String::new(),
            all_tags: false,
            insecure: false,
        };
        let mut pull = provider.image_pull(&request).await.expect("the pull should start");
        loop {
            match tokio::time::timeout(Duration::from_secs(120), pull.next()).await {
                Ok(Some(line)) => eprintln!("[pull] {line}"),
                Ok(None) => break,
                Err(_) => {
                    pull.stop();
                    break;
                }
            }
        }
        image_ready = provider
            .images()
            .await
            .map(|rows| rows.iter().any(|row| row.repository == "busybox"))
            .unwrap_or(false);
    }
    if !image_ready {
        eprintln!("skipped the lifecycle: {IMAGE} is not available on this engine");
        manager.remove(&connection);
        return;
    }

    let name = format!("{NAME_PREFIX}{}", std::process::id());
    let request = ContainerCreateRequest {
        image: IMAGE.to_owned(),
        name: name.clone(),
        ports: Vec::new(),
        volumes: Vec::new(),
        env: Vec::new(),
        restart_policy: "no".to_owned(),
        command: "sh -c 'echo live-ready; sleep 120'".to_owned(),
        interactive: false,
        tty: false,
    };
    provider
        .create(&request)
        .await
        .expect("the container should be created");

    // `run -d` returns as soon as the container exists; the list is what says which id it got.
    let mut container_id = None;
    for _ in 0..40 {
        if let Ok(rows) = provider.list().await
            && let Some(row) = rows.into_iter().find(|row| row.name == name)
        {
            container_id = Some(row.id);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let container_id = container_id.expect("the created container never appeared in the list");

    let outcome: Result<(), String> = async {
        let mut logs = provider
            .logs(&container_id, 50, true)
            .await
            .map_err(|error| format!("the log stream did not open: {error:?}"))?;
        let mut collected = String::new();
        let ready = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(line) = logs.next().await {
                collected.push_str(&line);
                collected.push('\n');
                if collected.contains("live-ready") {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        logs.stop();
        if !ready {
            return Err(format!(
                "the log stream never showed the container's first line:\n{collected}"
            ));
        }

        let samples = provider
            .stats()
            .await
            .map_err(|error| format!("stats did not answer: {error:?}"))?;
        if !samples
            .iter()
            .any(|sample| container_id.starts_with(&sample.id) || sample.id.starts_with(&container_id))
        {
            return Err(format!("the running container was missing from stats: {samples:?}"));
        }

        let mut terminal = provider
            .open_exec(&container_id, "sh", 100, 30)
            .await
            .map_err(|error| format!("the exec terminal did not open: {error:?}"))?;
        terminal
            .write(b"echo exec-ok\n")
            .await
            .map_err(|error| format!("writing to the exec terminal failed: {error:?}"))?;
        let mut seen = Vec::new();
        let echoed = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(chunk) = terminal.next_output().await {
                seen.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&seen).contains("exec-ok") {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        let _ = terminal.close().await;
        if !echoed {
            return Err(format!(
                "the exec terminal never echoed the command: {:?}",
                String::from_utf8_lossy(&seen)
            ));
        }

        provider
            .action(&container_id, "stop")
            .await
            .map_err(|error| format!("stop failed: {error:?}"))?;
        let mut exited = false;
        for _ in 0..40 {
            if let Ok(rows) = provider.list().await
                && rows.iter().any(|row| row.id == container_id && row.state == "exited")
            {
                exited = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !exited {
            return Err("the container never reported itself as exited".to_owned());
        }

        provider
            .action(&container_id, "start")
            .await
            .map_err(|error| format!("start failed: {error:?}"))?;
        let mut running = false;
        for _ in 0..40 {
            if let Ok(rows) = provider.list().await
                && rows.iter().any(|row| row.id == container_id && row.state == "running")
            {
                running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !running {
            return Err("the container never reported itself as running again".to_owned());
        }
        Ok(())
    }
    .await;

    // Cleanup happens whatever the outcome: a failed live test should not leave a container behind.
    let _ = provider.action(&container_id, "stop").await;
    let _ = provider.action(&container_id, "remove").await;
    manager.remove(&connection);

    if let Err(message) = outcome {
        panic!("{message}");
    }

    let mut still_listed = false;
    for _ in 0..40 {
        if let Ok(rows) = provider.list().await
            && rows.iter().any(|row| row.id == container_id || row.name == name)
        {
            still_listed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(!still_listed, "the container was removed but is still listed");
}

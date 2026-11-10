//! A container runtime reached over SSH, against the lab's `docker:dind` service.
//!
//! Ignored by default. The lab's sshd is the transport; the runtime is the sibling dind service
//! (`DOCKER_HOST=tcp://docker:2375`), so the commands really run on the far side.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=3222 \
//! SEVENSHELL_TEST_USER=labuser SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-core --test live_container -- --ignored --test-threads=1
//! ```

use std::sync::Arc;
use std::time::Duration;

use sevenshell_core::{Core, CoreEvent};
use sevenshell_session::TerminalEvent;
use sevenshell_types::{
    AuthMethod, ContainerCreateRequest, ContainerRuntime, ContainerSettings, ContainerTransport, OpenOutcome, Protocol,
    Session, SessionAuth, TerminalSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;
use tokio::time::timeout;

struct Lab {
    _directory: TempDir,
    core: Arc<Core>,
    ssh_session: String,
    container_session: String,
    events: mpsc::Receiver<CoreEvent>,
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn session(id: &str, name: &str, protocol: Protocol) -> Session {
    Session {
        id: id.to_owned(),
        name: name.to_owned(),
        group_id: None,
        protocol,
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
        kubernetes: sevenshell_types::KubernetesSettings::default(),
        webdav: sevenshell_types::WebDavSettings::default(),
        ftp: sevenshell_types::FtpSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    }
}

fn lab() -> Option<Lab> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD");
        return None;
    };

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    let mut transport = session("lab-ssh", "lab ssh", Protocol::Ssh);
    transport.host = host;
    transport.port = port.parse().expect("a port");
    transport.user = user;
    vault
        .save_session(transport, Some(SessionSecret::Password(password)))
        .expect("save the transport session");

    let mut container = session("lab-container", "lab docker", Protocol::Container);
    container.container = ContainerSettings {
        runtime: ContainerRuntime::Docker,
        transport: ContainerTransport::Ssh,
        ssh_session: "lab-ssh".to_owned(),
        namespace: String::new(),
    };
    vault.save_session(container, None).expect("save the container session");

    let (core, events) = Core::new(Arc::clone(&vault));
    Some(Lab {
        _directory: directory,
        core,
        ssh_session: "lab-ssh".to_owned(),
        container_session: "lab-container".to_owned(),
        events,
    })
}

async fn trust_transport(lab: &Lab) {
    for _ in 0..4 {
        match lab
            .core
            .open_terminal(&lab.ssh_session)
            .await
            .expect("open the transport")
        {
            OpenOutcome::Opened { handle, .. } => {
                lab.core.close_terminal(&handle).await.expect("close the transport");
                return;
            }
            OpenOutcome::HostKeyUnknown { key } => lab.core.trust_host_key(&key).expect("trust the lab key"),
            other => panic!("expected the transport to open, got {other:?}"),
        }
    }
    panic!("the lab host key was never trusted");
}

async fn read_until(events: &mut mpsc::Receiver<CoreEvent>, handle: &str, needle: &str) -> String {
    let mut collected = String::new();
    let _ = timeout(Duration::from_secs(20), async {
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
#[ignore = "needs the SSH lab and its dind service"]
async fn a_container_runtime_over_ssh_lists_creates_and_execs() {
    let Some(mut lab) = lab() else {
        return;
    };
    trust_transport(&lab).await;

    let connection = lab
        .core
        .container_connect(&lab.container_session)
        .await
        .expect("connect");

    let initial = lab.core.container_list(&connection).await.expect("list containers");
    assert!(
        initial.iter().all(|container| container.name != "sevenshell-live"),
        "the fixture is created by this test, not by an earlier run"
    );

    let create = ContainerCreateRequest {
        image: "busybox:1.36".to_owned(),
        name: "sevenshell-live".to_owned(),
        ports: Vec::new(),
        volumes: Vec::new(),
        env: Vec::new(),
        restart_policy: String::new(),
        command: "sleep 300".to_owned(),
        interactive: false,
        tty: false,
    };
    lab.core
        .container_create(&connection, &create)
        .await
        .expect("create a container on the far side");

    let containers = lab.core.container_list(&connection).await.expect("list after create");
    let created = containers
        .iter()
        .find(|container| container.name == "sevenshell-live")
        .expect("the created container is listed");
    assert_eq!(created.image, "busybox:1.36");
    assert!(
        created.state.contains("running") || created.state == "running",
        "state: {}",
        created.state
    );

    let stats = lab.core.container_stats(&connection).await.expect("stats");
    assert!(stats.iter().any(|sample| sample.name == "sevenshell-live"));

    let handle = lab
        .core
        .container_exec_open(&connection, &created.id, "sh", 80, 24)
        .await
        .expect("open a terminal in the container");
    lab.core
        .write(&handle, b"echo container-over-ssh-ok\n")
        .await
        .expect("write to the terminal");
    let echoed = read_until(&mut lab.events, &handle, "container-over-ssh-ok").await;
    assert!(
        echoed.contains("container-over-ssh-ok"),
        "the container terminal answered: {echoed}"
    );
    lab.core.close_terminal(&handle).await.expect("close the terminal");

    lab.core
        .container_action(&connection, &created.id, "stop", true)
        .await
        .expect("stop the container");
    lab.core
        .container_action(&connection, &created.id, "remove", true)
        .await
        .expect("remove the container");

    let final_list = lab.core.container_list(&connection).await.expect("list after remove");
    assert!(final_list.iter().all(|container| container.name != "sevenshell-live"));

    lab.core.container_disconnect(&connection).await;
}

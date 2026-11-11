//! A real registry that asks for a password.
//!
//! The fixture is `deploy/lab/registry`: `registry:2` behind htpasswd on loopback. This test starts
//! its own container from the same `htpasswd` file the compose recipe mounts, so the preconditions
//! are Docker and two small images; a machine without either skips with a sentence rather than
//! failing, because that is an environment, not a defect in this crate.
//!
//! Docker treats a loopback registry as insecure HTTP by default, which is what makes
//! `127.0.0.1:5000` usable without certificates; a remote registry would need TLS instead.
//!
//! ```text
//! SEVENSHELL_TEST_DOCKER=1 cargo test -p sevenshell-container --test live_registry -- --ignored --test-threads=1
//! ```

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sevenshell_container::{ContainerManager, LineStream, Provider, Runner};
use sevenshell_error::ErrorCode;
use sevenshell_types::{ContainerImageTransfer, ContainerRegistryLogin, ContainerRuntime, SecretInput};

/// Loopback only, and outside this machine's Hyper-V reserved ranges (1779-2978, 9211-10010).
const REGISTRY: &str = "127.0.0.1:5000";
const REGISTRY_IMAGE: &str = "registry:2";
/// The same trivial credentials as the SSH lab, and the hash in `deploy/lab/registry/htpasswd`.
const USER: &str = "labuser";
const PASSWORD: &str = "labpassword";
/// Small enough to push in seconds, and tagged explicitly so the test never pushes `latest` of
/// whatever the machine happens to have.
const IMAGE: &str = "busybox:1.36";
const REPOSITORY: &str = "127.0.0.1:5000/sevenshell-live";

/// Both tests drive the same loopback port, and the isolated `DOCKER_CONFIG` is process-wide.
/// Running them one at a time is what the documented command asks for; taking this lock makes a
/// plain `cargo test` correct too, which is why it is a `tokio` lock and not a standard one: a
/// standard guard cannot be held across the awaits below.
static PORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn engine_is_reachable() -> bool {
    if std::env::var("SEVENSHELL_TEST_DOCKER").as_deref() != Ok("1") {
        eprintln!("skipped: set SEVENSHELL_TEST_DOCKER=1 to run the live registry test");
        return false;
    }
    let reachable = docker(&["version"]).is_some_and(|output| output.status.success());
    if !reachable {
        eprintln!("skipped: the Docker engine is not reachable from this process");
    }
    reachable
}

fn image_or_pull(image: &str) -> bool {
    if docker(&["image", "inspect", image]).is_some_and(|output| output.status.success()) {
        return true;
    }
    let pulled = docker(&["pull", image]).is_some_and(|output| output.status.success());
    if !pulled {
        eprintln!("skipped: {image} is not on this engine and could not be pulled");
    }
    pulled
}

fn docker(arguments: &[&str]) -> Option<Output> {
    Command::new("docker").args(arguments).output().ok()
}

fn docker_provider() -> Arc<Provider> {
    let manager = ContainerManager::new();
    let connection = manager.new_id();
    manager
        .connect(connection.clone(), ContainerRuntime::Docker, "", Runner::Local)
        .expect("the connection settings should be accepted");
    manager.provider(&connection).expect("the connection should be open")
}

fn login(password: &str) -> ContainerRegistryLogin {
    ContainerRegistryLogin {
        registry: REGISTRY.to_owned(),
        username: USER.to_owned(),
        secret: SecretInput {
            value: password.to_owned(),
        },
        insecure: false,
    }
}

fn transfer(image: &str) -> ContainerImageTransfer {
    ContainerImageTransfer {
        image: image.to_owned(),
        platform: String::new(),
        all_tags: false,
        insecure: false,
    }
}

/// Reads the registry's answer to an unauthenticated `GET /v2/`.
///
/// A registry with any auth configured answers 401 `WWW-Authenticate`; that is both "it is up" and
/// "the htpasswd file was accepted", so it is a better readiness gate than an open TCP port.
fn registry_challenges() -> bool {
    let Ok(address) = REGISTRY.parse::<SocketAddr>() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if stream
        .write_all(b"GET /v2/ HTTP/1.1\r\nHost: 127.0.0.1:5000\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut answer = String::new();
    stream.read_to_string(&mut answer).is_ok() && answer.starts_with("HTTP/1.1 401")
}

/// The fixture directory, spelled the way Docker Desktop accepts it as a bind source.
fn fixture_directory() -> Option<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/lab/registry");
    match std::path::absolute(&path) {
        Ok(path) => Some(path.display().to_string().replace('\\', "/")),
        Err(error) => {
            eprintln!(
                "skipped: the registry fixture at {} is unusable: {error}",
                path.display()
            );
            None
        }
    }
}

/// A registry container started by this test, removed when the test ends, whatever it asserted.
///
/// `container` is `None` when the compose recipe already published the port: that recipe mounts the
/// same `htpasswd`, so it is the same fixture and must not be stopped here.
struct Fixture {
    container: Option<String>,
}

impl Fixture {
    async fn start(suffix: &str) -> Option<Self> {
        if registry_challenges() {
            return Some(Self { container: None });
        }
        if !image_or_pull(REGISTRY_IMAGE) {
            return None;
        }
        let directory = fixture_directory()?;
        let name = format!("7shell-live-registry-{suffix}-{}", std::process::id());
        // A container left behind by a killed run still owns the name. `-v` because the registry's
        // storage is an anonymous volume, and one per run would accumulate silently.
        let _ = docker(&["rm", "-f", "-v", &name]);
        let mount = format!("{directory}:/auth:ro");
        let Some(output) = docker(&[
            "run",
            "-d",
            "--name",
            &name,
            "-p",
            "127.0.0.1:5000:5000",
            "-v",
            &mount,
            "-e",
            "REGISTRY_AUTH=htpasswd",
            "-e",
            "REGISTRY_AUTH_HTPASSWD_REALM=7shell-live",
            "-e",
            "REGISTRY_AUTH_HTPASSWD_PATH=/auth/htpasswd",
            REGISTRY_IMAGE,
        ]) else {
            eprintln!("skipped: the docker CLI could not be started");
            return None;
        };
        if !output.status.success() {
            eprintln!(
                "skipped: the registry container did not start, so the port may already belong to something else: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return None;
        }

        let fixture = Self {
            container: Some(name.clone()),
        };
        for _ in 0..120 {
            if registry_challenges() {
                return Some(fixture);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let logs = docker(&["logs", &name])
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        panic!("the registry container never answered with an authentication challenge:\n{logs}");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(name) = self.container.take() {
            let _ = docker(&["rm", "-f", "-v", &name]);
        }
    }
}

/// `docker login` stores what it was given under `DOCKER_CONFIG`. Pointing that at a temporary
/// directory keeps the test from writing a credential into the developer's own
/// `~/.docker/config.json`, and makes the push depend on the login this test performed instead of
/// on something the machine already had.
struct IsolatedDockerConfig {
    previous: Option<std::ffi::OsString>,
    directory: PathBuf,
}

impl IsolatedDockerConfig {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "7shell-live-registry-config-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&directory);
        let previous = std::env::var_os("DOCKER_CONFIG");
        // SAFETY: the caller holds `PORT_LOCK` for the whole test, so the two tests in this binary
        // never overlap, and the only user of the variable is the child process started below.
        unsafe { std::env::set_var("DOCKER_CONFIG", &directory) };
        Self { previous, directory }
    }
}

impl Drop for IsolatedDockerConfig {
    fn drop(&mut self) {
        // SAFETY: see `new`; the lock is still held while this runs.
        unsafe {
            match self.previous.as_ref() {
                Some(value) => std::env::set_var("DOCKER_CONFIG", value),
                None => std::env::remove_var("DOCKER_CONFIG"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Collects a transfer's lines until the command ends, stopping it if it outlives the deadline.
async fn drain(stream: &mut LineStream) -> Vec<String> {
    let mut lines = Vec::new();
    let finished = tokio::time::timeout(Duration::from_secs(180), async {
        while let Some(line) = stream.next().await {
            lines.push(line);
        }
    })
    .await;
    if finished.is_err() {
        stream.stop();
    }
    lines
}

#[tokio::test]
#[ignore = "needs Docker Desktop, registry:2 and SEVENSHELL_TEST_DOCKER=1"]
async fn a_local_registry_accepts_the_configured_password_and_refuses_a_wrong_one() {
    if !engine_is_reachable() {
        return;
    }
    let _port = PORT_LOCK.lock().await;
    let _config = IsolatedDockerConfig::new();
    let Some(_fixture) = Fixture::start("login").await else {
        return;
    };
    let provider = docker_provider();

    provider
        .registry_login(&login(PASSWORD))
        .await
        .expect("the registry should accept the password in the fixture's htpasswd file");

    let error = provider
        .registry_login(&login("not-the-password"))
        .await
        .expect_err("a wrong password must not be accepted");
    assert_eq!(
        error.code(),
        ErrorCode::ContainerCommandFailed,
        "the refusal should be the CLI's own failure, not something else: {error:?}"
    );
    let rendered = format!("{error:?}");
    assert!(
        !rendered.contains("not-the-password"),
        "the refused password came back in the error: {rendered}"
    );
}

#[tokio::test]
#[ignore = "needs Docker Desktop, registry:2 and SEVENSHELL_TEST_DOCKER=1"]
async fn an_image_pushed_to_a_local_registry_pulls_back_after_its_local_tag_is_removed() {
    if !engine_is_reachable() || !image_or_pull(IMAGE) {
        return;
    }
    let _port = PORT_LOCK.lock().await;
    let _config = IsolatedDockerConfig::new();
    let Some(_fixture) = Fixture::start("push").await else {
        return;
    };
    let provider = docker_provider();
    provider
        .registry_login(&login(PASSWORD))
        .await
        .expect("the registry should accept the password in the fixture's htpasswd file");

    let tag = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
        .to_string();
    let target = format!("{REPOSITORY}:{tag}");
    // A tag from a killed run would make "the image is back" pass on something that never left.
    let _ = provider.image_remove(&target).await;

    provider
        .image_tag(IMAGE, &target)
        .await
        .expect("the tagged image should be accepted");

    let mut push = provider
        .image_push(&transfer(&target))
        .await
        .expect("the push should start");
    let pushed = drain(&mut push).await;
    let push_log = pushed.join("\n");
    assert!(
        push_log.contains("digest: sha256:"),
        "the push never reported a stored digest:\n{push_log}"
    );

    provider
        .image_remove(&target)
        .await
        .expect("the local tag should be removable");
    let after_removal = provider.images().await.expect("the image list should answer");
    assert!(
        !after_removal
            .iter()
            .any(|row| row.repository == REPOSITORY && row.tag == tag),
        "the tag survived its own removal, so the pull below would prove nothing: {after_removal:?}"
    );

    let mut pull = provider
        .image_pull(&transfer(&target))
        .await
        .expect("the pull should start");
    let pulled = drain(&mut pull).await;
    let pull_log = pulled.join("\n");
    let after_pull = provider.images().await.expect("the image list should answer");
    assert!(
        after_pull
            .iter()
            .any(|row| row.repository == REPOSITORY && row.tag == tag),
        "the image was not listed again after the pull:\n{pull_log}"
    );

    let _ = provider.image_remove(&target).await;
}

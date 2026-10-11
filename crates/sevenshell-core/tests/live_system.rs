//! What a real host says it is, asked over a real connection.
//!
//! Ignored by default: it needs a running sshd. `deploy/lab/` has one and `make lab-up` starts it.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 SEVENSHELL_TEST_USER=labuser \
//!   SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-core --test live_system -- --ignored
//! ```
//!
//! What only a real server can show: that the script the identity probe runs is one a login shell
//! accepts, that its unquoted markers survive the trip through the shell that runs it, and that a
//! machine which describes itself in `/etc/os-release` comes back with an id a table can be keyed
//! on. The parsing is proved against text in `src/system.rs`; this is about the far side answering
//! at all.
//!
//! The assertions stop at "a real distribution answered", rather than naming the lab's own: the
//! container is a small Alpine image today, and a test that asserts today's base image is a test
//! that fails the day somebody rebuilds the lab on something else — for a reason that has nothing
//! to do with what it is checking.

use std::sync::Arc;

use sevenshell_core::Core;
use sevenshell_types::{AuthMethod, OpenOutcome, Session, SessionAuth, TerminalSettings};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;

struct Lab {
    /// Owned so the vault file outlives the test.
    _directory: TempDir,
    core: Arc<Core>,
    session_id: String,
    /// Kept alive so the terminal's output has somewhere to go; the probe never reads it.
    events: mpsc::Receiver<sevenshell_core::CoreEvent>,
}

impl Lab {
    /// Opens a terminal, trusting whatever host key it is asked about, the way a person who has
    /// compared the fingerprint would.
    async fn open_trusting_everything(&self) -> String {
        for _ in 0..4 {
            match self.core.open_terminal(&self.session_id).await.expect("open") {
                OpenOutcome::Opened { handle, .. } => return handle,
                OpenOutcome::HostKeyUnknown { key } => self.core.trust_host_key(&key).expect("trust"),
                other => panic!("expected the session to open, got {other:?}"),
            }
        }
        panic!("the route never finished asking");
    }
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// A vault holding one session for the lab, with its password stored.
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
    let port: u16 = port.parse().ok()?;

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    vault
        .save_session(
            Session {
                id: "lab-session".to_owned(),
                name: "lab".to_owned(),
                group_id: None,
                protocol: sevenshell_types::Protocol::Ssh,
                host,
                port,
                user,
                auth: SessionAuth::with_password(AuthMethod::Password),
                terminal: TerminalSettings::default(),
                jump: Vec::new(),
                forwards: Vec::new(),
                updated_at: 0,
            },
            Some(SessionSecret::Password(password)),
        )
        .expect("save session");

    let (core, events) = Core::new(Arc::clone(&vault));
    Some(Lab {
        _directory: directory,
        core,
        session_id: "lab-session".to_owned(),
        events,
    })
}

/// Empties the terminal's output, so a login banner is not still queued when the probe answers: the
/// registry forwards a terminal's bytes over the same channel this test holds the other end of.
fn drain(events: &mut mpsc::Receiver<sevenshell_core::CoreEvent>) {
    while events.try_recv().is_ok() {}
}

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab/"]
async fn a_real_host_says_which_system_it_is() {
    let Some(mut lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    drain(&mut lab.events);

    let system = lab
        .core
        .host_system(&handle)
        .await
        .expect("an identity from a live host");

    assert_eq!(
        system.kernel.as_deref(),
        Some("Linux"),
        "the lab is a Linux container, and uname is the answer every Linux host gives: {system:?}"
    );

    let distro = system.distro.as_deref().expect("a Linux host keeps /etc/os-release");
    assert!(!distro.is_empty(), "an id with nothing in it is not an id: {system:?}");
    assert_eq!(
        distro,
        distro.to_lowercase(),
        "ids are published in lower case: {system:?}"
    );
    assert!(
        !distro.contains("@@") && !distro.contains('='),
        "the id is a value, not a line of the file it came from: {system:?}"
    );

    let name = system.name.as_deref().expect("and it names itself too");
    assert!(
        !name.is_empty() && !name.contains("@@"),
        "the marker that introduced the file must not survive into the name: {system:?}"
    );

    // `ID_LIKE` is optional — Alpine publishes none — but what is there has to be ids as well.
    for like in &system.distro_like {
        assert_eq!(like, &like.to_lowercase(), "families are ids too: {system:?}");
        assert!(!like.contains("@@"), "and values rather than lines: {system:?}");
    }

    // The same question costs one round trip and is safe to ask twice: nothing here is state.
    let again = lab.core.host_system(&handle).await.expect("asked again");
    assert_eq!(again, system, "the answer is the machine, not the moment");
}

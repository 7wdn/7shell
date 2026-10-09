//! Keys this application makes, held up against OpenSSH's own tools.
//!
//! The unit tests in `keygen.rs` prove that a generated key reads back as itself *through the same
//! library that wrote it*, which is a closed circle: a writer and a reader that agree with each other
//! and with nothing else would pass them. These tests leave that circle. The private half is written
//! to a file and handed to `ssh-keygen`, the program every server administrator already has, and what
//! it says about the key is compared with what this application said. The last test goes further and
//! puts the key on a real sshd.
//!
//! ```text
//! cargo test -p sevenshell-ssh --test live_keygen -- --ignored --nocapture
//!
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 \
//!   SEVENSHELL_TEST_USER=labuser SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-ssh --test live_keygen -- --ignored --nocapture
//! ```
//!
//! The first form needs only `ssh-keygen`, which is part of any OpenSSH installation. The second adds
//! the lab from `deploy/lab/openssh`, and with it the test that a server accepts a key made here.
//! Without the binary, or without the four variables, the tests say so and skip.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use sevenshell_ssh::{
    Credential, OpenRequest, Purpose, Route, SshSession, Target, TerminalSize, generate, probe_host_key,
};
use sevenshell_types::KeyAlgorithm;
use tempfile::TempDir;

const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// The comment on every key these tests make, which is also how the lab's `authorized_keys` is
/// cleaned up again: nothing else in that file was written by this run.
const COMMENT: &str = "7shell-live-keygen";

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// Whether `ssh-keygen` is here, said out loud when it is not.
fn ssh_keygen_works() -> bool {
    match Command::new("ssh-keygen").arg("-?").output() {
        Ok(_) => true,
        Err(error) => {
            eprintln!("ssh-keygen is not installed; skipping ({error})");
            false
        }
    }
}

/// Writes a private key where `ssh-keygen` can read it.
///
/// On unix the file is made private first: `ssh-keygen` refuses a key anybody could read, and a test
/// that failed on file permissions would be saying something about the test rather than the key.
fn key_file(area: &TempDir, name: &str, contents: &str) -> PathBuf {
    let path = area.path().join(name);
    std::fs::write(&path, contents).expect("the key file is written");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("made private");
    }

    path
}

/// What `ssh-keygen` says when asked to describe a key file.
fn ssh_keygen(args: &[&str], path: &Path) -> String {
    let output = Command::new("ssh-keygen")
        .args(args)
        .arg(path)
        .output()
        .expect("ssh-keygen runs");

    assert!(
        output.status.success(),
        "ssh-keygen {args:?} refused the key this application made: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
#[ignore = "needs ssh-keygen on this machine"]
fn openssh_reads_every_kind_of_key_this_application_makes() {
    if !ssh_keygen_works() {
        return;
    }

    let area = TempDir::new().expect("a temporary directory");

    for algorithm in KeyAlgorithm::ALL {
        let key = generate(*algorithm, COMMENT, None).expect("a generated key");
        let path = key_file(&area, algorithm.as_str(), key.private_text());

        let described = ssh_keygen(&["-l", "-f"], &path);

        // `ssh-keygen -l` prints "3072 SHA256:… comment (RSA)" or "256 SHA256:… comment (ED25519)".
        // The fingerprint is the part a person compares, and it has to be the one this application
        // reported — that is the whole reason the vault stores it.
        assert!(
            described.contains(&key.description().fingerprint),
            "{algorithm}: ssh-keygen read {} where this application reported {}",
            described,
            key.description().fingerprint
        );
        assert!(
            described.contains(COMMENT),
            "{algorithm}: the comment did not survive the round trip: {described}"
        );
        assert!(
            described.contains(ssh_keygen_kind(*algorithm)),
            "{algorithm}: ssh-keygen called the key something else: {described}"
        );
    }
}

/// What `ssh-keygen -l` calls a kind of key, in the parenthesis at the end of its line.
///
/// Not the key type and not the short name this application stores: `ssh-keygen` prints `(ECDSA)`
/// where the type is `ecdsa-sha2-nistp256`, because the line is meant for a person's eye.
fn ssh_keygen_kind(algorithm: KeyAlgorithm) -> &'static str {
    match algorithm {
        KeyAlgorithm::Ed25519 => "ED25519",
        KeyAlgorithm::Ecdsa => "ECDSA",
        KeyAlgorithm::Rsa => "RSA",
    }
}

#[test]
#[ignore = "needs ssh-keygen on this machine"]
fn openssh_derives_the_same_public_half_that_this_application_reports() {
    if !ssh_keygen_works() {
        return;
    }

    let area = TempDir::new().expect("a temporary directory");

    for algorithm in KeyAlgorithm::ALL {
        let key = generate(*algorithm, COMMENT, None).expect("a generated key");
        let path = key_file(&area, algorithm.as_str(), key.private_text());

        // `-y` reads the private key and prints the public one, which is exactly the derivation a
        // server does when it checks a signature. A public line that disagreed here would be a key
        // that can never authenticate, and the failure would look like the server's fault.
        let derived = ssh_keygen(&["-y", "-f"], &path);

        assert_eq!(
            derived,
            key.public_line(),
            "{algorithm}: the public half this application reports is not the one OpenSSH derives"
        );
    }
}

#[test]
#[ignore = "needs ssh-keygen on this machine"]
fn openssh_opens_a_protected_key_with_the_passphrase_it_was_given() {
    if !ssh_keygen_works() {
        return;
    }

    let area = TempDir::new().expect("a temporary directory");
    let key = generate(KeyAlgorithm::Ed25519, COMMENT, Some("a passphrase for the test")).expect("a generated key");
    let path = key_file(&area, "protected", key.private_text());

    // Without the passphrase there is no public half to print, because OpenSSH's encrypted container
    // keeps even that behind the passphrase — the one place this differs from a key with none.
    let refused = Command::new("ssh-keygen")
        .args(["-y", "-f"])
        .arg(&path)
        .output()
        .expect("ssh-keygen runs");
    assert!(
        !refused.status.success(),
        "ssh-keygen printed a public key for a locked key, which means the key is not locked"
    );

    // With it, the public half is the one this application reports.
    let derived = Command::new("ssh-keygen")
        .args(["-y", "-f"])
        .arg(&path)
        .arg("-P")
        .arg("a passphrase for the test")
        .output()
        .expect("ssh-keygen runs");
    assert!(
        derived.status.success(),
        "ssh-keygen could not open the key with the passphrase it was given: {}",
        String::from_utf8_lossy(&derived.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&derived.stdout).trim(),
        key.public_line(),
        "the public half reported with the passphrase is not the one this application reports"
    );
    assert!(
        derived.stderr.is_empty(),
        "ssh-keygen had something to say about the key: {}",
        String::from_utf8_lossy(&derived.stderr)
    );
}

#[tokio::test]
#[ignore = "needs the SSH lab: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD"]
async fn a_server_lets_in_a_key_this_application_made() {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return;
    };
    let Ok(port) = port.parse::<u16>() else {
        eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
        return;
    };

    let target = Target { host, port, user };
    let presented = probe_host_key(&target.host, target.port, OPEN_TIMEOUT)
        .await
        .expect("probe the host key");

    // One connection with the password, to install the public half; a second with the key itself, to
    // see whether the server accepts it. Two connections rather than one reconfigured session,
    // because "the server accepts this key" is the claim being tested.
    let open = |credential: Credential| {
        let presented = presented.clone();
        let target = target.clone();
        async move {
            let request = OpenRequest {
                target,
                credential,
                terminal_type: String::new(),
                keepalive_seconds: 0,
                forward_agent: false,
                timeout: OPEN_TIMEOUT,
                expected_host_key: presented,
                size: TerminalSize::default(),
                route: Route::Direct,
                purpose: Purpose::Carrier,
            };
            let (events, _unused) = tokio::sync::mpsc::channel(1);
            SshSession::open(request, events).await
        }
    };

    let made = generate(KeyAlgorithm::Ed25519, COMMENT, None).expect("a generated key");

    // A control, and the reason this test is not satisfied by "the server let something in": a key the
    // server has never been told about must be refused. This runs first, while `authorized_keys` holds
    // nothing from this run.
    let unknown = generate(KeyAlgorithm::Ed25519, COMMENT, None).expect("another generated key");
    let stranger = open(Credential::Key {
        contents: unknown.private_text().to_owned(),
        passphrase: None,
    })
    .await;
    assert!(
        stranger.is_err(),
        "the server accepted a key it was never given, so nothing this test proves would be about the key"
    );

    let installing = open(Credential::Password(password))
        .await
        .expect("connect with the password");
    let installing = Arc::new(installing);
    let line = made.public_line().to_owned();
    installing
        .exec(
            &format!(
                "mkdir -p ~/.ssh && chmod 700 ~/.ssh && printf '%s\\n' '{line}' >> ~/.ssh/authorized_keys && \
                 chmod 600 ~/.ssh/authorized_keys"
            ),
            RUN_TIMEOUT,
        )
        .await
        .expect("the public half goes into authorized_keys");

    let accepted = open(Credential::Key {
        contents: made.private_text().to_owned(),
        passphrase: None,
    })
    .await;

    match accepted {
        Ok(session) => {
            let session = Arc::new(session);
            let outcome = session
                .exec("echo hello-from-a-generated-key", RUN_TIMEOUT)
                .await
                .expect("the command runs");
            assert_eq!(
                outcome.stdout, "hello-from-a-generated-key\n",
                "the server authenticated the key but the session is not usable"
            );

            // The line is taken back out: the lab is a container somebody may keep between runs, and
            // leaving keys in its authorized_keys would make the control above fail next time.
            let removed = installing
                .exec(
                    &format!("sed -i '/{COMMENT}/d' ~/.ssh/authorized_keys; echo removed"),
                    RUN_TIMEOUT,
                )
                .await
                .expect("the line is removed again");
            assert_eq!(removed.stdout, "removed\n");
        }
        Err(error) => panic!("the server refused a key this application made: {error}"),
    }
}

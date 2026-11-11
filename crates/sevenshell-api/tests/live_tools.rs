//! The tools against a real server.
//!
//! Ignored by default: these need a running sshd. `deploy/lab/` has one and `make lab-up` starts it.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 SEVENSHELL_TEST_USER=labuser \
//!   SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-api --test live_tools -- --ignored --test-threads=1
//! ```
//!
//! What only a real server can show: that an unknown host key reaches a person and an approval
//! produces a trusted entry, that a changed key stops everything without asking, that a refused
//! destructive command leaves the far side untouched, and that the file tools and the command runner
//! reuse one authenticated connection through the cache.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use sevenshell_api::local;
use sevenshell_api::{
    ApprovalBroker, ApprovalDecision, ApprovalReason, ApprovalRequest, ToolCall, ToolContext, ToolError, ToolOutcome,
};
use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, ContainerSettings, DesktopSettings, FtpSettings, KubernetesSettings, LocalSettings, McpScope,
    PresentedHostKey, Protocol, SerialSettings, Session, SessionAuth, TelnetSettings, TerminalSettings, WebDavSettings,
    WslSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};

/// A broker that answers from a script and remembers every question.
#[derive(Default)]
struct ScriptedBroker {
    decisions: Mutex<VecDeque<ApprovalDecision>>,
    asked: Mutex<Vec<ApprovalRequest>>,
}

impl ScriptedBroker {
    fn new(decisions: &[ApprovalDecision]) -> Arc<Self> {
        Arc::new(Self {
            decisions: Mutex::new(decisions.iter().copied().collect()),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<ApprovalRequest> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl ApprovalBroker for ScriptedBroker {
    async fn request(&self, request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        self.asked.lock().expect("not poisoned").push(request);
        Ok(self
            .decisions
            .lock()
            .expect("not poisoned")
            .pop_front()
            .unwrap_or(ApprovalDecision::Denied))
    }
}

struct Lab {
    /// Owned so the vault file outlives the test.
    _directory: TempDir,
    vault: Arc<Vault>,
    context: ToolContext,
    broker: Arc<ScriptedBroker>,
    host: String,
    port: u16,
}

impl Lab {
    async fn call(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        let outcome = local::default_registry()
            .call(
                &self.context,
                ToolCall {
                    name: name.to_owned(),
                    arguments,
                },
            )
            .await;

        match outcome {
            ToolOutcome::Succeeded { result } => Ok(result.value),
            ToolOutcome::Failed { error } => Err(error),
        }
    }

    /// Closes whatever the test opened, so the lab is left the way it was found.
    async fn close(&self) {
        for open in self.context.core().open_terminals().await {
            let _ = self.context.core().close_terminal(&open.handle).await;
        }
        self.context.cache().close_all().await;
    }
}

/// A lab with one saved SSH session called `lab`, and no host key trusted yet.
fn lab(decisions: &[ApprovalDecision]) -> Option<Lab> {
    let (host, port, user, password) = credentials()?;

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    let session = Session {
        id: "lab".to_owned(),
        name: "lab".to_owned(),
        group_id: None,
        protocol: Protocol::Ssh,
        host: host.clone(),
        port,
        user,
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        desktop: DesktopSettings::default(),
        telnet: TelnetSettings::default(),
        serial: SerialSettings::default(),
        local: LocalSettings::default(),
        wsl: WslSettings::default(),
        container: ContainerSettings::default(),
        kubernetes: KubernetesSettings::default(),
        webdav: WebDavSettings::default(),
        ftp: FtpSettings::default(),
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 0,
    };
    vault
        .save_session(session, Some(SessionSecret::Password(password)))
        .expect("save session");

    let core = Core::new(Arc::clone(&vault)).0;
    let broker = ScriptedBroker::new(decisions);
    let context = ToolContext::new(Arc::clone(&core), McpScope::SafeWrite)
        .with_approvals(Arc::clone(&broker) as Arc<dyn ApprovalBroker>);

    Some(Lab {
        _directory: directory,
        vault,
        context,
        broker,
        host,
        port,
    })
}

fn credentials() -> Option<(String, u16, String, String)> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return None;
    };

    match port.parse() {
        Ok(port) => Some((host, port, user, password)),
        Err(_) => {
            eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
            None
        }
    }
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

// -- Host keys ----------------------------------------------------------

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab"]
async fn an_unknown_key_is_asked_about_and_the_answer_is_remembered() {
    let Some(lab) = lab(&[ApprovalDecision::Approved]) else {
        return;
    };

    let opened = lab
        .call("sshell_session_open", json!({ "sessionId": "lab" }))
        .await
        .expect("the session opens once the key is trusted");

    assert_eq!(opened["opened"], json!(true));
    assert_eq!(opened["alreadyOpen"], json!(false));

    let asked = lab.broker.asked();
    assert_eq!(asked.len(), 1, "one key, one question");
    assert!(
        matches!(&asked[0].reason, ApprovalReason::HostKeyUnknown { key } if !key.fingerprint.is_empty()),
        "the question has to carry the key the server presented"
    );
    assert!(
        lab.vault
            .find_known_host(&lab.host, lab.port)
            .expect("the vault is readable")
            .is_some(),
        "an approval writes the key into the vault"
    );

    // The second call opens nothing: the session already has a terminal.
    let again = lab
        .call("sshell_session_open", json!({ "sessionId": "lab" }))
        .await
        .expect("a session that is open answers without opening again");
    assert_eq!(again["alreadyOpen"], json!(true));
    assert_eq!(lab.broker.asked().len(), 1, "and asks nobody anything");

    lab.close().await;
}

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab"]
async fn a_changed_key_stops_the_open_and_asks_nobody() {
    let Some(lab) = lab(&[ApprovalDecision::Approved]) else {
        return;
    };

    // A key that is not the server's: whatever the lab presents now differs from it, which is the
    // situation this test is about.
    lab.context
        .core()
        .trust_host_key(&PresentedHostKey {
            host: lab.host.clone(),
            port: lab.port,
            key_type: "ssh-ed25519".to_owned(),
            public_key: "AAAAC3NzaC1lZDI1NTE5AAAAIthisIsNotTheServersKey".to_owned(),
            fingerprint: "SHA256:not-the-servers-fingerprint".to_owned(),
        })
        .expect("a key can be trusted");

    let error = lab
        .call("sshell_session_open", json!({ "sessionId": "lab" }))
        .await
        .expect_err("a changed key is a stop");

    assert_eq!(error.code(), "SSH-HOSTKEY-MISMATCH");
    assert!(
        lab.broker.asked().is_empty(),
        "a changed key must never become a question somebody can wave through"
    );
    assert!(
        error
            .message()
            .is_some_and(|message| message.contains("Nothing was sent")),
        "the agent is told that the credential was withheld: {:?}",
        error.message()
    );

    lab.close().await;
}

// -- Commands -----------------------------------------------------------

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab"]
async fn a_command_answers_with_its_streams_and_its_exit_status() {
    let Some(lab) = lab(&[ApprovalDecision::Approved]) else {
        return;
    };

    // The first command meets the unknown host key, so the approving broker is spent here.
    let ran = lab
        .call(
            "sshell_exec_run",
            json!({ "sessionId": "lab", "command": "printf 'seven-shell'" }),
        )
        .await
        .expect("a harmless command runs");

    assert_eq!(ran["exitCode"], json!(0));
    assert_eq!(ran["stdout"], json!("seven-shell"));
    assert_eq!(ran["stderr"], json!(""));
    assert_eq!(ran["truncated"]["stdout"], json!(false));
    assert_eq!(ran["risk"]["level"], json!("safe"));

    let failed = lab
        .call("sshell_exec_run", json!({ "sessionId": "lab", "command": "exit 3" }))
        .await
        .expect("a command that says no is still an answer");
    assert_eq!(failed["exitCode"], json!(3));

    lab.close().await;
}

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab"]
async fn a_refused_destructive_command_leaves_the_far_side_alone() {
    // The first answer is the host key, the second is the command: the key question is asked
    // before the command can run, so the script has to answer both.
    let Some(lab) = lab(&[ApprovalDecision::Approved, ApprovalDecision::Denied]) else {
        return;
    };
    let scratch = "/tmp/seven-shell-mcp-scratch";

    lab.call(
        "sshell_exec_run",
        json!({ "sessionId": "lab", "command": format!("mkdir -p {scratch}") }),
    )
    .await
    .expect("the marker directory is made");

    let error = lab
        .call(
            "sshell_exec_run",
            json!({ "sessionId": "lab", "command": format!("rm -rf {scratch}") }),
        )
        .await
        .expect_err("an unapproved destructive command does not run");

    assert_eq!(error.code(), "POLICY-DENIED");
    assert_eq!(lab.broker.asked().len(), 2, "the key and the command");

    let still_there = lab
        .call(
            "sshell_exec_run",
            json!({ "sessionId": "lab", "command": format!("test -d {scratch} && echo yes") }),
        )
        .await
        .expect("the check runs");
    assert_eq!(
        still_there["stdout"], "yes\n",
        "a refused command must not have run on the far side"
    );

    lab.call(
        "sshell_exec_run",
        json!({ "sessionId": "lab", "command": format!("rmdir {scratch}") }),
    )
    .await
    .expect("the marker directory is removed");

    lab.close().await;
}

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab"]
async fn a_batch_answers_one_row_for_the_session_it_named() {
    let Some(lab) = lab(&[ApprovalDecision::Approved]) else {
        return;
    };

    // Opened first so the host key is trusted and the batch is about the command alone.
    lab.call("sshell_session_open", json!({ "sessionId": "lab" }))
        .await
        .expect("the session opens");

    let value = lab
        .call(
            "sshell_batch_exec",
            json!({ "sessionIds": ["lab"], "command": "printf 'batched'" }),
        )
        .await
        .expect("a batch runs");

    let rows = value["results"].as_array().expect("one row per session");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["sessionId"], json!("lab"));
    assert_eq!(rows[0]["ok"], json!(true));
    assert_eq!(rows[0]["stdout"], json!("batched"));

    lab.close().await;
}

// -- Files --------------------------------------------------------------

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab"]
async fn a_file_is_written_read_and_listed_through_one_connection() {
    let Some(lab) = lab(&[ApprovalDecision::Approved]) else {
        return;
    };
    let path = format!("/tmp/seven-shell-mcp-{}.txt", std::process::id());
    let text = "hello from the bridge\n";

    // The file tools have no host-key prompt of their own: opening the session once is how a person
    // is asked, and every later channel reuses the trust.
    lab.call("sshell_session_open", json!({ "sessionId": "lab" }))
        .await
        .expect("the session opens");

    let written = lab
        .call(
            "sshell_sftp_write_text",
            json!({ "sessionId": "lab", "path": path, "text": text }),
        )
        .await
        .expect("the file is written");
    assert_eq!(written["bytes"], json!(text.len()));
    assert_eq!(written["path"], json!(path));

    let read = lab
        .call("sshell_sftp_read_text", json!({ "sessionId": "lab", "path": path }))
        .await
        .expect("the file is read");
    assert_eq!(read["text"], json!(text));

    let listing = lab
        .call("sshell_sftp_list", json!({ "sessionId": "lab", "path": "/tmp" }))
        .await
        .expect("the directory is listed");
    let name = path.rsplit('/').next().expect("a file name");
    assert!(
        listing["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|entry| entry["name"] == json!(name)),
        "the file that was just written is in the listing"
    );
    assert!(
        listing["total"].as_u64().unwrap_or(0) <= 500 || listing["truncated"] == json!(true),
        "a listing never returns more rows than the limit"
    );

    lab.call(
        "sshell_exec_run",
        json!({ "sessionId": "lab", "command": format!("rm -f {path}") }),
    )
    .await
    .expect("the file is removed");

    lab.close().await;
}

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab"]
async fn a_file_too_large_to_return_is_refused_with_its_size() {
    let Some(lab) = lab(&[ApprovalDecision::Approved]) else {
        return;
    };
    let path = format!("/tmp/seven-shell-mcp-large-{}.txt", std::process::id());
    let text = "x".repeat(300 * 1024);

    lab.call("sshell_session_open", json!({ "sessionId": "lab" }))
        .await
        .expect("the session opens");
    lab.call(
        "sshell_sftp_write_text",
        json!({ "sessionId": "lab", "path": path, "text": text }),
    )
    .await
    .expect("the file is written");

    let error = lab
        .call("sshell_sftp_read_text", json!({ "sessionId": "lab", "path": path }))
        .await
        .expect_err("a file past the limit is refused rather than cut");
    assert_eq!(error.code(), "MCP-RESULT-TOO-LARGE");

    lab.call(
        "sshell_exec_run",
        json!({ "sessionId": "lab", "command": format!("rm -f {path}") }),
    )
    .await
    .expect("the file is removed");

    lab.close().await;
}

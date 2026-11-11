//! The stdio transport against a real child process.
//!
//! This test target has no libtest harness on purpose: it is one program that runs two ways. With
//! `SSHELL_MCP_FAKE_CHILD` in its environment it *is* the fake server — read a line, answer a line,
//! on stdin and stdout — and without it, it spawns itself, connects the stdio transport to it, and
//! asserts what came back. That is the only way to prove the parts a scripted transport cannot:
//! that a real process is started with the configured environment, that newline-delimited JSON-RPC
//! crosses a pipe in both directions, that its stderr does not block the protocol, and that its exit
//! becomes an error rather than a hang.

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{Value, json};

use sevenshell_mcp_client::{McpClientError, McpServerSecrets, McpSession, connect};
use sevenshell_types::{McpServerConfig, McpServerTransport};

const CHILD_ENV: &str = "SSHELL_MCP_FAKE_CHILD";
const MODE_ENV: &str = "SSHELL_MCP_FAKE_MODE";
const STDERR_MARKER: &str = "fake-server-stderr-marker";

fn main() {
    if std::env::var(CHILD_ENV).is_ok() {
        if let Err(error) = serve() {
            eprintln!("the fake server failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    if let Err(error) = runtime.block_on(run()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

/// The server side: one JSON-RPC message per line, on this process's own stdin and stdout.
fn serve() -> std::io::Result<()> {
    let mode = std::env::var(MODE_ENV).unwrap_or_default();
    if mode == "stderr" {
        eprintln!("{STDERR_MARKER}");
    }

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        let id = message.get("id").cloned().filter(|id| !id.is_null());

        let answer = match method {
            "initialize" => Some(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake-stdio", "version": "0" },
            })),
            // A notification has no answer, which is exactly what makes it a good case here: the
            // client must not wait for one.
            "notifications/initialized" => None,
            "ping" => Some(json!({})),
            "tools/list" => Some(json!({
                "tools": [
                    {
                        "name": "read",
                        "description": "reads something",
                        "inputSchema": { "type": "object", "properties": {} },
                        "annotations": { "readOnlyHint": true },
                    },
                    {
                        "name": "write",
                        "description": "writes something",
                        "inputSchema": { "type": "object", "properties": {} },
                    },
                ],
            })),
            "tools/call" => {
                let name = message["params"]["name"].as_str().unwrap_or_default();
                Some(json!({
                    "content": [{ "type": "text", "text": format!("called {name}") }],
                    "isError": false,
                }))
            }
            // A method this server answers by saying nothing, which is how a caller's timeout is
            // produced without a sleeping child.
            "no_answer" => None,
            other => {
                if let Some(id) = id.clone() {
                    let refusal = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("no method is named `{other}`") },
                    });
                    let _ = writeln!(stdout, "{refusal}");
                    let _ = stdout.flush();
                }
                if mode == "exit_after_initialize" {
                    // The client is about to ask for tools; leaving now is the case this scenario
                    // exists for.
                    std::process::exit(0);
                }
                continue;
            }
        };

        if let (Some(id), Some(result)) = (id, answer) {
            let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }

        if mode == "exit_after_initialize" && method == "initialize" {
            std::process::exit(0);
        }
    }

    Ok(())
}

async fn run() -> Result<(), String> {
    normal_exchange().await?;
    a_child_that_exits_is_an_error().await?;
    stderr_does_not_block_the_protocol().await?;
    a_dropped_request_does_not_wedge_the_transport().await?;
    Ok(())
}

fn config() -> McpServerConfig {
    let executable = std::env::current_exe().expect("the test binary's own path");
    let mut config = McpServerConfig::new("fake", "Fake", McpServerTransport::Stdio);
    config.command = Some(executable.to_string_lossy().into_owned());
    config.enabled = true;
    config
}

/// The environment that turns this binary into the server, in the one place an stdio server's
/// values can travel: the vault entry.
fn secrets(mode: &str) -> McpServerSecrets {
    let mut secrets = McpServerSecrets::default();
    secrets.env.insert(CHILD_ENV.to_owned(), "1".to_owned());
    secrets.env.insert(MODE_ENV.to_owned(), mode.to_owned());
    secrets
}

async fn normal_exchange() -> Result<(), String> {
    let session = tokio::time::timeout(
        Duration::from_secs(20),
        McpSession::connect(config(), Some(secrets("normal"))),
    )
    .await
    .map_err(|_| "the handshake with a real child process did not finish in 20 seconds".to_owned())?
    .map_err(|error| format!("the session could not be opened: {error}"))?;

    let names: Vec<String> = session.tools().into_iter().map(|tool| tool.name).collect();
    assert_eq!(
        names,
        vec!["mcp__fake__read", "mcp__fake__write"],
        "both tools crossed the pipe"
    );

    let call = session
        .call_tool("read", json!({ "path": "x" }), 64 * 1024)
        .await
        .map_err(|error| format!("the call failed: {error}"))?;
    assert_eq!(call.value, json!("called read"), "the child answered the call");

    session
        .close()
        .await
        .map_err(|error| format!("close failed: {error}"))?;
    Ok(())
}

async fn a_child_that_exits_is_an_error() -> Result<(), String> {
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        McpSession::connect(config(), Some(secrets("exit_after_initialize"))),
    )
    .await
    .map_err(|_| "a child that exited left the handshake hanging instead of failing it".to_owned())?;

    let error = match outcome {
        Ok(_) => return Err("a server that exited has no session".to_owned()),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            McpClientError::ConnectFailed { .. } | McpClientError::Failed { .. }
        ),
        "a child going away is a failure with a code, not a panic: {error:?}"
    );
    Ok(())
}

async fn stderr_does_not_block_the_protocol() -> Result<(), String> {
    let transport = tokio::time::timeout(Duration::from_secs(20), connect(&config(), Some(secrets("stderr"))))
        .await
        .map_err(|_| "starting the child did not finish in 20 seconds".to_owned())?
        .map_err(|error| format!("the child could not be started: {error}"))?;

    // The reader task needs a moment to see the child's first line; the assertion is about what it
    // kept, not about when.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let diagnostics = transport.diagnostics().unwrap_or_default();
    assert!(
        diagnostics.contains(STDERR_MARKER),
        "a child's stderr is kept for a diagnostic: {diagnostics:?}"
    );
    assert!(
        diagnostics.len() <= 8 * 1024 + 1,
        "the diagnostic ring is capped: {} bytes",
        diagnostics.len()
    );

    transport
        .close()
        .await
        .map_err(|error| format!("close failed: {error}"))?;
    Ok(())
}

async fn a_dropped_request_does_not_wedge_the_transport() -> Result<(), String> {
    let transport = tokio::time::timeout(Duration::from_secs(20), connect(&config(), Some(secrets("normal"))))
        .await
        .map_err(|_| "starting the child did not finish in 20 seconds".to_owned())?
        .map_err(|error| format!("the child could not be started: {error}"))?;

    let dropped = tokio::time::timeout(Duration::from_millis(150), transport.request("no_answer", None)).await;
    assert!(
        dropped.is_err(),
        "the fake server answers `no_answer` by staying silent"
    );

    let pong = tokio::time::timeout(Duration::from_secs(5), transport.request("ping", None))
        .await
        .map_err(|_| "the transport stopped answering after one request was given up on".to_owned())?
        .map_err(|error| format!("ping failed: {error}"))?;
    assert_eq!(pong, json!({}), "the next request still gets its own answer");

    transport
        .close()
        .await
        .map_err(|error| format!("close failed: {error}"))?;
    Ok(())
}

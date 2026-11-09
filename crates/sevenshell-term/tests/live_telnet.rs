//! A Telnet console against the lab's telnetd.
//!
//! The in-process test proves the negotiation and the byte pump; this one proves the same path
//! against a server that negotiates for real and runs a shell behind it. Busybox `telnetd` is the
//! server in `deploy/lab/openssh/compose.yaml`, exposed on loopback.

use std::time::Duration;

use sevenshell_term::{Size, TelnetSpec, TerminalEvent, open_telnet};
use tokio::time::timeout;

const LAB_PORT: u16 = 23230;

fn lab_host() -> String {
    std::env::var("SEVENSHELL_TEST_TELNET_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned())
}

fn lab_port() -> u16 {
    std::env::var("SEVENSHELL_TEST_TELNET_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(LAB_PORT)
}

async fn read_until(events: &mut tokio::sync::mpsc::Receiver<TerminalEvent>, needle: &str, within: Duration) -> String {
    let deadline = tokio::time::Instant::now() + within;
    let mut seen = String::new();
    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Some(TerminalEvent::Data(bytes))) => {
                seen.push_str(&String::from_utf8_lossy(&bytes));
                if seen.contains(needle) {
                    return seen;
                }
            }
            Ok(Some(TerminalEvent::Closed { reason })) => {
                panic!("the console closed before {needle:?} arrived: {reason:?}; saw {seen:?}");
            }
            Ok(None) => panic!("the event stream ended before {needle:?} arrived; saw {seen:?}"),
            Err(_) => continue,
        }
    }
    panic!("{needle:?} did not arrive within {within:?}; saw {seen:?}");
}

#[tokio::test]
#[ignore = "needs the telnet lab; start it with make lab-up and run with make lab-verify"]
async fn a_real_telnet_server_gets_a_shell_and_runs_a_command() {
    let (session, mut events) = open_telnet(
        TelnetSpec {
            host: lab_host(),
            port: lab_port(),
            enter: sevenshell_types::EnterMode::Lf,
            backspace: sevenshell_types::BackspaceMode::Del,
            timeout: Duration::from_secs(10),
        },
        Size::default(),
    )
    .await
    .expect("the lab's telnetd is reachable");

    // A shell prompt, whichever prompt the image's shell chooses.
    let prompt = read_until(&mut events, "#", Duration::from_secs(15)).await;
    assert!(prompt.contains('#'), "{prompt:?}");

    // The marker is computed by the shell: seeing `...-42` proves the line ran, while the typed
    // text alone would only prove the console echoed it.
    session
        .write(b"echo seven-shell-$((6*7))\n")
        .await
        .expect("the command goes out");
    read_until(&mut events, "seven-shell-42", Duration::from_secs(15)).await;

    session.close().await.expect("the console closes");
}

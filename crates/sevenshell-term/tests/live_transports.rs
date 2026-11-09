//! End-to-end checks for the transports that do not need a lab.
//!
//! The Telnet test runs an in-process server, so it proves the negotiation and the byte pump
//! together without Docker. The local and WSL tests start real processes and are `#[ignore]`d: they
//! belong to the machine they run on, not to every build.

use std::time::Duration;

use sevenshell_term::{
    SerialSpec, Size, TcpSpec, TelnetSpec, TerminalEvent, open_local, open_raw_tcp, open_telnet, open_wsl,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;

const IAC: u8 = 255;
const WILL: u8 = 251;
const DO: u8 = 253;
const SB: u8 = 250;
const SE: u8 = 240;
const OPT_ECHO: u8 = 1;
const OPT_NAWS: u8 = 31;

async fn collect(events: &mut mpsc::Receiver<TerminalEvent>, needle: &str, within: Duration) -> String {
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Some(TerminalEvent::Data(bytes))) => {
                seen.push_str(&String::from_utf8_lossy(&bytes));
                if seen.contains(needle) {
                    return seen;
                }
            }
            Ok(Some(TerminalEvent::Closed { reason })) => {
                panic!("the terminal ended before {needle:?} arrived: {reason:?}; saw {seen:?}");
            }
            Ok(None) => panic!("the event stream ended before {needle:?} arrived; saw {seen:?}"),
            Err(_) => continue,
        }
    }
    panic!("{needle:?} did not arrive within {within:?}; saw {seen:?}");
}

#[tokio::test]
async fn a_telnet_console_negotiates_and_carries_bytes_both_ways() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a loopback listener");
    let address = listener.local_addr().expect("an address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("a client");
        // The client opens with a window report before anything is asked for.
        let mut opening = [0u8; 9];
        socket
            .read_exact(&mut opening)
            .await
            .expect("the opening window report");
        assert_eq!(&opening[..3], &[IAC, SB, OPT_NAWS]);
        assert_eq!(&opening[7..], &[IAC, SE]);

        socket.write_all(&[IAC, WILL, OPT_ECHO]).await.expect("an offer");
        socket.write_all(b"login: ").await.expect("a prompt");

        let mut reply = [0u8; 3];
        socket.read_exact(&mut reply).await.expect("the answer to the offer");
        assert_eq!(reply, [IAC, DO, OPT_ECHO]);

        let mut typed = [0u8; 5];
        socket.read_exact(&mut typed).await.expect("keystrokes");
        assert_eq!(&typed, b"root\r");
        socket.write_all(b"Password: ").await.expect("a second prompt");

        // A resize after the session is up is a fresh window report, not only the opening one.
        let mut resized = [0u8; 9];
        socket
            .read_exact(&mut resized)
            .await
            .expect("the resized window report");
        assert_eq!(&resized, &[IAC, SB, OPT_NAWS, 0, 100, 0, 40, IAC, SE]);
        typed
    });

    let (session, mut events) = open_telnet(
        TelnetSpec {
            host: "127.0.0.1".to_owned(),
            port: address.port(),
            enter: sevenshell_types::EnterMode::Cr,
            backspace: sevenshell_types::BackspaceMode::Del,
            timeout: Duration::from_secs(5),
        },
        Size::default(),
    )
    .await
    .expect("the console opens");

    collect(&mut events, "login: ", Duration::from_secs(5)).await;
    session.write(b"root\r").await.expect("the keystrokes go out");
    collect(&mut events, "Password: ", Duration::from_secs(5)).await;
    session.resize(100, 40).await.expect("the window size goes out");
    assert_eq!(&server.await.expect("the server task"), b"root\r");
    session.close().await.expect("the console closes");
}

#[tokio::test]
async fn a_raw_tcp_console_carries_bytes_verbatim() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a loopback listener");
    let address = listener.local_addr().expect("an address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("a client");
        socket.write_all(b"ready\r\n").await.expect("a greeting");
        let mut typed = [0u8; 4];
        socket.read_exact(&mut typed).await.expect("keystrokes");
        typed
    });

    let (session, mut events) = open_raw_tcp(TcpSpec {
        host: "127.0.0.1".to_owned(),
        port: address.port(),
        timeout: Duration::from_secs(5),
    })
    .await
    .expect("the console opens");

    collect(&mut events, "ready", Duration::from_secs(5)).await;
    // No negotiation, no translation: exactly these four bytes.
    session.write(b"ping").await.expect("the bytes go out");
    assert_eq!(&server.await.expect("the server task"), b"ping");
    session.resize(100, 40).await.expect("resize is a no-op");
    session.close().await.expect("the console closes");
}

#[tokio::test]
async fn a_serial_port_that_does_not_exist_is_named_in_the_error() {
    let error = sevenshell_term::open_serial(SerialSpec {
        port: "definitely-not-a-port".to_owned(),
        baud_rate: 115_200,
        data_bits: 8,
        parity: sevenshell_types::SerialParity::None,
        stop_bits: sevenshell_types::SerialStopBits::One,
        flow_control: sevenshell_types::SerialFlowControl::None,
        backspace: sevenshell_types::BackspaceMode::Del,
    })
    .await
    .err()
    .expect("a port that does not exist cannot open");

    assert_eq!(error.code(), sevenshell_error::ErrorCode::TerminalSerialFailed);
    assert_eq!(
        error.message_params().get("port").map(String::as_str),
        Some("definitely-not-a-port")
    );
}

#[tokio::test]
#[ignore = "starts a shell on this machine; run where that is the point"]
async fn a_local_shell_echoes_what_is_typed() {
    let shell = if cfg!(windows) { "cmd" } else { "" };
    let (session, mut events) = open_local(
        sevenshell_term::LocalSpec {
            shell: shell.to_owned(),
            args: String::new(),
            cwd: None,
        },
        Size::default(),
    )
    .await
    .expect("a local shell starts");

    let marker = "seven-shell-local-marker";
    let line = format!("echo {marker}\r\n");
    session.write(line.as_bytes()).await.expect("keystrokes");
    collect(&mut events, marker, Duration::from_secs(10)).await;
    session.close().await.expect("the shell closes");
}

#[tokio::test]
#[ignore = "needs WSL installed; run where that is the point"]
#[cfg(windows)]
async fn a_wsl_shell_echoes_what_is_typed() {
    let (session, mut events) = open_wsl(
        sevenshell_term::WslSpec {
            distro: String::new(),
            args: String::new(),
        },
        Size::default(),
    )
    .await
    .expect("a WSL shell starts");

    let marker = "seven-shell-wsl-marker";
    let line = format!("echo {marker}\n");
    session.write(line.as_bytes()).await.expect("keystrokes");
    collect(&mut events, marker, Duration::from_secs(20)).await;
    session.close().await.expect("the shell closes");
}

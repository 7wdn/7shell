//! Against a real RDP server: the certificate decision, the first frame, and a wrong password.
//!
//! The default server is the xrdp container in `deploy/lab/rdp`:
//!
//! ```text
//! docker compose -f deploy/lab/rdp/compose.yaml up -d --build
//! SEVENSHELL_TEST_RDP_HOST=127.0.0.1 SEVENSHELL_TEST_RDP_PORT=13389 \
//!   cargo test -p sevenshell-rdp --test live_desktop -- --ignored --test-threads=1
//! ```
//!
//! The same first test is the NLA check against a real Windows host, which is the thing xrdp
//! cannot exercise (it negotiates TLS without NLA). Point the environment at that machine and run
//! only the cases that apply to it:
//!
//! ```text
//! SEVENSHELL_TEST_RDP_HOST=192.168.x.y SEVENSHELL_TEST_RDP_PORT=3389 \
//! SEVENSHELL_TEST_RDP_USER=<user> SEVENSHELL_TEST_RDP_PASSWORD=<password> \
//!   cargo test -p sevenshell-rdp --test live_desktop -- --ignored --test-threads=1 a_real_desktop
//! ```
//!
//! These are `#[ignore]` because they need that server. CI runs them against the xrdp container.

use std::time::Duration;

use sevenshell_error::ErrorCode;
use sevenshell_rdp::{DesktopEvent, DesktopSession, OpenRequest, Target};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

fn server() -> (String, u16, String, String) {
    let host = std::env::var("SEVENSHELL_TEST_RDP_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let port = std::env::var("SEVENSHELL_TEST_RDP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(13389);
    let user = std::env::var("SEVENSHELL_TEST_RDP_USER").unwrap_or_else(|_| "labuser".to_owned());
    let password = std::env::var("SEVENSHELL_TEST_RDP_PASSWORD").unwrap_or_else(|_| "labpassword".to_owned());
    (host, port, user, password)
}

fn target(host: &str, port: u16, user: &str) -> Target {
    Target {
        host: host.to_owned(),
        port,
        user: user.to_owned(),
        domain: String::new(),
    }
}

async fn next_event(receiver: &mut mpsc::UnboundedReceiver<DesktopEvent>, within: Duration) -> DesktopEvent {
    tokio::time::timeout(within, receiver.recv())
        .await
        .expect("the desktop produced no event in time")
        .expect("the desktop event stream closed")
}

#[tokio::test]
#[ignore = "needs an RDP server (the xrdp container, or a Windows host)"]
async fn a_real_desktop_presents_its_certificate() {
    // The negotiated security layer is logged at info level; `RUST_LOG=info` with `--nocapture`
    // is how a manual run tells HYBRID (NLA) from SSL (login inside the session).
    let _ = env_logger::builder().is_test(true).try_init();
    let (host, port, user, _password) = server();
    let target = target(&host, port, &user);

    let certificate = sevenshell_rdp::probe(&target).await.expect("probe");
    assert!(
        certificate.fingerprint.starts_with("SHA256:"),
        "the fingerprint a person compares is spelled like the SSH ones: {}",
        certificate.fingerprint
    );
    // The probe reads the certificate and drops the socket. It is the step that has to work
    // against a Windows host before there is any password to try.
    eprintln!("{host}:{port} presented {}", certificate.fingerprint);
}

#[tokio::test]
#[ignore = "needs the xrdp container in deploy/lab/rdp"]
async fn a_real_desktop_probes_and_draws_its_first_frame() {
    let (host, port, user, password) = server();
    let target = target(&host, port, &user);

    let certificate = sevenshell_rdp::probe(&target).await.expect("probe");
    assert!(
        certificate.fingerprint.starts_with("SHA256:"),
        "the fingerprint a person compares is spelled like the SSH ones: {}",
        certificate.fingerprint
    );

    let (sender, mut receiver) = mpsc::unbounded_channel();
    let session = DesktopSession::open(
        OpenRequest {
            target,
            password: Zeroizing::new(password),
            expected_certificate: certificate.fingerprint.clone(),
            desktop: Default::default(),
        },
        sender,
    )
    .await
    .expect("open");

    let (width, height) = session.size();
    assert!(width > 0 && height > 0, "the negotiated desktop has a size");

    let frame = loop {
        match next_event(&mut receiver, Duration::from_secs(20)).await {
            DesktopEvent::Frames(frames) => break frames.into_iter().next().expect("a batch with a frame"),
            DesktopEvent::Resized { .. } => {}
            DesktopEvent::Closed => panic!("the desktop closed before drawing anything"),
            DesktopEvent::Failed(error) => panic!("the desktop failed before drawing: {error}"),
        }
    };

    assert!(frame.width > 0 && frame.height > 0, "a frame has pixels");
    assert_eq!(
        frame.rgba.len(),
        usize::from(frame.width) * usize::from(frame.height) * 4,
        "a frame is RGBA8, exactly as wide and tall as it says"
    );
    assert!(
        frame.rgba.iter().any(|byte| *byte != 0),
        "the first frame is not a black rectangle"
    );

    session.close().await;
}

#[tokio::test]
#[ignore = "needs the xrdp container in deploy/lab/rdp"]
async fn a_changed_certificate_stops_the_connection_before_authentication() {
    let (host, port, user, password) = server();
    let (sender, _receiver) = mpsc::unbounded_channel();

    let error = match DesktopSession::open(
        OpenRequest {
            target: target(&host, port, &user),
            password: Zeroizing::new(password),
            // A fingerprint that is well formed and is not the server's. Removing the comparison
            // makes this test succeed at opening, which is exactly what must not happen.
            expected_certificate: "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned(),
            desktop: Default::default(),
        },
        sender,
    )
    .await
    {
        Ok(_) => panic!("a certificate that does not match must stop the connection"),
        Err(error) => error,
    };

    assert_eq!(
        error.code(),
        ErrorCode::RdpCertificateChanged,
        "a changed certificate is a stop, not a warning: {error}"
    );
}

#[tokio::test]
#[ignore = "needs the xrdp container in deploy/lab/rdp"]
async fn a_tls_only_server_asks_for_the_password_inside_the_session() {
    // xrdp negotiates TLS but not NLA. With NLA, a wrong password fails the connection (the error
    // this build maps to RDP-AUTH-FAILED); over TLS alone the session is established and the
    // server shows its own login screen. That difference is the server's, not a missing check on
    // this side — and it is why the authentication-failure path needs a real Windows host to be
    // verified end to end.
    let (host, port, user, _password) = server();
    let target = target(&host, port, &user);
    let certificate = sevenshell_rdp::probe(&target).await.expect("probe");

    let (sender, mut receiver) = mpsc::unbounded_channel();
    let session = DesktopSession::open(
        OpenRequest {
            target,
            password: Zeroizing::new("this is not the password".to_owned()),
            expected_certificate: certificate.fingerprint,
            desktop: Default::default(),
        },
        sender,
    )
    .await
    .expect("TLS-only servers authenticate inside the session");

    let frame = loop {
        match next_event(&mut receiver, Duration::from_secs(20)).await {
            DesktopEvent::Frames(frames) => break frames.into_iter().next().expect("a batch with a frame"),
            DesktopEvent::Resized { .. } => {}
            DesktopEvent::Closed => panic!("the desktop closed before drawing anything"),
            DesktopEvent::Failed(error) => panic!("the desktop failed before drawing: {error}"),
        }
    };

    assert!(
        frame.rgba.iter().any(|byte| *byte != 0),
        "the login screen is drawn rather than the connection being refused"
    );

    session.close().await;
}

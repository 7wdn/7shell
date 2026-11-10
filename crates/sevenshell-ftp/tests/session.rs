//! The protocol client against a scripted server on a loopback socket.
//!
//! The server here is deliberately dumb: it speaks the handful of replies this crate consumes, in
//! the order a passive client produces commands, and records every command line it received. That
//! recording is the point — several of the tests below are about what was *not* sent (a password
//! after a refused TLS negotiation, a command after a path was refused).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use sevenshell_error::OperationOutcome;
use sevenshell_ftp::{FtpConnectionOptions, FtpSession};
use sevenshell_vault::ServicePassword;

/// Long enough to be recognizable if it ever appears in an error or a debug line.
const PASSWORD: &str = "s3cret-do-not-log";
const USERNAME: &str = "alice";

type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TlsAnswer {
    /// `AUTH TLS` is not implemented.
    Refuse,
    /// `AUTH TLS` is accepted and then the socket answers with something that is not TLS.
    Garbage,
}

struct Fixture {
    features: Vec<String>,
    mlsd: Vec<String>,
    list: Vec<String>,
    files: Files,
    tls: TlsAnswer,
    reject_mlsd: bool,
    reject_login: bool,
    root: String,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            features: vec!["MLST".to_owned(), "SIZE".to_owned()],
            mlsd: Vec::new(),
            list: Vec::new(),
            files: Arc::new(Mutex::new(HashMap::new())),
            tls: TlsAnswer::Refuse,
            reject_mlsd: false,
            reject_login: false,
            root: "/".to_owned(),
        }
    }
}

impl Fixture {
    fn with_file(self, path: &str, bytes: &[u8]) -> Self {
        self.files
            .lock()
            .expect("files")
            .insert(path.to_owned(), bytes.to_vec());
        self
    }
}

struct FakeServer {
    port: u16,
    commands: Arc<Mutex<Vec<String>>>,
    files: Files,
}

impl FakeServer {
    fn start(fixture: Fixture) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let port = listener.local_addr().expect("fixture address").port();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&commands);
        let files = Arc::clone(&fixture.files);
        thread::spawn(move || {
            if let Ok((control, _)) = listener.accept() {
                serve(control, fixture, log);
            }
        });
        Self { port, commands, files }
    }

    fn options(&self, secure: bool) -> FtpConnectionOptions {
        FtpConnectionOptions::new("127.0.0.1", USERNAME, ServicePassword::new(PASSWORD.to_owned()))
            .with_port(self.port)
            .with_secure(secure)
    }

    fn commands(&self) -> Vec<String> {
        self.commands.lock().expect("command log").clone()
    }

    fn stored(&self, path: &str) -> Option<Vec<u8>> {
        self.files.lock().expect("files").get(path).cloned()
    }
}

fn serve(control: TcpStream, fixture: Fixture, log: Arc<Mutex<Vec<String>>>) {
    let mut writer = control.try_clone().expect("control clone");
    let mut reader = BufReader::new(control);
    reply(&mut writer, "220 fixture ready");

    let mut data: Option<TcpStream> = None;

    loop {
        let mut raw = String::new();
        if reader.read_line(&mut raw).unwrap_or(0) == 0 {
            break;
        }
        let line = raw.trim_end_matches(['\r', '\n']).to_owned();
        log.lock().expect("command log").push(line.clone());
        let (verb, argument) = match line.split_once(' ') {
            Some((verb, argument)) => (verb.to_ascii_uppercase(), argument.to_owned()),
            None => (line.to_ascii_uppercase(), String::new()),
        };

        match verb.as_str() {
            "USER" => reply(&mut writer, "331 password required"),
            "PASS" => {
                if fixture.reject_login {
                    reply(&mut writer, "530 Login incorrect");
                } else {
                    reply(&mut writer, "230 Logged in");
                }
            }
            "TYPE" => reply(&mut writer, "200 Type set to I"),
            "PWD" => reply(&mut writer, &format!("257 \"{}\"", fixture.root)),
            "FEAT" => {
                if fixture.features.is_empty() {
                    reply(&mut writer, "500 FEAT not understood");
                } else {
                    let mut body = String::from("211-Features:\r\n");
                    for feature in &fixture.features {
                        body.push_str(&format!(" {feature}\r\n"));
                    }
                    body.push_str("211 End\r\n");
                    let _ = writer.write_all(body.as_bytes());
                    let _ = writer.flush();
                }
            }
            "PASV" => {
                // The client connects as soon as it has the address, before it sends the data
                // command, so accepting here keeps the loop aligned with the client.
                data.take();
                let listener = TcpListener::bind("127.0.0.1:0").expect("bind data listener");
                let port = listener.local_addr().expect("data address").port();
                reply(
                    &mut writer,
                    &format!("227 Entering Passive Mode (127,0,0,1,{},{}).", port >> 8, port & 0xff),
                );
                data = listener.accept().ok().map(|(socket, _)| socket);
            }
            "MLSD" | "LIST" => {
                if verb == "MLSD" && fixture.reject_mlsd {
                    data = None;
                    reply(&mut writer, "500 MLSD not understood");
                    continue;
                }
                let Some(mut socket) = data.take() else {
                    reply(&mut writer, "425 no data connection");
                    continue;
                };
                reply(&mut writer, "150 Here comes the directory listing");
                let lines = if verb == "MLSD" { &fixture.mlsd } else { &fixture.list };
                let mut body = String::new();
                for line in lines {
                    body.push_str(line);
                    body.push_str("\r\n");
                }
                send_data(&mut socket, body.as_bytes());
                reply(&mut writer, "226 Directory send OK");
            }
            "RETR" => {
                let Some(mut socket) = data.take() else {
                    reply(&mut writer, "425 no data connection");
                    continue;
                };
                reply(&mut writer, "150 Opening data connection");
                let payload = fixture
                    .files
                    .lock()
                    .expect("files")
                    .get(&argument)
                    .cloned()
                    .unwrap_or_default();
                send_data(&mut socket, &payload);
                reply(&mut writer, "226 Transfer complete");
            }
            "STOR" => {
                let Some(mut socket) = data.take() else {
                    reply(&mut writer, "425 no data connection");
                    continue;
                };
                reply(&mut writer, "150 Ok to send data");
                let mut payload = Vec::new();
                let _ = socket.read_to_end(&mut payload);
                fixture.files.lock().expect("files").insert(argument.clone(), payload);
                reply(&mut writer, "226 Transfer complete");
            }
            "MLST" => {
                let machine = fixture
                    .features
                    .iter()
                    .any(|feature| feature.eq_ignore_ascii_case("MLST") || feature.eq_ignore_ascii_case("MLSD"));
                if !machine {
                    reply(&mut writer, "500 MLST not understood");
                    continue;
                }
                let size = fixture.files.lock().expect("files").get(&argument).map(Vec::len);
                match size {
                    Some(size) => {
                        let body = format!(
                            "250-Listing {argument}\r\n type=file;size={size};modify=20240102030405; {argument}\r\n250 End\r\n"
                        );
                        let _ = writer.write_all(body.as_bytes());
                        let _ = writer.flush();
                    }
                    None => reply(&mut writer, "550 No such file"),
                }
            }
            "MKD" => reply(&mut writer, &format!("257 \"{argument}\" created")),
            "RMD" | "DELE" => reply(&mut writer, "250 Removed"),
            "RNFR" => reply(&mut writer, "350 Ready for the target name"),
            "RNTO" => reply(&mut writer, "250 Renamed"),
            "AUTH" => match fixture.tls {
                TlsAnswer::Refuse => reply(&mut writer, "500 AUTH not understood"),
                TlsAnswer::Garbage => {
                    reply(&mut writer, "234 Proceed with negotiation");
                    // Not a TLS record, and then the socket closes: the handshake must fail.
                    let _ = writer.write_all(b"this is not a tls record\r\n");
                    let _ = writer.flush();
                    break;
                }
            },
            "QUIT" => {
                reply(&mut writer, "221 Goodbye");
                break;
            }
            _ => reply(&mut writer, "502 Command not implemented"),
        }
    }
}

fn reply(writer: &mut TcpStream, line: &str) {
    let _ = writer.write_all(format!("{line}\r\n").as_bytes());
    let _ = writer.flush();
}

fn send_data(socket: &mut TcpStream, payload: &[u8]) {
    let _ = socket.write_all(payload);
    let _ = socket.flush();
    let _ = socket.shutdown(Shutdown::Write);
}

fn fixture_with_mlsd() -> Fixture {
    let mut fixture = Fixture::default().with_file("/hello.txt", b"hello");
    fixture.mlsd = vec![
        "type=file;size=5;modify=20240102030405; hello.txt".to_owned(),
        "type=dir;modify=20240102030405; docs".to_owned(),
        "type=cdir;modify=20240102030405; .".to_owned(),
        "type=pdir;modify=20240102030405; ..".to_owned(),
    ];
    fixture
}

async fn connect(server: &FakeServer, secure: bool) -> FtpSession {
    FtpSession::connect(server.options(secure))
        .await
        .expect("the fixture server accepts the session")
}

#[tokio::test]
async fn a_plain_session_lists_moves_files_and_removes_them() {
    let server = FakeServer::start(fixture_with_mlsd());
    let mut session = connect(&server, false).await;
    assert!(!session.is_secure());

    let entries = session.list("/").await.expect("list the root");
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["hello.txt", "docs"]);
    assert_eq!(entries[0].path, "/hello.txt");
    assert_eq!(entries[0].size, 5);
    assert_eq!(entries[0].modified, Some(1_704_164_645));
    assert!(entries[1].is_dir);

    let mut downloaded = Vec::new();
    let received = session.download("/hello.txt", &mut downloaded).await.expect("download");
    assert_eq!(received, 5);
    assert_eq!(downloaded, b"hello");

    let payload = b"uploaded bytes";
    let mut input = payload.as_slice();
    let sent = session.upload("/new.bin", &mut input).await.expect("upload");
    assert_eq!(sent, payload.len() as u64);
    assert_eq!(server.stored("/new.bin").as_deref(), Some(payload.as_slice()));

    session.mkdir("/made").await.expect("mkdir");
    session.rename("/hello.txt", "/renamed.txt").await.expect("rename");
    session.remove_file("/renamed.txt").await.expect("remove file");
    session.remove_dir("/docs").await.expect("remove directory");
    session.quit().await.expect("quit");

    let commands = server.commands();
    for expected in [
        "USER alice",
        "PASS s3cret-do-not-log",
        "TYPE I",
        "MLSD /",
        "PASV",
        "RETR /hello.txt",
        "STOR /new.bin",
        "MKD /made",
        "RNFR /hello.txt",
        "RNTO /renamed.txt",
        "DELE /renamed.txt",
        "RMD /docs",
        "QUIT",
    ] {
        assert!(
            commands.iter().any(|command| command == expected),
            "the session never sent {expected:?}; it sent {commands:?}"
        );
    }
}

#[tokio::test]
async fn a_server_without_mlsd_is_listed_with_the_unix_format() {
    let fixture = Fixture {
        features: vec!["SIZE".to_owned()],
        list: vec![
            "total 8".to_owned(),
            "-rw-r--r-- 1 alice alice 12 Nov  5 13:46 hello.txt".to_owned(),
            "drwxr-xr-x 2 alice alice 4096 Nov  5 13:46 docs".to_owned(),
        ],
        ..Fixture::default()
    };
    let server = FakeServer::start(fixture);
    let mut session = connect(&server, false).await;

    let entries = session.list("/").await.expect("list the root");
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["hello.txt", "docs"]);
    assert!(!entries[0].is_dir);
    assert_eq!(entries[0].size, 12);
    assert!(entries[1].is_dir);
    assert_eq!(entries[1].path, "/docs");

    let commands = server.commands();
    assert!(
        !commands.iter().any(|command| command.starts_with("MLSD")),
        "MLSD was attempted although FEAT did not advertise MLST: {commands:?}"
    );
    assert!(commands.iter().any(|command| command == "LIST /"));
}

#[tokio::test]
async fn a_windows_list_format_is_understood() {
    let fixture = Fixture {
        features: Vec::new(),
        reject_mlsd: true,
        list: vec![
            "04-20-21  03:40PM       <DIR>          docs".to_owned(),
            "04-20-21  03:40PM                 1234 hello.txt".to_owned(),
        ],
        ..Fixture::default()
    };
    let server = FakeServer::start(fixture);
    let mut session = connect(&server, false).await;

    let entries = session.list("/").await.expect("list the root");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "docs");
    assert!(entries[0].is_dir);
    assert_eq!(entries[1].name, "hello.txt");
    assert_eq!(entries[1].size, 1234);
    assert!(entries[1].modified.is_some());
}

#[tokio::test]
async fn a_refused_mlsd_falls_back_to_list_for_this_and_later_listings() {
    let fixture = Fixture {
        reject_mlsd: true,
        list: vec!["-rw-r--r-- 1 alice alice 3 Nov  5 13:46 one.txt".to_owned()],
        ..Fixture::default()
    };
    let server = FakeServer::start(fixture);
    let mut session = connect(&server, false).await;

    let first = session.list("/").await.expect("first listing");
    assert_eq!(first[0].name, "one.txt");
    let _ = session.list("/").await.expect("second listing");

    let commands = server.commands();
    let mlsd = commands.iter().filter(|command| command.starts_with("MLSD")).count();
    assert_eq!(mlsd, 1, "the refused verb was retried: {commands:?}");
    assert_eq!(commands.iter().filter(|command| *command == "LIST /").count(), 2);
}

#[tokio::test]
async fn a_refused_auth_tls_ends_the_attempt_instead_of_logging_in_plainly() {
    let fixture = Fixture {
        tls: TlsAnswer::Refuse,
        ..Fixture::default()
    };
    let server = FakeServer::start(fixture);

    let error = match FtpSession::connect(server.options(true)).await {
        Ok(_) => panic!("a server without AUTH TLS must not yield a session"),
        Err(error) => error,
    };

    let detail = error.detail().unwrap_or_default();
    assert!(!detail.contains(PASSWORD), "the password reached the error: {detail}");
    assert!(
        !detail.contains(&server.port.to_string()),
        "the error named the full address: {detail}"
    );

    let commands = server.commands();
    assert!(commands.iter().any(|command| command == "AUTH TLS"));
    assert!(
        !commands
            .iter()
            .any(|command| command.starts_with("USER") || command.starts_with("PASS")),
        "credentials were sent after TLS was refused: {commands:?}"
    );
}

#[tokio::test]
async fn a_failed_tls_handshake_does_not_fall_back_to_a_plain_login() {
    let fixture = Fixture {
        tls: TlsAnswer::Garbage,
        ..Fixture::default()
    };
    let server = FakeServer::start(fixture);

    let error = match FtpSession::connect(server.options(true)).await {
        Ok(_) => panic!("a server that cannot complete TLS must not yield a session"),
        Err(error) => error,
    };
    assert!(error.detail().is_some(), "the failure carried no detail");

    let commands = server.commands();
    assert!(commands.iter().any(|command| command == "AUTH TLS"));
    assert!(
        !commands
            .iter()
            .any(|command| command.starts_with("USER") || command.starts_with("PASS")),
        "credentials were sent after the handshake failed: {commands:?}"
    );
}

#[tokio::test]
async fn a_rejected_login_reports_the_status_without_the_password_or_the_address() {
    let fixture = Fixture {
        reject_login: true,
        ..Fixture::default()
    };
    let server = FakeServer::start(fixture);

    let error = match FtpSession::connect(server.options(false)).await {
        Ok(_) => panic!("a rejected login must not yield a session"),
        Err(error) => error,
    };
    let detail = error.detail().unwrap_or_default();
    assert!(detail.contains("530"), "the status was lost: {detail}");
    assert!(!detail.contains(PASSWORD), "the password reached the error: {detail}");
    assert!(
        !detail.contains(&server.port.to_string()),
        "the error named the full address: {detail}"
    );
    assert_eq!(error.outcome(), OperationOutcome::Sent);
}

#[tokio::test]
async fn a_path_that_would_leave_the_session_root_is_refused_before_any_command() {
    let server = FakeServer::start(fixture_with_mlsd());
    let mut session = connect(&server, false).await;
    let before = server.commands().len();

    let upward = session.list("../etc").await.expect_err("a parent path must be refused");
    assert!(
        upward.detail().unwrap_or_default().contains("session root"),
        "the refusal did not name the rule: {upward:?}"
    );

    let mut sink = Vec::new();
    session
        .download("/docs/../../etc/passwd", &mut sink)
        .await
        .expect_err("a download outside the root must be refused");
    session
        .remove_file("/ok\r\nDELE /other")
        .await
        .expect_err("a line break in a path must be refused");

    let after = server.commands();
    assert_eq!(
        after.len(),
        before,
        "a refused path still produced commands: {:?}",
        &after[before..]
    );
}

#[tokio::test]
async fn the_login_directory_is_the_session_root() {
    let mut fixture = fixture_with_mlsd();
    fixture.root = "/srv/site".to_owned();
    fixture
        .files
        .lock()
        .expect("files")
        .insert("/srv/site/hello.txt".to_owned(), b"hello".to_vec());
    let server = FakeServer::start(fixture);
    let mut session = connect(&server, false).await;

    let entries = session.list("/").await.expect("list the root");
    assert_eq!(entries[0].path, "/hello.txt", "entry paths are session paths");

    let mut downloaded = Vec::new();
    session.download("/hello.txt", &mut downloaded).await.expect("download");
    assert_eq!(downloaded, b"hello");

    let commands = server.commands();
    assert!(commands.iter().any(|command| command == "MLSD /srv/site"));
    assert!(commands.iter().any(|command| command == "RETR /srv/site/hello.txt"));
}

#[tokio::test]
async fn stat_uses_mlst_and_reports_a_missing_entry_as_none() {
    let server = FakeServer::start(fixture_with_mlsd());
    let mut session = connect(&server, false).await;

    let entry = session
        .stat("/hello.txt")
        .await
        .expect("stat")
        .expect("the file exists");
    assert_eq!(entry.name, "hello.txt");
    assert_eq!(entry.path, "/hello.txt");
    assert_eq!(entry.size, 5);
    assert_eq!(entry.modified, Some(1_704_164_645));

    assert!(session.stat("/missing.txt").await.expect("stat").is_none());
    assert!(session.exists("/hello.txt").await.expect("exists"));
    assert!(!session.exists("/missing.txt").await.expect("exists"));
}

#[tokio::test]
async fn stat_falls_back_to_the_parent_listing_when_mlst_is_not_implemented() {
    let fixture = Fixture {
        features: vec!["SIZE".to_owned()],
        reject_mlsd: true,
        list: vec!["-rw-r--r-- 1 alice alice 5 Nov  5 13:46 hello.txt".to_owned()],
        ..Fixture::default()
    };
    let server = FakeServer::start(fixture);
    let mut session = connect(&server, false).await;

    let entry = session
        .stat("/hello.txt")
        .await
        .expect("stat")
        .expect("the file exists");
    assert_eq!(entry.name, "hello.txt");
    assert_eq!(entry.size, 5);

    let commands = server.commands();
    assert!(commands.iter().any(|command| command == "MLST /hello.txt"));
    assert!(commands.iter().any(|command| command == "LIST /"));
}

#[tokio::test]
async fn credentials_with_a_line_break_are_refused_before_a_socket_is_opened() {
    let options = FtpConnectionOptions::new(
        "127.0.0.1",
        "alice\r\nDELE x",
        ServicePassword::new(PASSWORD.to_owned()),
    )
    .with_port(1);
    let error = match FtpSession::connect(options).await {
        Ok(_) => panic!("a username with a line break must not connect"),
        Err(error) => error,
    };
    assert!(
        error.detail().unwrap_or_default().contains("line break"),
        "the refusal did not say why: {error:?}"
    );

    let options =
        FtpConnectionOptions::new("127.0.0.1", USERNAME, ServicePassword::new("pw\nDELE x".to_owned())).with_port(1);
    let error = match FtpSession::connect(options).await {
        Ok(_) => panic!("a password with a line break must not connect"),
        Err(error) => error,
    };
    let detail = error.detail().unwrap_or_default();
    assert!(
        detail.contains("password"),
        "the refusal did not name the field: {detail}"
    );
    assert!(!detail.contains("pw\nDELE"), "the password was echoed back: {detail}");
}

#[test]
fn a_session_can_cross_an_await_point_in_the_orchestration_layer() {
    fn assert_send<T: Send>() {}
    assert_send::<FtpSession>();
}

#[test]
fn debug_output_names_the_host_but_not_the_password_or_the_port() {
    let options = FtpConnectionOptions::new("files.example.org", USERNAME, ServicePassword::new(PASSWORD.to_owned()))
        .with_port(2121);
    let rendered = format!("{options:?}");
    assert!(rendered.contains("files.example.org"));
    assert!(!rendered.contains(PASSWORD), "the password reached Debug: {rendered}");
    assert!(!rendered.contains("2121"), "the full address reached Debug: {rendered}");
}

//! An FTP session through the core: open, list, and move bytes both ways.
//!
//! The protocol client has its own tests; what this proves is the wiring — that an `ftp` session
//! reaches the same file browser and transfer registry as SFTP and WebDAV, and that the
//! progress-aware reader/writer around an FTP transfer still produces the right bytes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, EntryKind, FtpSettings, Protocol, Session, SessionAuth, SftpOutcome, TerminalSettings, TransferState,
    WebDavSettings, WslSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

type StoredFiles = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

#[derive(Clone, Default)]
struct Recorded {
    commands: Arc<Mutex<Vec<String>>>,
    stored: StoredFiles,
    renamed: Arc<Mutex<Vec<(String, String)>>>,
}

struct FakeFtp {
    port: u16,
    recorded: Recorded,
}

impl FakeFtp {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a control port");
        let port = listener.local_addr().expect("an address").port();
        let recorded = Recorded::default();
        let for_task = recorded.clone();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let recorded = for_task.clone();
                tokio::spawn(async move {
                    let _ = serve_control(stream, recorded).await;
                });
            }
        });

        Self { port, recorded }
    }
}

async fn serve_control(stream: TcpStream, recorded: Recorded) -> std::io::Result<()> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut passive: Option<TcpListener> = None;
    let mut rename_from: Option<String> = None;

    writer.write_all(b"220 fake ftp ready\r\n").await?;

    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let command = line.trim_end_matches(['\r', '\n']).to_owned();
        recorded.commands.lock().expect("commands").push(command.clone());
        let (verb, argument) = command.split_once(' ').unwrap_or((command.as_str(), ""));
        let verb = verb.to_ascii_uppercase();

        match verb.as_str() {
            "USER" => writer.write_all(b"331 password please\r\n").await?,
            "PASS" => writer.write_all(b"230 logged in\r\n").await?,
            "TYPE" | "OPTS" | "MODE" => writer.write_all(b"200 ok\r\n").await?,
            "PWD" => writer.write_all(b"257 \"/\"\r\n").await?,
            "FEAT" => writer.write_all(b"500 FEAT not understood\r\n").await?,
            "MLST" => writer.write_all(b"502 not implemented\r\n").await?,
            "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let port = listener.local_addr()?.port();
                passive = Some(listener);
                let high = port / 256;
                let low = port % 256;
                writer
                    .write_all(format!("227 Entering Passive Mode (127,0,0,1,{high},{low}).\r\n").as_bytes())
                    .await?;
            }
            "EPSV" => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let port = listener.local_addr()?.port();
                passive = Some(listener);
                writer
                    .write_all(format!("229 Entering Extended Passive Mode (|||{port}|)\r\n").as_bytes())
                    .await?;
            }
            "LIST" => {
                let Some(listener) = passive.take() else {
                    writer.write_all(b"425 no data connection\r\n").await?;
                    continue;
                };
                writer.write_all(b"150 opening data\r\n").await?;
                let (mut data, _) = listener.accept().await?;
                data.write_all(
                    b"-rw-r--r--    1 alice    users           5 Jan 01 00:00 notes.txt\r\n\
                      drwxr-xr-x    2 alice    users        4096 Jan 01 00:00 docs\r\n",
                )
                .await?;
                data.shutdown().await?;
                writer.write_all(b"226 listing sent\r\n").await?;
            }
            "RETR" => {
                let Some(listener) = passive.take() else {
                    writer.write_all(b"425 no data connection\r\n").await?;
                    continue;
                };
                writer.write_all(b"150 opening data\r\n").await?;
                let (mut data, _) = listener.accept().await?;
                data.write_all(b"hello").await?;
                data.shutdown().await?;
                writer.write_all(b"226 transfer done\r\n").await?;
            }
            "STOR" => {
                let Some(listener) = passive.take() else {
                    writer.write_all(b"425 no data connection\r\n").await?;
                    continue;
                };
                writer.write_all(b"150 opening data\r\n").await?;
                let (mut data, _) = listener.accept().await?;
                let mut bytes = Vec::new();
                data.read_to_end(&mut bytes).await?;
                recorded
                    .stored
                    .lock()
                    .expect("stored")
                    .push((argument.to_owned(), bytes));
                writer.write_all(b"226 transfer done\r\n").await?;
            }
            "MKD" => writer.write_all(b"257 created\r\n").await?,
            "DELE" | "RMD" => writer.write_all(b"250 removed\r\n").await?,
            "RNFR" => {
                rename_from = Some(argument.to_owned());
                writer.write_all(b"350 ready for target\r\n").await?;
            }
            "RNTO" => {
                if let Some(from) = rename_from.take() {
                    recorded
                        .renamed
                        .lock()
                        .expect("renamed")
                        .push((from, argument.to_owned()));
                }
                writer.write_all(b"250 renamed\r\n").await?;
            }
            "QUIT" => {
                writer.write_all(b"221 goodbye\r\n").await?;
                return Ok(());
            }
            _ => writer.write_all(b"502 not implemented\r\n").await?,
        }
    }
}

fn ftp_session(port: u16) -> Session {
    Session {
        id: "ftp-1".to_owned(),
        name: "ftp".to_owned(),
        group_id: None,
        protocol: Protocol::Ftp,
        host: "127.0.0.1".to_owned(),
        port,
        user: "alice".to_owned(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        desktop: sevenshell_types::DesktopSettings::default(),
        telnet: sevenshell_types::TelnetSettings::default(),
        serial: sevenshell_types::SerialSettings::default(),
        local: sevenshell_types::LocalSettings::default(),
        wsl: WslSettings::default(),
        webdav: WebDavSettings::default(),
        ftp: FtpSettings { secure: false },
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 1_700_000_000,
    }
}

async fn settle(core: &Core, id: &str) -> TransferState {
    for _ in 0..200 {
        if let Some(info) = core.transfers().await.into_iter().find(|info| info.id == id)
            && !matches!(info.state, TransferState::Running | TransferState::Paused)
        {
            return info.state;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the transfer did not settle");
}

#[tokio::test]
async fn an_ftp_session_browses_and_moves_bytes_both_ways() {
    let server = FakeFtp::start().await;
    let directory = tempfile::tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("master password").expect("a vault");
    vault
        .save_session(
            ftp_session(server.port),
            Some(SessionSecret::Password("hunter2".to_owned())),
        )
        .expect("a session");
    let (core, _events) = Core::new(vault);

    let outcome = core.open_files("ftp-1").await.expect("opened");
    let SftpOutcome::Opened { handle, listing, .. } = outcome else {
        panic!("an FTP session should open a browser");
    };
    assert_eq!(listing.path, "/");
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(listing.entries[0].name, "docs", "folders come first");
    assert_eq!(listing.entries[0].kind, EntryKind::Directory);
    assert_eq!(listing.entries[1].name, "notes.txt");
    assert_eq!(listing.entries[1].size, 5);

    let target = directory.path().join("notes.txt");
    let download = core
        .start_download(&handle, "/notes.txt", target.clone())
        .await
        .expect("a download");
    assert_eq!(settle(&core, &download).await, TransferState::Done);
    assert_eq!(std::fs::read_to_string(&target).expect("read"), "hello");

    let source = directory.path().join("up.txt");
    std::fs::write(&source, b"world").expect("write");
    let upload = core.start_upload(&handle, source, "/").await.expect("an upload");
    assert_eq!(settle(&core, &upload).await, TransferState::Done);

    let stored = server.recorded.stored.lock().expect("stored").clone();
    assert_eq!(stored.len(), 1, "one STOR, through the part name");
    assert_eq!(stored[0].0, "/.up.txt.7shell-part");
    assert_eq!(stored[0].1, b"world");

    let renamed = server.recorded.renamed.lock().expect("renamed").clone();
    assert_eq!(renamed, vec![("/.up.txt.7shell-part".to_owned(), "/up.txt".to_owned())]);

    let commands = server.recorded.commands.lock().expect("commands").clone();
    assert!(
        commands.iter().any(|command| command == "USER alice"),
        "the control connection logged in: {commands:?}"
    );
}

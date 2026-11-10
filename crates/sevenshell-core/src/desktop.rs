//! Remote desktops, handed to the Windows client that does them best.
//!
//! The protocol does not live in this process. On Windows the system client (`mstsc.exe`) is given
//! a temporary `.rdp` file: it brings GPU rendering, clipboard, audio and local-drive redirection
//! that a second implementation would spend a long time approaching.
//!
//! The file is written here and never named to the interface. A stored password is included as a
//! DPAPI blob scoped to the current Windows user, so it never appears on a command line or in the
//! file in readable form; an account without a stored password is left to the client's own
//! credential prompt. The file is removed a minute after the client starts.
//!
//! Every other platform refuses: the product offers remote desktops on Windows only, rather than
//! pretending a different client would behave the same.

use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::Protocol;

use crate::sessions::Core;

/// How long the temporary `.rdp` stays on disk after the client is started.
#[cfg(windows)]
const RDP_FILE_LIFETIME: Duration = Duration::from_secs(60);

impl Core {
    /// Opens a remote desktop in the system client.
    #[cfg(windows)]
    pub async fn open_desktop(&self, session_id: &str) -> Result<(), ShellError> {
        let session = self.session(session_id)?;
        if session.protocol != Protocol::Rdp {
            return Err(ShellError::app_unsupported("this session is not a remote desktop"));
        }

        let password = self
            .vault
            .secret_for_session(&session.id)?
            .map(|secret| secret.into_value());
        let document = rdp_document(&session, password.as_deref())?;
        launch(&session, document).await
    }

    /// Remote desktops are a Windows feature of this build.
    #[cfg(not(windows))]
    pub async fn open_desktop(&self, _session_id: &str) -> Result<(), ShellError> {
        Err(ShellError::app_unsupported(
            "the Windows Remote Desktop client is only available on Windows",
        ))
    }
}

/// Builds the `.rdp` document handed to the system client.
///
/// The defaults come from the session's [`DesktopSettings`](sevenshell_types::DesktopSettings):
/// a 1280×1024 window, the local drives mapped and the clipboard shared. The server's certificate
/// warning is left alone — an RDP certificate is usually self-signed, and suppressing the warning
/// would hide a real change too.
#[cfg(windows)]
fn rdp_document(session: &sevenshell_types::Session, password: Option<&str>) -> Result<String, ShellError> {
    let (domain, user) = match session.user.split_once('\\') {
        Some((domain, user)) => (domain, user),
        None => ("", session.user.as_str()),
    };
    let width = session.desktop.width.max(640);
    let height = session.desktop.height.max(480);
    let (window_x, window_y) = window_position(width, height);

    let mut lines = vec![
        format!("full address:s:{}:{}", session.host, session.port),
        format!("username:s:{user}"),
        format!("screen mode id:i:{}", if session.desktop.fullscreen { 2 } else { 1 }),
        format!("desktopwidth:i:{width}"),
        format!("desktopheight:i:{height}"),
        // A normal window at the configured size, not a maximized one: show state 1 is
        // SW_SHOWNORMAL, and the rectangle is where the window opens on this screen.
        format!(
            "winposstr:s:0,1,{window_x},{window_y},{},{}",
            window_x + i32::from(width),
            window_y + i32::from(height)
        ),
        "session bpp:i:32".to_owned(),
        // The window is resizable and the remote picture scales with it. Dynamic resolution is
        // deliberately left off: the resolution a person chose is the resolution they get.
        "smart sizing:i:1".to_owned(),
        "displayconnectionbar:i:1".to_owned(),
        "autoreconnection enabled:i:1".to_owned(),
        "redirectclipboard:i:1".to_owned(),
        "audiomode:i:0".to_owned(),
        "authentication level:i:2".to_owned(),
        "promptcredentialonce:i:0".to_owned(),
        // The local-network experience: wallpaper, themes and composition stay on. The keys are
        // written explicitly rather than left to the client's connection-quality guess.
        "connection type:i:7".to_owned(),
        "networkautodetect:i:1".to_owned(),
        "bandwidthautodetect:i:1".to_owned(),
        "disable wallpaper:i:0".to_owned(),
        "allow font smoothing:i:1".to_owned(),
        "allow desktop composition:i:1".to_owned(),
        "disable full window drag:i:0".to_owned(),
        "disable menu anims:i:0".to_owned(),
        "disable themes:i:0".to_owned(),
        "bitmapcachepersistenable:i:1".to_owned(),
        "use multimon:i:0".to_owned(),
    ];

    if session.desktop.redirect_drives {
        // The reference maps every local drive by default; the remote can read and write them, so
        // this is the setting a person is told about rather than a quiet default.
        lines.push("drivestoredirect:s:*".to_owned());
    }

    if !domain.is_empty() {
        lines.push(format!("domain:s:{domain}"));
    }

    match password {
        Some(password) => {
            lines.push(format!("password 51:b:{}", protect_password(password)?));
            lines.push("prompt for credentials:i:0".to_owned());
        }
        None => {
            // A blank password is a legitimate account state, and the server decides whether to
            // accept it. Let the client ask rather than inventing one here.
            lines.push("prompt for credentials:i:1".to_owned());
        }
    }

    Ok(format!("{}\r\n", lines.join("\r\n")))
}

#[cfg(windows)]
async fn launch(session: &sevenshell_types::Session, document: String) -> Result<(), ShellError> {
    let path = std::env::temp_dir().join(format!(
        "7Shell - {} - {}.rdp",
        file_stem(&session.name),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    std::fs::write(&path, document)
        .map_err(|error| ShellError::rdp_launch_failed(format!("the desktop file could not be written: {error}")))?;

    let started = std::process::Command::new("mstsc.exe").arg(&path).spawn();
    if let Err(error) = started {
        let _ = std::fs::remove_file(&path);
        return Err(ShellError::rdp_launch_failed(format!(
            "the Windows Remote Desktop client could not be started: {error}"
        )));
    }

    // The client has read the file by the time its process exists; keeping it around only widens
    // the window in which a DPAPI blob sits on disk.
    tokio::spawn(async move {
        tokio::time::sleep(RDP_FILE_LIFETIME).await;
        let _ = std::fs::remove_file(path);
    });

    Ok(())
}

/// Encrypts the password for the `.rdp` file, for the current Windows user only.
///
/// `mstsc` stores this value as the UTF-16LE password protected with `CryptProtectData` and no
/// additional entropy. The blob is useless to another account on the same machine and to anyone
/// reading the temporary file.
#[cfg(windows)]
fn protect_password(password: &str) -> Result<String, ShellError> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
    use windows::core::PCWSTR;

    let mut data: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).expect("a password fits in u32"),
        pbData: data.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            0x1, // CRYPTPROTECT_UI_FORBIDDEN
            &mut output,
        )
        .map_err(|error| {
            ShellError::rdp_launch_failed(format!("the password could not be protected for the client: {error}"))
        })?;

        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(to_hex(&bytes))
    }
}

#[cfg(windows)]
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// A file name that reads like the session in the client's title bar, and is legal on Windows.
#[cfg(windows)]
fn file_stem(name: &str) -> String {
    let safe: String = name
        .chars()
        .filter(|character| character.is_alphanumeric() || matches!(character, ' ' | '-' | '_' | '.'))
        .take(40)
        .collect();
    let safe = safe.trim().trim_end_matches('.').trim();
    if safe.is_empty() {
        "desktop".to_owned()
    } else {
        safe.to_owned()
    }
}

/// Where a normal window of this size sits on the primary screen: centred, above the taskbar.
#[cfg(windows)]
fn window_position(width: u16, height: u16) -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = ((screen_width - i32::from(width)) / 2).max(0);
    // 48 pixels is roughly the taskbar; a window that overlaps it would open partly off-screen on
    // a 1080p display, which is the common case.
    let y = ((screen_height - i32::from(height) - 48) / 2).max(0);
    (x, y)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use sevenshell_types::{AuthMethod, Session, SessionAuth, TerminalSettings};

    fn session() -> Session {
        Session {
            id: "desktop-1".to_owned(),
            name: "desktop".to_owned(),
            group_id: None,
            protocol: Protocol::Rdp,
            host: "desktop.example".to_owned(),
            port: 3389,
            user: "DOMAIN\\alice".to_owned(),
            auth: SessionAuth::with_password(AuthMethod::Password),
            terminal: TerminalSettings::default(),
            desktop: sevenshell_types::DesktopSettings::default(),
            telnet: sevenshell_types::TelnetSettings::default(),
            serial: sevenshell_types::SerialSettings::default(),
            local: sevenshell_types::LocalSettings::default(),
            wsl: sevenshell_types::WslSettings::default(),
            webdav: sevenshell_types::WebDavSettings::default(),
            ftp: sevenshell_types::FtpSettings::default(),
            jump: Vec::new(),
            forwards: Vec::new(),
            updated_at: 0,
        }
    }

    #[test]
    fn the_document_names_the_host_and_maps_the_local_drives() {
        let document = rdp_document(&session(), None).expect("a document");

        assert!(document.contains("full address:s:desktop.example:3389"));
        assert!(document.contains("username:s:alice"));
        assert!(document.contains("domain:s:DOMAIN"));
        assert!(
            document.contains("drivestoredirect:s:*"),
            "local drives are on by default"
        );
        assert!(document.contains("redirectclipboard:i:1"));
        assert!(document.contains("smart sizing:i:1"), "the window must be resizable");
        assert!(
            document.contains("winposstr:s:0,1,"),
            "the window opens normal, not maximized"
        );
        assert!(
            !document.contains("dynamic resolution:i:1"),
            "the chosen resolution stays the remote resolution"
        );
        assert!(document.contains("disable wallpaper:i:0"), "the wallpaper stays on");
        // Nothing was stored, so the client asks — a blank password is the account's business.
        assert!(document.contains("prompt for credentials:i:1"));
        assert!(!document.contains("password 51:b:"));
        assert!(document.ends_with("\r\n"));
    }

    #[test]
    fn the_document_uses_the_sessions_screen_settings() {
        let mut session = session();
        session.desktop.width = 1920;
        session.desktop.height = 1080;
        session.desktop.fullscreen = true;
        session.desktop.redirect_drives = false;

        let document = rdp_document(&session, None).expect("a document");

        assert!(document.contains("screen mode id:i:2"));
        assert!(document.contains("desktopwidth:i:1920"));
        assert!(document.contains("desktopheight:i:1080"));
        assert!(
            !document.contains("drivestoredirect:"),
            "mapping is off for this session"
        );
    }

    #[test]
    fn a_stored_password_is_sealed_and_never_written_in_plain_text() {
        let document = rdp_document(&session(), Some("hunter2")).expect("a document");

        assert!(document.contains("password 51:b:"));
        assert!(
            !document.contains("hunter2"),
            "the password must not be readable in the file"
        );
        assert!(document.contains("prompt for credentials:i:0"));
    }
}

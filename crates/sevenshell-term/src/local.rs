//! A shell on this machine, carried over a pseudo terminal.
//!
//! This is the one transport with no device or server on the other side: the program is a process
//! started here. ConPTY on Windows and a Unix pseudo terminal are what `portable-pty` gives us, and
//! with them the shell sees a real terminal and can do line editing, colours and window-size
//! reporting exactly as it would in a native window.
//!
//! Shells are named to the interface by identifier, never by path: the picker shows `PowerShell`
//! and the session stores `pwsh`, while the path is resolved in this process. A WSL session is the
//! same machinery with `wsl.exe` as the program.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use sevenshell_error::ShellError;
use sevenshell_types::LocalShell;
use tokio::sync::mpsc;

use crate::{Size, TerminalEvent};

/// How long a closed shell has to exit on its own before it is killed.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// How long the output that was already produced gets to arrive after the shell exits.
const EXIT_DRAIN: Duration = Duration::from_millis(250);

/// Which shell to start, and with what extra arguments.
#[derive(Debug, Clone)]
pub struct LocalSpec {
    /// The identifier from a saved session, or empty for the platform default.
    pub shell: String,
    /// Extra arguments as one line, split on whitespace.
    pub args: String,
    /// Where to start, when something other than the home directory is wanted.
    pub cwd: Option<PathBuf>,
}

/// Which WSL distribution to start.
#[derive(Debug, Clone)]
pub struct WslSpec {
    /// A distribution name, or empty for the default one.
    pub distro: String,
    pub args: String,
}

/// A shell that is open.
pub struct LocalSession {
    commands: mpsc::Sender<Command>,
}

enum Command {
    Input(Vec<u8>),
    Resize { columns: u16, rows: u16 },
    Close,
}

impl LocalSession {
    pub async fn write(&self, data: &[u8]) -> Result<(), ShellError> {
        self.commands
            .send(Command::Input(data.to_vec()))
            .await
            .map_err(|_| ShellError::session_not_found("the shell is no longer running"))
    }

    pub async fn resize(&self, columns: u32, rows: u32) -> Result<(), ShellError> {
        self.commands
            .send(Command::Resize {
                columns: columns.clamp(1, u16::MAX as u32) as u16,
                rows: rows.clamp(1, u16::MAX as u32) as u16,
            })
            .await
            .map_err(|_| ShellError::session_not_found("the shell is no longer running"))
    }

    pub async fn close(&self) -> Result<(), ShellError> {
        let _ = self.commands.send(Command::Close).await;
        Ok(())
    }
}

/// The shells this machine can start.
pub fn local_shells() -> Vec<LocalShell> {
    #[cfg(windows)]
    {
        windows_shells()
    }
    #[cfg(not(windows))]
    {
        unix_shells()
    }
}

/// The WSL distributions this machine has.
///
/// The list comes from `wsl.exe -l -q`, whose output is UTF-16LE. A machine without WSL answers
/// with nothing rather than an error: the picker then offers the default entry, and starting it
/// says what is missing in the session's own error.
pub fn wsl_distros() -> Vec<String> {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("wsl.exe")
            .args(["-l", "-q"])
            .creation_flags_no_window()
            .output();
        let Ok(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }
        decode_wsl_list(&output.stdout)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

/// Decodes `wsl.exe -l -q` output. The command writes UTF-16LE with a byte-order mark; a build
/// that prints ASCII still decodes, because every other byte is zero.
#[cfg(any(windows, test))]
fn decode_wsl_list(bytes: &[u8]) -> Vec<String> {
    let body = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
    // A truncated tail is half a code unit, not a distribution name, so the odd byte is dropped.
    let mut units = Vec::with_capacity(body.len() / 2);
    let mut index = 0;
    while index + 1 < body.len() {
        units.push(u16::from_le_bytes([body[index], body[index + 1]]));
        index += 2;
    }
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim_matches(['\0', '\r', ' ', '\t']))
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

pub async fn open(spec: LocalSpec, size: Size) -> Result<(LocalSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let resolved = resolve_shell(&spec.shell)?;
    let mut args = resolved.args(platform(), &spec.args);
    args = windows_utf8_console(platform(), resolved.kind, args);
    spawn(resolved.program, args, spec.cwd, size)
}

pub async fn open_wsl(spec: WslSpec, size: Size) -> Result<(LocalSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    #[cfg(not(windows))]
    {
        let _ = (spec, size);
        Err(ShellError::app_unsupported("WSL is only available on Windows"))
    }
    #[cfg(windows)]
    {
        if std::process::Command::new("wsl.exe")
            .arg("--status")
            .creation_flags_no_window()
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
        {
            return Err(ShellError::app_unsupported("WSL is not installed on this machine"));
        }
        let mut args = wsl_arguments(&spec.distro);
        args.extend(split_args(&spec.args));
        spawn("wsl.exe".to_owned(), args, None, size)
    }
}

/// The argument list for a WSL start. The distribution name is passed as its own argument and is
/// not quoted: `wsl.exe` treats a quoted name as part of the name, and the result is a
/// distribution that does not exist.
#[cfg(any(windows, test))]
fn wsl_arguments(distro: &str) -> Vec<String> {
    let distro = distro.trim();
    let mut args = Vec::new();
    if !distro.is_empty() {
        args.push("-d".to_owned());
        args.push(distro.to_owned());
    }
    // Start in the distribution's own home rather than in the Windows working directory, which
    // WSL would otherwise map to /mnt/c/...
    args.push("--cd".to_owned());
    args.push("~".to_owned());
    args
}

fn spawn(
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    size: Size,
) -> Result<(LocalSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: size.rows.clamp(1, u16::MAX as u32) as u16,
            cols: size.columns.clamp(1, u16::MAX as u32) as u16,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| ShellError::terminal_shell_failed(&program, format!("no pseudo terminal: {error}")))?;

    let mut command = CommandBuilder::new(&program);
    command.args(&args);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    if let Some(cwd) = cwd.filter(|path| path.is_dir()) {
        command.cwd(cwd);
    } else if let Some(home) = home_dir() {
        command.cwd(home);
    }

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| ShellError::terminal_shell_failed(&program, error.to_string()))?;
    // This process must not keep the slave side open: with it alive, the master never sees the
    // end of the shell.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| ShellError::terminal_shell_failed(&program, format!("cannot read the terminal: {error}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| ShellError::terminal_shell_failed(&program, format!("cannot write the terminal: {error}")))?;
    let killer = child.clone_killer();
    let mut child = child;

    let (events, receiver) = mpsc::channel(256);
    let (commands, command_receiver) = mpsc::channel(128);
    let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>(256);
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<Option<u32>>();

    let mut reader = reader;
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return,
                Ok(count) => {
                    if output_tx.blocking_send(buffer[..count].to_vec()).is_err() {
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            }
        }
    });
    std::thread::spawn(move || {
        let _ = exit_tx.send(child.wait().ok().map(|status| status.exit_code()));
    });

    tokio::spawn(pump(
        pair.master,
        writer,
        killer,
        program,
        command_receiver,
        output_rx,
        exit_rx,
        events,
    ));
    Ok((LocalSession { commands }, receiver))
}

async fn pump(
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
    program: String,
    mut commands: mpsc::Receiver<Command>,
    mut output: mpsc::Receiver<Vec<u8>>,
    mut exit: tokio::sync::oneshot::Receiver<Option<u32>>,
    events: mpsc::Sender<TerminalEvent>,
) {
    let mut master = Some(master);
    let writer = Arc::new(Mutex::new(writer));

    let reason = loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                Some(data) => {
                    if events.send(TerminalEvent::Data(data)).await.is_err() {
                        return;
                    }
                }
                None => {
                    // The reader ended with the shell gone. The exit thread supplies the sentence.
                    let code = wait_for_exit(&mut exit, CLOSE_GRACE).await.flatten();
                    break exit_reason(&program, code);
                }
            },
            code = &mut exit => {
                let code = code.ok().flatten();
                // Everything already written to the terminal is drained before the end is
                // announced; otherwise the last line of a command can disappear.
                while let Ok(Some(data)) = tokio::time::timeout(EXIT_DRAIN, output.recv()).await {
                    if events.send(TerminalEvent::Data(data)).await.is_err() {
                        return;
                    }
                }
                break exit_reason(&program, code);
            },
            command = commands.recv() => match command {
                Some(Command::Input(data)) => {
                    let handle = Arc::clone(&writer);
                    let written = tokio::task::spawn_blocking(move || {
                        let mut writer = match handle.lock() {
                            Ok(writer) => writer,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        writer.write_all(&data).and_then(|()| writer.flush())
                    })
                    .await;
                    match written {
                        Ok(Ok(())) => {}
                        _ => break Some(format!("the shell ({program}) stopped accepting input")),
                    }
                }
                Some(Command::Resize { columns, rows }) => {
                    if let Some(master) = master.as_ref() {
                        let _ = master.resize(PtySize {
                            rows,
                            cols: columns,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                }
                Some(Command::Close) | None => {
                    // Dropping the master closes the terminal; a shell that honours EOF exits on
                    // its own and gets to flush its history. Only a shell that outlives the grace
                    // is killed.
                    drop(master.take());
                    if wait_for_exit(&mut exit, CLOSE_GRACE).await.is_none() {
                        let _ = killer.kill();
                        let _ = wait_for_exit(&mut exit, Duration::from_secs(2)).await;
                    }
                    break None;
                }
            },
        }
    };

    let _ = events.send(TerminalEvent::Closed { reason }).await;
}

fn exit_reason(program: &str, code: Option<u32>) -> Option<String> {
    match code {
        Some(0) | None => None,
        Some(code) => Some(format!("{program} exited with code {code}")),
    }
}

async fn wait_for_exit(
    exit: &mut tokio::sync::oneshot::Receiver<Option<u32>>,
    within: Duration,
) -> Option<Option<u32>> {
    match tokio::time::timeout(within, exit).await {
        Ok(Ok(code)) => Some(code),
        Ok(Err(_)) => Some(None),
        Err(_) => None,
    }
}

/// The platform a shell is being started on. A parameter rather than `cfg!` so the argument
/// builders can be tested against both shapes on either host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Unix,
}

fn platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Unix
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    PowerShell,
    Cmd,
    Bash,
    Other,
}

fn shell_kind(program: &str) -> ShellKind {
    let name = basename(program).to_ascii_lowercase();
    match name.as_str() {
        "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe" => ShellKind::PowerShell,
        "cmd" | "cmd.exe" => ShellKind::Cmd,
        name if name.starts_with("bash") || name == "zsh" || name == "fish" => ShellKind::Bash,
        _ => ShellKind::Other,
    }
}

struct ResolvedShell {
    program: String,
    kind: ShellKind,
}

impl ResolvedShell {
    fn args(&self, platform: Platform, extra: &str) -> Vec<String> {
        let mut args = Vec::new();
        if platform == Platform::Unix {
            // A GUI process has a skeletal PATH; a login shell is what gives the shell the
            // environment a person's terminal would have.
            args.push("-l".to_owned());
        }
        args.extend(split_args(extra));
        args
    }
}

fn split_args(args: &str) -> Vec<String> {
    args.split_whitespace().map(str::to_owned).collect()
}

/// Switches a Windows ConPTY child to UTF-8.
///
/// A Chinese Windows starts the console at code page 936. The terminal on this side speaks UTF-8,
/// and every program that trusts the console code page (`cmd type`, git, node) writes GBK, so CJK
/// output garbles. `chcp 65001` moves the console code page; PowerShell caches a second decode layer
/// at start and needs `[Console]::OutputEncoding` set as well. WSL and MSYS shells manage their own
/// UTF-8 world and are left alone.
fn windows_utf8_console(platform: Platform, kind: ShellKind, mut args: Vec<String>) -> Vec<String> {
    if platform != Platform::Windows {
        return args;
    }
    const POWERSHELL_PRELUDE: &str = "chcp 65001 >$null; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8";
    match kind {
        ShellKind::Cmd => {
            let mut bootstrapped = vec!["/k".to_owned(), "chcp 65001>nul&title %COMSPEC%".to_owned()];
            bootstrapped.append(&mut args);
            bootstrapped
        }
        ShellKind::PowerShell => {
            if let Some(index) = args.iter().position(|arg| arg == "-Command")
                && let Some(payload) = args.get(index + 1)
            {
                args[index + 1] = format!("{POWERSHELL_PRELUDE}; {payload}");
                return args;
            }
            args.extend([
                "-NoExit".to_owned(),
                "-Command".to_owned(),
                POWERSHELL_PRELUDE.to_owned(),
            ]);
            args
        }
        _ => args,
    }
}

fn resolve_shell(requested: &str) -> Result<ResolvedShell, ShellError> {
    let requested = requested.trim();
    if requested.is_empty() {
        return default_shell();
    }
    for shell in local_shells() {
        if shell.id == requested
            && let Some(program) = program_for(&shell.id)
        {
            return Ok(ResolvedShell {
                kind: shell_kind(&program),
                program,
            });
        }
    }
    // A saved session may name a shell this machine does not offer, for example one copied from
    // another installation. Resolving the identifier through PATH is the one useful reading.
    if let Some(program) = lookup_program(requested) {
        return Ok(ResolvedShell {
            kind: shell_kind(&program),
            program,
        });
    }
    Err(ShellError::terminal_shell_failed(
        requested,
        "no such shell is available on this machine",
    ))
}

fn default_shell() -> Result<ResolvedShell, ShellError> {
    for shell in local_shells() {
        if let Some(program) = program_for(&shell.id) {
            return Ok(ResolvedShell {
                kind: shell_kind(&program),
                program,
            });
        }
    }
    Err(ShellError::terminal_shell_failed(
        "default",
        "this machine has no shell to start",
    ))
}

#[cfg(windows)]
fn windows_shells() -> Vec<LocalShell> {
    let mut shells = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut push = |id: &str, label: &str, program: Option<String>| {
        if program.is_some() && seen.insert(id.to_owned()) {
            shells.push(LocalShell {
                id: id.to_owned(),
                label: label.to_owned(),
            });
        }
    };
    push("pwsh", "PowerShell", lookup_program("pwsh.exe"));
    push(
        "powershell",
        "Windows PowerShell",
        lookup_program("powershell.exe").or_else(|| {
            std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .map(|root| root.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
                .filter(|path| path.is_file())
                .map(|path| path.to_string_lossy().into_owned())
        }),
    );
    push(
        "cmd",
        "Command Prompt",
        lookup_program("cmd.exe").or_else(|| {
            std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .map(|root| root.join("System32/cmd.exe"))
                .filter(|path| path.is_file())
                .map(|path| path.to_string_lossy().into_owned())
        }),
    );
    push("git-bash", "Git Bash", git_bash());
    shells
}

#[cfg(not(windows))]
fn unix_shells() -> Vec<LocalShell> {
    let candidates = unix_shell_candidates();
    let mut shells = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for program in candidates {
        let id = basename(&program).to_owned();
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        shells.push(LocalShell {
            id: id.clone(),
            label: display_name(&id),
        });
    }
    shells
}

#[cfg(any(not(windows), test))]
fn display_name(id: &str) -> String {
    match id {
        "pwsh" => "PowerShell".to_owned(),
        "powershell" => "Windows PowerShell".to_owned(),
        "cmd" => "Command Prompt".to_owned(),
        "git-bash" => "Git Bash".to_owned(),
        other => {
            let mut name = other.to_owned();
            if let Some(first) = name.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            name
        }
    }
}

fn program_for(id: &str) -> Option<String> {
    #[cfg(windows)]
    {
        match id {
            "pwsh" => lookup_program("pwsh.exe"),
            "powershell" => lookup_program("powershell.exe").or_else(|| {
                std::env::var_os("SystemRoot")
                    .map(PathBuf::from)
                    .map(|root| root.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
                    .filter(|path| path.is_file())
                    .map(|path| path.to_string_lossy().into_owned())
            }),
            "cmd" => lookup_program("cmd.exe").or_else(|| {
                std::env::var_os("SystemRoot")
                    .map(PathBuf::from)
                    .map(|root| root.join("System32/cmd.exe"))
                    .filter(|path| path.is_file())
                    .map(|path| path.to_string_lossy().into_owned())
            }),
            "git-bash" => git_bash(),
            other => lookup_program(other),
        }
    }
    #[cfg(not(windows))]
    {
        lookup_program(id)
    }
}

fn lookup_program(name: &str) -> Option<String> {
    if name.contains(['/', '\\']) {
        let path = Path::new(name);
        return path.is_file().then(|| name.to_owned());
    }
    let candidate_names = if cfg!(windows) {
        let mut names = vec![name.to_owned()];
        if !name.to_ascii_lowercase().ends_with(".exe") {
            names.push(format!("{name}.exe"));
        }
        names
    } else {
        vec![name.to_owned()]
    };
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        for candidate in &candidate_names {
            let path = dir.join(candidate);
            if path.is_file() {
                return Some(path.to_string_lossy().into_owned());
            }
        }
    }
    None
}

#[cfg(windows)]
fn git_bash() -> Option<String> {
    let mut candidates = Vec::new();
    for root in [
        std::env::var_os("ProgramFiles"),
        std::env::var_os("ProgramFiles(x86)"),
        std::env::var_os("ProgramData"),
        std::env::var_os("LOCALAPPDATA").map(|root| Path::new(&root).join("Programs").into_os_string()),
    ]
    .into_iter()
    .flatten()
    {
        let root = PathBuf::from(root);
        candidates.push(root.join("Git/bin/bash.exe"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(|| lookup_program("bash.exe"))
}

#[cfg(not(windows))]
fn unix_shell_candidates() -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(shell) = std::env::var_os("SHELL") {
        let shell = shell.to_string_lossy().into_owned();
        if !shell.is_empty() {
            candidates.push(shell);
        }
    }
    if let Ok(list) = std::fs::read_to_string("/etc/shells") {
        for line in list.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if Path::new(line).is_file() {
                candidates.push(line.to_owned());
            }
        }
    }
    if candidates.is_empty() {
        for fallback in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
            if Path::new(fallback).is_file() {
                candidates.push(fallback.to_owned());
            }
        }
    }
    candidates
}

fn basename(program: &str) -> &str {
    program.rsplit(['/', '\\']).next().unwrap_or(program)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wsl_argument_list_keeps_the_distribution_name_unquoted() {
        assert_eq!(wsl_arguments("Ubuntu"), vec!["-d", "Ubuntu", "--cd", "~"]);
        assert_eq!(wsl_arguments("  "), vec!["--cd", "~"]);
    }

    #[test]
    fn the_distribution_list_decodes_utf16_with_or_without_a_mark() {
        let text = "Ubuntu\r\ndocker-desktop\r\n";
        let mut bytes = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_wsl_list(&bytes), vec!["Ubuntu", "docker-desktop"]);

        let bare = text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        assert_eq!(decode_wsl_list(&bare), vec!["Ubuntu", "docker-desktop"]);
    }

    #[test]
    fn a_windows_console_gets_utf8_before_it_starts() {
        let cmd = windows_utf8_console(Platform::Windows, ShellKind::Cmd, Vec::new());
        assert_eq!(cmd, vec!["/k", "chcp 65001>nul&title %COMSPEC%"]);

        let powershell = windows_utf8_console(Platform::Windows, ShellKind::PowerShell, Vec::new());
        assert_eq!(powershell[0], "-NoExit");
        assert_eq!(powershell[1], "-Command");
        assert!(powershell[2].starts_with("chcp 65001"));

        let bash = windows_utf8_console(Platform::Windows, ShellKind::Bash, vec!["-l".to_owned()]);
        assert_eq!(bash, vec!["-l"], "MSYS shells manage their own UTF-8");

        let unix = windows_utf8_console(Platform::Unix, ShellKind::PowerShell, Vec::new());
        assert!(unix.is_empty());
    }

    #[test]
    fn a_unix_shell_starts_as_a_login_shell() {
        let shell = ResolvedShell {
            program: "/bin/bash".to_owned(),
            kind: ShellKind::Bash,
        };
        assert_eq!(shell.args(Platform::Unix, ""), vec!["-l"]);
        assert_eq!(shell.args(Platform::Unix, "--noprofile"), vec!["-l", "--noprofile"]);
    }

    #[test]
    fn shell_names_are_identifiers_not_paths() {
        assert_eq!(basename("/bin/zsh"), "zsh");
        assert_eq!(basename(r"C:\Program Files\Git\bin\bash.exe"), "bash.exe");
        assert_eq!(display_name("pwsh"), "PowerShell");
        assert_eq!(display_name("git-bash"), "Git Bash");
        assert_eq!(display_name("zsh"), "Zsh");
    }

    #[test]
    fn a_shell_name_with_a_directory_is_taken_as_a_path() {
        assert!(lookup_program("definitely-not-a-shell").is_none());
    }
}

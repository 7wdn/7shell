//! Running a container command on this machine.
//!
//! The process is started from an argv with no shell in between: nothing here splits a string, and
//! nothing here quotes anything. The one exception is a script the *container* runs (`sh -c` inside
//! `docker exec`), which is an argument to the runtime's CLI like any other.
//!
//! The command paths (`run`, `run_stdin`, `run_stream_stdin`) use Tokio's process handle and
//! `kill_on_drop`: a tool call that is cancelled drops the future, which kills the runtime CLI
//! rather than leaving a pull or a remove running after the person said stop. The line and chunk
//! streaming paths still use blocking threads because they hand output to a reader as it arrives;
//! their own stop closures own that child, and a reader that goes away kills it.
//!
//! Two platform details are deliberate. The runtime binary is looked up here rather than by the
//! operating system, because a window opened from a desktop launcher inherits a PATH that omits
//! Docker Desktop's install directory, and because "not found" should say so instead of surfacing
//! as a spawn error. And on Windows the child is started without a console window: this process has
//! no console of its own, so a console child would flash a black window on every refresh.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child as PtyChild, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use sevenshell_error::ShellError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt as _};
use tokio::sync::mpsc;

use crate::runner::{ByteStream, InputStream, LineStream, PtyControl, PtySession, StreamEnding};

/// The most a single answer may carry. A listing of this size is already past what a person reads;
/// an `inspect` that exceeds it is refused so the interface says why instead of parsing a truncated
/// document.
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
/// How much of the runtime's own complaint travels in an error's detail.
const MAX_DETAIL_CHARS: usize = 4096;
/// How long a process gets to exit after its output ended before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// How often a waiting thread asks whether the process has exited.
const EXIT_POLL: Duration = Duration::from_millis(25);
/// How many stderr lines a failed stream still shows.
const MAX_ERROR_LINES: usize = 32;
const STREAM_LINE_CAPACITY: usize = 256;
const PTY_OUTPUT_CAPACITY: usize = 256;
const PTY_CONTROL_CAPACITY: usize = 64;
/// How much of a streamed command's output waits for its reader. The queue is the memory bound:
/// a download never grows past this many chunks, whatever the size of the tree.
const STREAM_BACKLOG: usize = 8;
/// The size of one chunk read from a streamed command's pipe.
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// Directories a desktop launcher's PATH usually omits. Docker Desktop on Windows and macOS does
/// not always add its CLI to PATH, and a macOS application inherits almost nothing.
const CANDIDATE_DIRECTORIES: &[&str] = &[
    "/usr/local/bin",
    "/opt/homebrew/bin",
    "/snap/bin",
    "/usr/bin",
    r"C:\Program Files\Docker\Docker\resources\bin",
    r"C:\Program Files\Rancher Desktop\resources\resources\win32\bin",
    r"C:\Windows\System32",
];

pub(crate) async fn run(argv: &[String]) -> Result<String, ShellError> {
    let mut command = async_command(argv)?;
    let mut child = command.spawn().map_err(|error| io_error(argv, &error))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ShellError::container_command_failed("the command", "no standard output"))?;
    let stderr = child.stderr.take();
    let out = tokio::spawn(read_capped_async(Some(stdout)));
    let err = tokio::spawn(read_capped_async(stderr));
    let status = child.wait().await.map_err(|error| io_error(argv, &error))?;
    let stdout = join_async(out).await?;
    let stderr = join_async(err).await?;
    finish(status, stdout, stderr)
}

pub(crate) async fn run_stdin(argv: &[String], input: Vec<u8>) -> Result<String, ShellError> {
    let mut command = async_command(argv)?;
    command.stdin(Stdio::piped());
    let mut child = command.spawn().map_err(|error| io_error(argv, &error))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ShellError::container_command_failed("the command", "no standard output"))?;
    let stderr = child.stderr.take();
    if let Some(mut stdin) = child.stdin.take() {
        // The write is its own task: an archive can be larger than a pipe buffer, and writing it
        // before reading anything would deadlock on a child that fills stdout first.
        tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
            // Dropping the handle closes the pipe, which is the EOF the reader waits for.
        });
    }
    let out = tokio::spawn(read_capped_async(Some(stdout)));
    let err = tokio::spawn(read_capped_async(stderr));
    let status = child.wait().await.map_err(|error| io_error(argv, &error))?;
    let stdout = join_async(out).await?;
    let stderr = join_async(err).await?;
    finish(status, stdout, stderr)
}

async fn read_capped_async<R>(reader: Option<R>) -> Result<Vec<u8>, ShellError>
where
    R: AsyncRead + Unpin,
{
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    let mut buffer = Vec::new();
    reader
        .take(MAX_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .await
        .map_err(|error| ShellError::container_command_failed("the command", error.to_string()))?;
    if buffer.len() > MAX_OUTPUT_BYTES {
        return Err(ShellError::container_command_failed(
            "the command",
            format!("its answer is larger than {MAX_OUTPUT_BYTES} bytes"),
        ));
    }
    Ok(buffer)
}

async fn join_async(reader: tokio::task::JoinHandle<Result<Vec<u8>, ShellError>>) -> Result<Vec<u8>, ShellError> {
    reader
        .await
        .unwrap_or_else(|error| Err(ShellError::app_internal(error.to_string())))
}

pub(crate) async fn run_stream_stdin(argv: &[String], input: &mut InputStream) -> Result<String, ShellError> {
    let mut command = async_command(argv)?;
    command.stdin(Stdio::piped());
    let mut child = command.spawn().map_err(|error| io_error(argv, &error))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ShellError::container_command_failed("the command", "no standard output"))?;
    let stderr = child.stderr.take();
    let mut chunks = input.take_chunks();
    if let Some(mut stdin) = child.stdin.take() {
        // The writer is its own task because the two pipes have to be drained at the same time: a
        // command that prints while it reads would otherwise fill the output pipe and stop reading,
        // and every chunk after that would wait for a command that is waiting for us.
        tokio::spawn(async move {
            while let Some(chunk) = chunks.recv().await {
                if stdin.write_all(&chunk).await.is_err() {
                    // The command stopped reading — it has already decided what to answer, and its
                    // exit status is a better explanation than a broken pipe on this side.
                    break;
                }
            }
            // Dropping the handle closes the pipe, which is the end of input the far side waits for.
        });
    }
    let out = tokio::spawn(read_capped_async(Some(stdout)));
    let err = tokio::spawn(read_capped_async(stderr));
    let status = child.wait().await.map_err(|error| io_error(argv, &error))?;
    let stdout = join_async(out).await?;
    let stderr = join_async(err).await?;
    finish(status, stdout, stderr)
}

pub(crate) async fn stream(argv: &[String]) -> Result<LineStream, ShellError> {
    let mut command = command(argv)?;
    command.stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| io_error(argv, &error))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ShellError::container_command_failed("the command", "no standard output"))?;
    let stderr = child.stderr.take();
    // The stop closure and the reader thread share the child: one kills it, the other reaps it.
    let child = Arc::new(Mutex::new(child));

    let (lines, receiver) = mpsc::channel(STREAM_LINE_CAPACITY);
    let stopper = Arc::clone(&child);
    let stop = Box::new(move || kill(&stopper));

    std::thread::spawn(move || {
        let stderr = std::thread::spawn(move || read_capped(stderr));
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::new();
        let mut abandoned = false;
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) => break,
                Ok(_) => {
                    trim_line_end(&mut buffer);
                    let line = String::from_utf8_lossy(&buffer).into_owned();
                    if lines.blocking_send(line).is_err() {
                        // Nobody is reading any more; the process should not keep running.
                        abandoned = true;
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if abandoned {
            kill(&child);
        }
        let status = wait_bounded(&child, EXIT_GRACE);
        // A pull that fails writes its reason to stderr and nothing to stdout; forwarding those
        // lines is the only way the person watching the stream learns why it stopped.
        if !abandoned
            && let Ok(status) = status
            && !status.success()
            && let Ok(Ok(stderr)) = stderr.join().map_err(|_| ())
        {
            for line in String::from_utf8_lossy(&stderr).lines().take(MAX_ERROR_LINES) {
                if lines.blocking_send(line.to_owned()).is_err() {
                    break;
                }
            }
        }
    });

    Ok(LineStream::new(receiver, stop))
}

pub(crate) async fn open_pty(argv: &[String], columns: u32, rows: u32) -> Result<PtySession, ShellError> {
    let (program, arguments) = split(argv)?;
    let program = program_path(program)?;
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(pty_size(columns, rows))
        .map_err(|error| ShellError::container_command_failed("the terminal", error.to_string()))?;

    let mut builder = CommandBuilder::new(&program);
    builder.args(arguments);
    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| ShellError::container_command_failed("the terminal", error.to_string()))?;
    // The parent must not keep the slave side open; with it alive the master never sees the end.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| ShellError::container_command_failed("the terminal", error.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| ShellError::container_command_failed("the terminal", error.to_string()))?;
    let killer = child.clone_killer();

    let (output_sender, output) = mpsc::channel::<Vec<u8>>(PTY_OUTPUT_CAPACITY);
    let (control, control_receiver) = mpsc::channel::<PtyControl>(PTY_CONTROL_CAPACITY);

    let mut reader = reader;
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return,
                Ok(count) => {
                    if output_sender.blocking_send(buffer[..count].to_vec()).is_err() {
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            }
        }
    });

    let writer = Arc::new(Mutex::new(writer));
    let master = pair.master;
    tokio::spawn(pump(master, writer, killer, child, control_receiver));
    Ok(PtySession { output, control })
}

async fn pump(
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
    mut child: Box<dyn PtyChild + Send + Sync>,
    mut control: mpsc::Receiver<PtyControl>,
) {
    let mut master = Some(master);
    while let Some(command) = control.recv().await {
        match command {
            PtyControl::Write(data) => {
                let writer = Arc::clone(&writer);
                let _ = tokio::task::spawn_blocking(move || {
                    let mut writer = writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    writer.write_all(&data).and_then(|()| writer.flush())
                })
                .await;
            }
            PtyControl::Resize { columns, rows } => {
                if let Some(master) = master.as_ref() {
                    let _ = master.resize(pty_size(columns, rows));
                }
            }
            PtyControl::Close => break,
        }
    }
    drop(master.take());
    let _ = killer.kill();
    let _ = tokio::task::spawn_blocking(move || {
        let _ = child.wait();
    })
    .await;
}

fn pty_size(columns: u32, rows: u32) -> PtySize {
    PtySize {
        rows: rows.clamp(1, u16::MAX as u32) as u16,
        cols: columns.clamp(1, u16::MAX as u32) as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn split(argv: &[String]) -> Result<(&str, &[String]), ShellError> {
    match argv.split_first() {
        Some((program, arguments)) => Ok((program, arguments)),
        None => Err(ShellError::app_internal("a container command needs a program")),
    }
}

/// Runs one command and hands its raw standard output over in chunks.
pub(crate) async fn stream_bytes(argv: &[String]) -> Result<ByteStream, ShellError> {
    let mut command = command(argv)?;
    let mut child = command.spawn().map_err(|error| io_error(argv, &error))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ShellError::container_command_failed("the command", "no standard output"))?;
    let stderr = child.stderr.take();
    // One side kills the child when the reader goes away; the other reaps it.
    let child = Arc::new(Mutex::new(child));
    let (sender, chunks) = mpsc::channel::<Vec<u8>>(STREAM_BACKLOG);
    let ending = Arc::new(Mutex::new(StreamEnding::default()));
    let recorded = Arc::clone(&ending);

    std::thread::spawn(move || {
        // Standard error is drained beside standard output: a command that writes a warning while
        // producing a large answer would otherwise block on a full pipe.
        let stderr = std::thread::spawn(move || read_capped(stderr));
        let mut reader = BufReader::new(stdout);
        let mut buffer = vec![0u8; STREAM_CHUNK_BYTES];
        let mut abandoned = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if sender.blocking_send(buffer[..count].to_vec()).is_err() {
                        // Nobody is reading any more; the command should not keep running.
                        abandoned = true;
                        break;
                    }
                }
                Err(error) => {
                    if let Ok(mut ending) = recorded.lock() {
                        ending.detail = format!("the output could not be read: {error}");
                    }
                    break;
                }
            }
        }
        if abandoned {
            kill(&child);
        }
        let status = wait_bounded(&child, EXIT_GRACE);
        let stderr = stderr.join().ok().and_then(Result::ok).unwrap_or_default();
        if let Ok(mut ending) = recorded.lock() {
            match status {
                Ok(status) => {
                    ending.exit_code = status.code().map(|code| code as u32);
                    // The command's own words are what a person can act on; the status is only the
                    // fallback when it said nothing.
                    if !status.success() && ending.detail.is_empty() {
                        ending.detail = bounded(String::from_utf8_lossy(&stderr).trim());
                    }
                }
                Err(error) => ending.detail = error.to_string(),
            }
        }
    });

    Ok(ByteStream::local(chunks, ending))
}

/// Waits for a streamed process to exit, killing it if it outlives the grace period.
fn wait_bounded(child: &Arc<Mutex<Child>>, grace: Duration) -> Result<ExitStatus, ShellError> {
    let deadline = Instant::now() + grace;
    loop {
        let status = {
            let mut guard = child.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.try_wait()
        };
        match status {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => {
                return Err(ShellError::container_command_failed("the command", error.to_string()));
            }
        }
        if Instant::now() >= deadline {
            kill(child);
            let mut guard = child.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            return guard
                .wait()
                .map_err(|error| ShellError::container_command_failed("the command", error.to_string()));
        }
        std::thread::sleep(EXIT_POLL);
    }
}

fn kill(child: &Arc<Mutex<Child>>) {
    let mut guard = child.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = guard.kill();
}

fn command(argv: &[String]) -> Result<Command, ShellError> {
    let (program, arguments) = split(argv)?;
    let program = program_path(program)?;
    let mut command = Command::new(&program);
    command.args(arguments);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    hide_console(&mut command);
    Ok(command)
}

/// The same lookup and platform flags, on a handle that can kill its child when dropped.
///
/// The drop is the cancellation: the tool layer drops the future when a person stops a call, and a
/// `spawn_blocking` task had no handle to drop because it was waiting inside `Command::output`.
fn async_command(argv: &[String]) -> Result<tokio::process::Command, ShellError> {
    let (program, arguments) = split(argv)?;
    let program = program_path(program)?;
    let mut command = tokio::process::Command::new(&program);
    command.args(arguments);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    hide_console_async(&mut command);
    Ok(command)
}

fn hide_console_async(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: this process has no console, so a console child would flash one.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Finds the runtime's executable the way a shell would, plus the directories a desktop launcher's
/// PATH does not carry. A name with a path separator is taken as-is.
fn program_path(program: &str) -> Result<PathBuf, ShellError> {
    let requested = std::path::Path::new(program);
    if requested.components().count() > 1 {
        return if requested.is_file() {
            Ok(requested.to_path_buf())
        } else {
            Err(ShellError::container_unavailable(
                program,
                "the program is not a file at that path",
            ))
        };
    }

    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    directories.extend(CANDIDATE_DIRECTORIES.iter().map(PathBuf::from));

    for directory in directories {
        for name in executable_names(program) {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(ShellError::container_unavailable(
        program,
        "the program was not found on PATH",
    ))
}

fn executable_names(program: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        if std::path::Path::new(program).extension().is_some() {
            return vec![program.to_owned()];
        }
        // The extensions come first, and the bare name last. Docker Desktop's bin directory holds
        // both `docker` (a shell script for git-bash and WSL) and `docker.exe`; only the latter can
        // be started directly, and a process started from the script fails with "not a valid Win32
        // application". `CreateProcess` appends `.exe` when no extension is given, which is why the
        // script was never a problem until a pseudo terminal named the file explicitly.
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        let mut names: Vec<String> = extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| format!("{program}{}", extension.to_ascii_lowercase()))
            .collect();
        names.push(program.to_owned());
        names
    }
    #[cfg(not(windows))]
    {
        vec![program.to_owned()]
    }
}

fn read_capped<R: Read>(reader: Option<R>) -> Result<Vec<u8>, ShellError> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    let mut buffer = Vec::new();
    reader
        .take(MAX_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|error| ShellError::container_command_failed("the command", error.to_string()))?;
    if buffer.len() > MAX_OUTPUT_BYTES {
        return Err(ShellError::container_command_failed(
            "the command",
            format!("the runtime printed more than {} MiB", MAX_OUTPUT_BYTES / (1024 * 1024)),
        ));
    }
    Ok(buffer)
}

fn finish(status: ExitStatus, stdout: Vec<u8>, stderr: Vec<u8>) -> Result<String, ShellError> {
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    if status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&stderr);
    // Some CLIs (WSLC's `exec` among them) write the reason to stdout and leave stderr empty; only
    // reading stderr turns a clear refusal into "exit status 126".
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    Err(ShellError::container_command_failed("the command", bounded(detail)))
}

fn bounded(text: &str) -> String {
    if text.chars().count() <= MAX_DETAIL_CHARS {
        return text.to_owned();
    }
    let mut bounded: String = text.chars().take(MAX_DETAIL_CHARS).collect();
    bounded.push_str("...");
    bounded
}

fn trim_line_end(buffer: &mut Vec<u8>) {
    while matches!(buffer.last(), Some(b'\n' | b'\r')) {
        buffer.pop();
    }
}

fn io_error(argv: &[String], error: &std::io::Error) -> ShellError {
    let program = argv.first().map(String::as_str).unwrap_or("the runtime");
    if error.kind() == std::io::ErrorKind::NotFound {
        ShellError::container_unavailable(program, error.to_string())
    } else {
        ShellError::container_command_failed("the command", error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dropping the future is what a cancelled tool call does. The child must not outlive it: a
    /// container action that keeps running after the person stopped is the side effect this test
    /// exists to forbid. The marker is written two seconds in, so a leaked process is visible.
    #[tokio::test]
    async fn dropping_a_run_kills_the_runtime_process_before_it_finishes() {
        let marker = std::env::temp_dir().join(format!("seven-shell-container-{}.txt", uuid::Uuid::new_v4()));
        let argv = delayed_marker(&marker);

        let mut running = Box::pin(run(&argv));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut running)
                .await
                .is_err(),
            "the command was supposed to still be running"
        );
        drop(running);

        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            !marker.exists(),
            "a dropped container command kept running: {}",
            marker.display()
        );
        let _ = std::fs::remove_file(marker);
    }

    #[tokio::test]
    async fn dropping_a_streamed_run_kills_the_runtime_process_before_it_finishes() {
        let marker = std::env::temp_dir().join(format!("seven-shell-container-{}.txt", uuid::Uuid::new_v4()));
        let argv = delayed_marker(&marker);
        let mut input = InputStream::from_reader(tokio::io::empty());

        let mut running = Box::pin(run_stream_stdin(&argv, &mut input));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut running)
                .await
                .is_err(),
            "the command was supposed to still be running"
        );
        drop(running);

        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            !marker.exists(),
            "a dropped streamed container command kept running: {}",
            marker.display()
        );
        let _ = std::fs::remove_file(marker);
    }

    #[cfg(windows)]
    fn delayed_marker(marker: &std::path::Path) -> Vec<String> {
        let marker = marker.display().to_string().replace('\'', "''");
        vec![
            "powershell".to_owned(),
            "-NoProfile".to_owned(),
            "-Command".to_owned(),
            format!("Start-Sleep -Seconds 2; Set-Content -Path '{marker}' -Value late"),
        ]
    }

    #[cfg(not(windows))]
    fn delayed_marker(marker: &std::path::Path) -> Vec<String> {
        let marker = marker.display().to_string().replace('\'', "'\''");
        vec![
            "sh".to_owned(),
            "-c".to_owned(),
            format!("sleep 2; printf late > '{marker}'"),
        ]
    }
}

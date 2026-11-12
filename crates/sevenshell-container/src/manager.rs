//! The connections a container tab holds open.
//!
//! One per application, like the session registry: a container tab asks it to connect, then
//! addresses the connection by an id it was given. The implementation lives behind this type so
//! the host never learns which runtime is on the other side.
//!
//! Nothing in here runs a command without first building its argv, and the builders are the only
//! place that refuses an argument. That order is the whole option-injection defence: a container id
//! that begins with `-` is rejected while it is still a string, before a process exists to receive
//! it.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use sevenshell_error::ShellError;
use sevenshell_types::{
    ContainerCommitRequest, ContainerCreateRequest, ContainerDetail, ContainerImage, ContainerImageTransfer,
    ContainerRegistryLogin, ContainerRuntime, ContainerRuntimeInfo, ContainerStats, ContainerSummary, ImageLayer,
};

use crate::commands::{self, ImageTransfer};
use crate::parse;
use crate::runner::{ChunkSender, InputStream, LineStream, PtySession, Runner};

/// The largest archive this build will hand to a runtime.
///
/// This is no longer a memory bound: the file is streamed into the runtime's standard input, so a
/// 4 GiB archive costs the same as a 4 KiB one. It stays a guard against a path that was never an
/// image archive — a disk image, a sparse file — being pumped into a runtime for hours, and 64 GiB
/// is far past anything `docker save` produces for one image.
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// The five things a runtime's CLI can be asked to do, as one interface.
///
/// A trait rather than calls straight to [`Runner`] so the guard tests can hand a provider a
/// recorder and prove that a rejected argument never reached the step that starts a process.
#[async_trait]
trait CommandRunner: Send + Sync {
    async fn run(&self, argv: &[String]) -> Result<String, ShellError>;
    async fn run_stdin(&self, argv: &[String], input: Vec<u8>) -> Result<String, ShellError>;
    async fn stream_stdin(&self, argv: &[String], input: InputStream) -> Result<String, ShellError>;
    async fn stream(&self, argv: &[String]) -> Result<LineStream, ShellError>;
    async fn open_pty(&self, argv: &[String], columns: u32, rows: u32) -> Result<PtySession, ShellError>;
}

#[async_trait]
impl CommandRunner for Runner {
    async fn run(&self, argv: &[String]) -> Result<String, ShellError> {
        Runner::run(self, argv).await
    }

    async fn run_stdin(&self, argv: &[String], input: Vec<u8>) -> Result<String, ShellError> {
        Runner::run_stdin(self, argv, input).await
    }

    async fn stream_stdin(&self, argv: &[String], input: InputStream) -> Result<String, ShellError> {
        Runner::run_stream_stdin(self, argv, input).await
    }

    async fn stream(&self, argv: &[String]) -> Result<LineStream, ShellError> {
        Runner::stream(self, argv).await
    }

    async fn open_pty(&self, argv: &[String], columns: u32, rows: u32) -> Result<PtySession, ShellError> {
        Runner::open_pty(self, argv, columns, rows).await
    }
}

/// One runtime, on one host, in one namespace.
pub struct Provider {
    runtime: ContainerRuntime,
    namespace: String,
    runner: Arc<dyn CommandRunner>,
    /// The concrete runner this provider was built from, when it was built from one. A file browser
    /// runs its own commands on the same connection, and a trait object cannot be turned back into
    /// the handle that knows where it runs; a provider built around a recorder therefore has none.
    handle: Option<Runner>,
}

// The runner is a process handle or a session; printing it would print neither anything a log can
// use nor anything safe to paste into a report.
impl std::fmt::Debug for Provider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Provider")
            .field("runtime", &self.runtime)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl Provider {
    /// A provider that runs its commands through `runner`. The namespace is refused here rather
    /// than at first use: a tab that cannot name a valid namespace has no business connecting.
    pub fn new(runtime: ContainerRuntime, namespace: &str, runner: Runner) -> Result<Self, ShellError> {
        let mut provider = Self::from_parts(runtime, namespace, Arc::new(runner.clone()))?;
        provider.handle = Some(runner);
        Ok(provider)
    }

    fn from_parts(
        runtime: ContainerRuntime,
        namespace: &str,
        runner: Arc<dyn CommandRunner>,
    ) -> Result<Self, ShellError> {
        Ok(Self {
            runtime,
            namespace: namespace_for(runtime, namespace)?,
            runner,
            handle: None,
        })
    }

    pub fn runtime(&self) -> ContainerRuntime {
        self.runtime
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The runner this provider was built from, for a caller that has to keep using the same
    /// connection — the file browser runs one `exec` per file operation on it.
    ///
    /// A provider built around a recorder has no runner to hand out, and says so rather than
    /// pretending to have one.
    pub fn runner(&self) -> Result<Runner, ShellError> {
        self.handle
            .clone()
            .ok_or_else(|| ShellError::app_unsupported("a container provider without a runner to hand out"))
    }

    pub async fn list(&self) -> Result<Vec<ContainerSummary>, ShellError> {
        let output = self.runner.run(&commands::list(self.runtime, &self.namespace)).await?;
        Ok(parse::containers(self.runtime, &output))
    }

    pub async fn inspect(&self, id: &str) -> Result<ContainerDetail, ShellError> {
        let argv = commands::inspect(self.runtime, &self.namespace, id)?;
        let output = self.runner.run(&argv).await?;
        parse::detail(&output)
    }

    pub async fn action(&self, id: &str, action: &str) -> Result<(), ShellError> {
        let argv = commands::action(self.runtime, &self.namespace, action, id)?;
        self.runner.run(&argv).await.map(|_| ())
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<(), ShellError> {
        let argv = commands::rename(self.runtime, &self.namespace, id, name)?;
        self.runner.run(&argv).await.map(|_| ())
    }

    pub async fn create(&self, request: &ContainerCreateRequest) -> Result<(), ShellError> {
        let argv = commands::create(self.runtime, &self.namespace, request)?;
        self.runner.run(&argv).await.map(|_| ())
    }

    pub async fn stats(&self) -> Result<Vec<ContainerStats>, ShellError> {
        let output = self.runner.run(&commands::stats(self.runtime, &self.namespace)).await?;
        Ok(parse::stats(&output))
    }

    pub async fn images(&self) -> Result<Vec<ContainerImage>, ShellError> {
        let output = self
            .runner
            .run(&commands::images(self.runtime, &self.namespace))
            .await?;
        Ok(parse::images(&output))
    }

    /// The runtime's own JSON for the image, untouched: its shape differs from a container's and
    /// the inspection tab shows it as printed.
    pub async fn image_inspect(&self, image: &str) -> Result<String, ShellError> {
        let argv = commands::image_inspect(self.runtime, &self.namespace, image)?;
        self.runner.run(&argv).await
    }

    pub async fn image_history(&self, image: &str) -> Result<Vec<ImageLayer>, ShellError> {
        if self.runtime == ContainerRuntime::Wslc {
            // No `history` subcommand; the layer digests are all its inspect carries.
            let argv = commands::wslc_image_inspect(&self.namespace, image)?;
            let output = self.runner.run(&argv).await?;
            return parse::image_layers_from_inspect(&output);
        }
        let argv = commands::image_history(self.runtime, &self.namespace, image)?;
        let output = self.runner.run(&argv).await?;
        Ok(parse::image_history(&output))
    }

    /// A pull or a push, line by line, stoppable by the caller.
    pub async fn image_transfer(
        &self,
        direction: ImageTransfer,
        request: &ContainerImageTransfer,
    ) -> Result<LineStream, ShellError> {
        let argv = commands::transfer(self.runtime, &self.namespace, direction, request)?;
        self.runner.stream(&argv).await
    }

    /// The two host commands are two calls, so they get two names; the argv is the same builder.
    pub async fn image_pull(&self, request: &ContainerImageTransfer) -> Result<LineStream, ShellError> {
        self.image_transfer(ImageTransfer::Pull, request).await
    }

    pub async fn image_push(&self, request: &ContainerImageTransfer) -> Result<LineStream, ShellError> {
        self.image_transfer(ImageTransfer::Push, request).await
    }

    pub async fn image_remove(&self, image: &str) -> Result<(), ShellError> {
        let argv = commands::remove_image(self.runtime, &self.namespace, image)?;
        self.runner.run(&argv).await.map(|_| ())
    }

    pub async fn image_prune(&self) -> Result<(), ShellError> {
        let argv = commands::image_prune(self.runtime, &self.namespace);
        self.runner.run(&argv).await.map(|_| ())
    }

    pub async fn image_tag(&self, image: &str, repository_tag: &str) -> Result<(), ShellError> {
        let argv = commands::tag(self.runtime, &self.namespace, image, repository_tag)?;
        self.runner.run(&argv).await.map(|_| ())
    }

    /// Loads an archive that the caller already holds. The bytes travel on standard input; they are
    /// never part of the argv and never cross to the interface.
    pub async fn image_load(&self, archive: Vec<u8>) -> Result<String, ShellError> {
        self.require_image_load()?;
        let argv = commands::load(self.runtime, &self.namespace);
        let output = self.runner.run_stdin(&argv, archive).await?;
        Ok(output.trim().to_owned())
    }

    /// Loads an archive from a file on this machine, which is where the native dialog left it.
    ///
    /// The file is streamed rather than read: its bytes go into the runtime's standard input as they
    /// leave the disk, so an archive is never a value in this process and its size stops being a
    /// memory question. What is still checked before the command starts is that the path is a
    /// regular file of a plausible size — a wrong file should be refused by name, not by having
    /// fed a runtime for an hour.
    pub async fn image_load_file(&self, path: &Path) -> Result<String, ShellError> {
        self.image_load_file_with(path, |_| true).await
    }

    /// The same load, counting the archive bytes it hands to the runtime and stopping on request.
    ///
    /// The observer runs on the thread reading the file, once per chunk; its total is the archive's
    /// size, which the caller already knows. `false` ends the transfer before the next chunk is
    /// written, and the row reports it as cancelled.
    pub async fn image_load_file_with<F>(&self, path: &Path, observe: F) -> Result<String, ShellError>
    where
        F: Fn(u64) -> bool + Send + Sync + 'static,
    {
        self.require_image_load()?;
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| archive_read_failure(&error))?;
        if !metadata.is_file() {
            return Err(ShellError::container_command_failed(
                "the image archive",
                "that is not a regular file",
            ));
        }
        ensure_archive_fits(metadata.len(), MAX_ARCHIVE_BYTES)?;
        let path = path.to_path_buf();
        let input = InputStream::from_producer(move |sink| read_archive(&path, sink, &observe));
        let argv = commands::load(self.runtime, &self.namespace);
        let output = self.runner.stream_stdin(&argv, input).await?;
        Ok(output.trim().to_owned())
    }

    /// Whether an archive can be handed to this runtime at all.
    ///
    /// Docker and Podman both read a save archive on standard input. nerdctl and WSLC are left out
    /// because neither was exercised on a machine this build can reach, and "not supported here" is
    /// a truer answer than an untested command line.
    fn require_image_load(&self) -> Result<(), ShellError> {
        if self.runtime == ContainerRuntime::Docker || self.runtime == ContainerRuntime::Podman {
            return Ok(());
        }
        Err(ShellError::app_unsupported("image archives on this runtime"))
    }

    pub async fn image_commit(&self, request: &ContainerCommitRequest) -> Result<String, ShellError> {
        let argv = commands::commit(self.runtime, &self.namespace, request)?;
        let output = self.runner.run(&argv).await?;
        Ok(output.trim().to_owned())
    }

    /// The one secret in this module goes in on standard input, never in the argv.
    pub async fn registry_login(&self, login: &ContainerRegistryLogin) -> Result<(), ShellError> {
        let argv = commands::login(self.runtime, login)?;
        let mut input = login.secret.value.clone().into_bytes();
        input.push(b'\n');
        self.runner.run_stdin(&argv, input).await.map(|_| ())
    }

    pub async fn namespaces(&self) -> Result<Vec<String>, ShellError> {
        if self.runtime != ContainerRuntime::Nerdctl {
            return Err(ShellError::app_unsupported("container namespaces on this runtime"));
        }
        let output = self.runner.run(&commands::namespaces(self.runtime)).await?;
        Ok(parse::namespaces(&output))
    }

    /// `info` and `version` say different halves of the same answer and neither is cheap, so they
    /// run together. One of them failing still leaves a page worth showing; both failing means the
    /// engine is not answering, and that is the error the person needs.
    pub async fn runtime_info(&self) -> Result<ContainerRuntimeInfo, ShellError> {
        let info = commands::info(self.runtime, &self.namespace);
        let version = commands::version(self.runtime, &self.namespace);
        let (info, version) = tokio::join!(self.runner.run(&info), self.runner.run(&version));
        let (info, version) = match (info, version) {
            (Ok(info), Ok(version)) => (Some(info), Some(version)),
            (Ok(info), Err(_)) => (Some(info), None),
            (Err(_), Ok(version)) => (None, Some(version)),
            (Err(error), Err(_)) => return Err(error),
        };
        Ok(parse::runtime_info(self.runtime, info.as_deref(), version.as_deref()))
    }

    /// Follows a container's log from now on. The caller owns the stream and stops it.
    pub async fn logs(&self, id: &str, tail: u32, timestamps: bool) -> Result<LineStream, ShellError> {
        let argv = commands::logs(self.runtime, &self.namespace, id, tail, true, timestamps)?;
        self.runner.stream(&argv).await
    }

    /// The argv of an interactive `exec`, for callers that need to see it before running it.
    pub fn exec_argv(&self, id: &str, shell: &str) -> Result<Vec<String>, ShellError> {
        commands::exec(self.runtime, &self.namespace, id, shell)
    }

    pub async fn open_exec(&self, id: &str, shell: &str, columns: u32, rows: u32) -> Result<PtySession, ShellError> {
        let argv = self.exec_argv(id, shell)?;
        self.runner.open_pty(&argv, columns, rows).await
    }

    fn with_namespace(&self, namespace: &str) -> Result<Self, ShellError> {
        Ok(Self {
            runtime: self.runtime,
            namespace: namespace_for(self.runtime, namespace)?,
            runner: Arc::clone(&self.runner),
            handle: self.handle.clone(),
        })
    }
}

/// An empty namespace is the runtime's default, except on nerdctl where the default is named and
/// the flag still has to be passed: its other namespaces are invisible without it.
fn namespace_for(runtime: ContainerRuntime, namespace: &str) -> Result<String, ShellError> {
    commands::validate_namespace(namespace)?;
    let namespace = namespace.trim();
    if runtime == ContainerRuntime::Nerdctl && namespace.is_empty() {
        return Ok("default".to_owned());
    }
    Ok(namespace.to_owned())
}

/// Refuses an image archive this build will not hand to a runtime.
///
/// The ceiling is a parameter rather than the constant so a test can prove the refusal without
/// writing a sixty-gigabyte file, and so the size rule is exercised by the same code that enforces
/// it.
fn ensure_archive_fits(size: u64, ceiling: u64) -> Result<(), ShellError> {
    if size > ceiling {
        return Err(ShellError::container_command_failed(
            "the image archive",
            format!(
                "the archive is {} MiB and this build loads at most {} MiB",
                mebibytes(size),
                mebibytes(ceiling)
            ),
        ));
    }
    Ok(())
}

fn archive_read_failure(error: &std::io::Error) -> ShellError {
    ShellError::container_command_failed("the image archive", format!("the file could not be read: {error}"))
}

/// Hands one archive to the runtime, reporting each chunk and stopping when asked.
fn read_archive(
    path: &Path,
    sink: &mut ChunkSender,
    observe: &(dyn Fn(u64) -> bool + Send + Sync),
) -> Result<(), ShellError> {
    let mut file = std::fs::File::open(path).map_err(|error| archive_read_failure(&error))?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| archive_read_failure(&error))?;
        if read == 0 {
            return Ok(());
        }
        sink.write_all(&buffer[..read])
            .map_err(|error| ShellError::container_command_failed("the image archive", error.to_string()))?;
        if !observe(read as u64) {
            return Err(ShellError::container_command_failed(
                "the image archive",
                "the transfer was cancelled",
            ));
        }
    }
}

fn mebibytes(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

/// The container manager.
pub struct ContainerManager {
    state: RwLock<State>,
}

#[derive(Default)]
struct State {
    connections: HashMap<String, Arc<Provider>>,
    /// Ids that were handed out and then closed. A stream, a terminal, or an interface tab holds an
    /// id; if the manager reused one, an old holder would silently address a new runtime.
    retired: HashSet<String>,
}

impl ContainerManager {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State::default()),
        }
    }

    /// A fresh opaque id. The manager never reuses one, so a connection can be added by the id the
    /// caller stored in its own tab.
    pub fn new_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn add(&self, id: impl Into<String>, provider: Provider) -> Result<(), ShellError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ShellError::app_internal("a container connection needs an id"));
        }
        let mut state = self.write();
        if state.retired.contains(&id) {
            return Err(ShellError::app_internal("container connection ids are never reused"));
        }
        state.connections.insert(id, Arc::new(provider));
        Ok(())
    }

    /// Builds a provider and registers it in one step, which is what a connect command does.
    pub fn connect(
        &self,
        id: impl Into<String>,
        runtime: ContainerRuntime,
        namespace: &str,
        runner: Runner,
    ) -> Result<(), ShellError> {
        let provider = Provider::new(runtime, namespace, runner)?;
        self.add(id, provider)
    }

    pub fn remove(&self, id: &str) {
        let mut state = self.write();
        if state.connections.remove(id).is_some() {
            state.retired.insert(id.to_owned());
        }
    }

    pub fn provider(&self, id: &str) -> Result<Arc<Provider>, ShellError> {
        let state = self.read();
        state
            .connections
            .get(id)
            .cloned()
            .ok_or_else(|| ShellError::container_not_connected(id))
    }

    /// Switches the namespace of a live connection. Streams and terminals that already started keep
    /// running: they are a process, not a setting.
    pub fn set_namespace(&self, id: &str, namespace: &str) -> Result<(), ShellError> {
        let mut state = self.write();
        let current = state
            .connections
            .get(id)
            .cloned()
            .ok_or_else(|| ShellError::container_not_connected(id))?;
        let replacement = current.with_namespace(namespace)?;
        state.connections.insert(id.to_owned(), Arc::new(replacement));
        Ok(())
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for ContainerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sevenshell_error::ErrorCode;

    use super::*;

    #[derive(Default)]
    struct RecordingRunner {
        calls: Mutex<Vec<Vec<String>>>,
        inputs: Mutex<Vec<Vec<u8>>>,
        output: String,
        fail: bool,
    }

    impl RecordingRunner {
        fn record(&self, argv: &[String]) {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(argv.to_vec());
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl CommandRunner for RecordingRunner {
        async fn run(&self, argv: &[String]) -> Result<String, ShellError> {
            self.record(argv);
            if self.fail {
                return Err(ShellError::container_unavailable("docker", "the engine is not running"));
            }
            Ok(self.output.clone())
        }

        async fn run_stdin(&self, argv: &[String], input: Vec<u8>) -> Result<String, ShellError> {
            self.record(argv);
            self.inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(input);
            Ok(self.output.clone())
        }

        async fn stream_stdin(&self, argv: &[String], mut input: InputStream) -> Result<String, ShellError> {
            self.record(argv);
            let mut chunks = input.take_chunks();
            let mut bytes = Vec::new();
            while let Some(chunk) = chunks.recv().await {
                bytes.extend_from_slice(&chunk);
            }
            self.inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(bytes);
            // The real runner prefers the source's own failure; a recorder that did not would let
            // a cancelled or unreadable archive look like a successful load.
            if let Some(detail) = input.failure() {
                return Err(ShellError::container_command_failed("the standard input", detail));
            }
            Ok(self.output.clone())
        }

        async fn stream(&self, argv: &[String]) -> Result<LineStream, ShellError> {
            self.record(argv);
            let (_sender, receiver) = tokio::sync::mpsc::channel(1);
            Ok(LineStream::new(receiver, || {}))
        }

        async fn open_pty(&self, argv: &[String], _columns: u32, _rows: u32) -> Result<PtySession, ShellError> {
            self.record(argv);
            Err(ShellError::app_unsupported("no terminal in this test"))
        }
    }

    fn build_provider(runtime: ContainerRuntime, namespace: &str, recorder: &Arc<RecordingRunner>) -> Provider {
        Provider::from_parts(runtime, namespace, Arc::clone(recorder) as Arc<dyn CommandRunner>)
            .expect("the namespace should be valid")
    }

    /// A file in the temporary directory whose name is unique to this process and to `label`.
    fn temporary_file(label: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("7shell-{label}-{}.tar", std::process::id()));
        std::fs::write(&path, contents).expect("a temporary archive");
        path
    }

    #[tokio::test]
    async fn a_rejected_container_id_never_reaches_the_runner() {
        let recorder = Arc::new(RecordingRunner::default());
        let provider = build_provider(ContainerRuntime::Docker, "", &recorder);
        for id in ["--help", "-x", "", "   ", "a\nb"] {
            assert!(provider.inspect(id).await.is_err(), "inspect accepted {id:?}");
            assert!(provider.logs(id, 10, false).await.is_err(), "logs accepted {id:?}");
            assert!(provider.action(id, "start").await.is_err(), "action accepted {id:?}");
            assert!(provider.rename(id, "web").await.is_err(), "rename accepted {id:?}");
            assert!(provider.exec_argv(id, "sh").is_err(), "exec accepted {id:?}");
            assert!(
                provider.open_exec(id, "sh", 80, 24).await.is_err(),
                "exec opened {id:?}"
            );
        }
        assert!(
            recorder.calls().is_empty(),
            "a rejected identifier still reached the runner: {:?}",
            recorder.calls()
        );
    }

    #[tokio::test]
    async fn a_rejected_image_reference_never_reaches_the_runner() {
        let recorder = Arc::new(RecordingRunner::default());
        let provider = build_provider(ContainerRuntime::Docker, "", &recorder);
        let request = ContainerImageTransfer {
            image: "--privileged".to_owned(),
            platform: String::new(),
            all_tags: false,
            insecure: false,
        };
        assert!(provider.image_pull(&request).await.is_err());
        assert!(provider.image_push(&request).await.is_err());
        assert!(provider.image_inspect(&request.image).await.is_err());
        assert!(provider.image_history(&request.image).await.is_err());
        assert!(provider.image_remove(&request.image).await.is_err());
        assert!(provider.image_tag(&request.image, "repo:tag").await.is_err());
        assert!(recorder.calls().is_empty(), "{:?}", recorder.calls());
    }

    #[tokio::test]
    async fn a_rejected_namespace_never_reaches_the_runner_and_keeps_the_old_one() {
        let recorder = Arc::new(RecordingRunner::default());
        let manager = ContainerManager::new();
        manager
            .add("tab", build_provider(ContainerRuntime::Nerdctl, "default", &recorder))
            .unwrap();
        let error = manager.set_namespace("tab", "--namespace").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        assert_eq!(manager.provider("tab").unwrap().namespace(), "default");
        assert!(recorder.calls().is_empty());
    }

    #[tokio::test]
    async fn the_manager_answers_for_an_unknown_id_with_the_not_connected_code() {
        let manager = ContainerManager::new();
        let error = manager.provider("missing").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContainerNotConnected);
        assert_eq!(
            manager.set_namespace("missing", "default").unwrap_err().code(),
            ErrorCode::ContainerNotConnected
        );
    }

    #[tokio::test]
    async fn a_removed_id_is_never_handed_out_again() {
        let recorder = Arc::new(RecordingRunner::default());
        let manager = ContainerManager::new();
        let id = manager.new_id();
        manager
            .add(id.clone(), build_provider(ContainerRuntime::Docker, "", &recorder))
            .unwrap();
        manager.remove(&id);
        assert_eq!(
            manager.provider(&id).unwrap_err().code(),
            ErrorCode::ContainerNotConnected
        );
        assert!(
            manager
                .add(id.clone(), build_provider(ContainerRuntime::Docker, "", &recorder))
                .is_err(),
            "the manager reused an id a stream could still be holding"
        );
    }

    #[tokio::test]
    async fn a_new_connection_starts_in_the_namespace_it_was_given() {
        let recorder = Arc::new(RecordingRunner::default());
        let manager = ContainerManager::new();
        manager
            .connect("tab", ContainerRuntime::Nerdctl, "", Runner::Local)
            .unwrap();
        assert_eq!(manager.provider("tab").unwrap().namespace(), "default");

        // The recorder proves the flag travels in the argv, before the subcommand.
        let manager = ContainerManager::new();
        manager
            .add("tab", build_provider(ContainerRuntime::Nerdctl, "", &recorder))
            .unwrap();
        manager.set_namespace("tab", "k8s.io").unwrap();
        manager.provider("tab").unwrap().list().await.unwrap();
        assert_eq!(
            recorder.calls().last().unwrap(),
            &vec!["nerdctl", "--namespace", "k8s.io", "ps", "-a", "--format", "{{json .}}"]
        );
    }

    #[tokio::test]
    async fn the_list_field_of_a_split_answer_reaches_the_wire_type() {
        let recorder = Arc::new(RecordingRunner {
            output: "abc123|nginx:1.27|web|running|Up 1 minute|0.0.0.0:80->80/tcp|2026-08-24 09:15:00 +0800 CST\n"
                .to_owned(),
            ..RecordingRunner::default()
        });
        let rows = build_provider(ContainerRuntime::Docker, "", &recorder)
            .list()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "web");
        assert_eq!(rows[0].state, "running");
    }

    #[tokio::test]
    async fn a_login_password_goes_in_on_standard_input_and_not_in_the_argv() {
        let recorder = Arc::new(RecordingRunner::default());
        let provider = build_provider(ContainerRuntime::Docker, "", &recorder);
        let login = ContainerRegistryLogin {
            registry: "registry.example.com".to_owned(),
            username: "alice".to_owned(),
            secret: sevenshell_types::SecretInput {
                value: "s3cr3t-value".to_owned(),
            },
            insecure: false,
        };
        provider.registry_login(&login).await.unwrap();
        let calls = recorder.calls();
        assert!(
            !calls[0].iter().any(|argument| argument.contains("s3cr3t")),
            "the password reached the argv: {:?}",
            calls[0]
        );
        assert_eq!(
            calls[0],
            vec![
                "docker",
                "login",
                "--password-stdin",
                "-u",
                "alice",
                "registry.example.com"
            ]
        );
        let inputs = recorder
            .inputs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(inputs, vec![b"s3cr3t-value\n".to_vec()]);
    }

    #[tokio::test]
    async fn an_image_archive_reaches_docker_and_podman_and_a_commit_stays_docker_only() {
        let recorder = Arc::new(RecordingRunner::default());
        let commit = ContainerCommitRequest {
            container_id: "abc123".to_owned(),
            image: "team/web:built".to_owned(),
            author: String::new(),
            message: String::new(),
        };
        assert_eq!(
            build_provider(ContainerRuntime::Podman, "", &recorder)
                .image_commit(&commit)
                .await
                .unwrap_err()
                .code(),
            ErrorCode::AppUnsupported
        );

        // Podman reads a docker-save archive on standard input just as Docker does, so the button
        // the interface draws exists on both; the other two runtimes refuse before any process.
        build_provider(ContainerRuntime::Podman, "", &recorder)
            .image_load(vec![1, 2, 3])
            .await
            .expect("podman should take an archive");
        assert_eq!(recorder.calls(), vec![vec!["podman", "load"]]);

        for runtime in [ContainerRuntime::Nerdctl, ContainerRuntime::Wslc] {
            assert_eq!(
                build_provider(runtime, "", &recorder)
                    .image_load(vec![1, 2, 3])
                    .await
                    .unwrap_err()
                    .code(),
                ErrorCode::AppUnsupported,
                "{runtime:?} accepted an archive it cannot load"
            );
        }
        assert_eq!(recorder.calls(), vec![vec!["podman", "load"]]);
    }

    #[tokio::test]
    async fn namespaces_are_only_asked_of_the_runtime_that_has_them() {
        let recorder = Arc::new(RecordingRunner::default());
        let provider = build_provider(ContainerRuntime::Docker, "", &recorder);
        assert_eq!(
            provider.namespaces().await.unwrap_err().code(),
            ErrorCode::AppUnsupported
        );
        assert!(recorder.calls().is_empty());

        let recorder = Arc::new(RecordingRunner {
            output: "NAME CONTAINERS IMAGES\n\ndefault 1 2\nk8s.io 3 4\n".to_owned(),
            ..RecordingRunner::default()
        });
        let provider = build_provider(ContainerRuntime::Nerdctl, "", &recorder);
        assert_eq!(provider.namespaces().await.unwrap(), vec!["default", "k8s.io"]);
        assert_eq!(recorder.calls(), vec![vec!["nerdctl", "namespace", "ls"]]);
    }

    #[tokio::test]
    async fn runtime_info_survives_one_half_failing_and_reports_both_failing() {
        let recorder = Arc::new(RecordingRunner {
            output: "{\"ServerVersion\":\"28.4.1\"}".to_owned(),
            ..RecordingRunner::default()
        });
        let info = build_provider(ContainerRuntime::Docker, "", &recorder)
            .runtime_info()
            .await
            .unwrap();
        assert_eq!(info.server_version, "28.4.1");
        assert_eq!(recorder.calls().len(), 2, "info and version should run together");

        let recorder = Arc::new(RecordingRunner {
            fail: true,
            ..RecordingRunner::default()
        });
        let error = build_provider(ContainerRuntime::Docker, "", &recorder)
            .runtime_info()
            .await
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContainerUnavailable);
    }

    #[tokio::test]
    async fn a_stream_hands_back_a_handle_that_can_be_stopped() {
        let recorder = Arc::new(RecordingRunner::default());
        let mut stream = build_provider(ContainerRuntime::Docker, "", &recorder)
            .logs("abc123", 100, true)
            .await
            .unwrap();
        assert_eq!(
            recorder.calls().last().unwrap(),
            &vec!["docker", "logs", "--tail", "100", "--timestamps", "-f", "abc123"],
            "a log stream follows, and the follow flag is what makes it a stream"
        );
        stream.stop();
        // The recorder's channel has no producer, so the end is immediate; the point is that a
        // stream can be stopped twice without panicking.
        stream.stop();
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn loading_an_archive_sends_the_bytes_on_standard_input() {
        let recorder = Arc::new(RecordingRunner {
            output: "Loaded image: team/web:built\n".to_owned(),
            ..RecordingRunner::default()
        });
        let provider = build_provider(ContainerRuntime::Docker, "", &recorder);
        let loaded = provider.image_load(b"archive-bytes".to_vec()).await.unwrap();
        assert_eq!(loaded, "Loaded image: team/web:built");
        assert_eq!(recorder.calls(), vec![vec!["docker", "load"]]);
        let inputs = recorder
            .inputs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(inputs, vec![b"archive-bytes".to_vec()]);
    }

    #[tokio::test]
    async fn an_archive_named_by_a_file_reaches_the_runtime_as_standard_input() {
        let recorder = Arc::new(RecordingRunner {
            output: "Loaded image: team/web:built\n".to_owned(),
            ..RecordingRunner::default()
        });
        let path = temporary_file("from-file", b"an archive on disk");
        let loaded = build_provider(ContainerRuntime::Docker, "", &recorder)
            .image_load_file(&path)
            .await
            .expect("a regular file under the ceiling is an archive");
        let _ = std::fs::remove_file(&path);

        assert_eq!(loaded, "Loaded image: team/web:built");
        assert_eq!(recorder.calls(), vec![vec!["docker", "load"]]);
        let inputs = recorder
            .inputs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(
            inputs,
            vec![b"an archive on disk".to_vec()],
            "the file's bytes, not its name, must be what the runtime reads"
        );
    }

    #[tokio::test]
    async fn an_archive_load_reports_bytes_and_stops_when_the_observer_does() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let contents = vec![b'x'; 128 * 1024 + 7];
        let path = temporary_file("load-progress", &contents);
        let recorder = Arc::new(RecordingRunner::default());
        let provider = build_provider(ContainerRuntime::Docker, "", &recorder);

        let seen = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&seen);
        let loaded = provider
            .image_load_file_with(&path, move |bytes| {
                counted.fetch_add(bytes, Ordering::SeqCst);
                true
            })
            .await
            .expect("the archive is read");
        assert_eq!(loaded, "");
        assert_eq!(seen.load(Ordering::SeqCst), contents.len() as u64);

        let stopped = provider
            .image_load_file_with(&path, |_| false)
            .await
            .expect_err("the observer stopped it");
        assert_eq!(stopped.code(), ErrorCode::ContainerCommandFailed);
        assert!(
            stopped.detail().unwrap_or_default().contains("cancelled"),
            "the refusal says why it stopped: {stopped:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_archive_over_the_ceiling_is_refused_with_its_size() {
        let error = ensure_archive_fits(1025 * 1024, 1024 * 1024).expect_err("the file is larger than the ceiling");
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        let detail = error.detail().unwrap_or_default();
        assert!(
            detail.contains("the archive is 2 MiB"),
            "the size that was refused is in the message: {detail}"
        );
        assert!(
            detail.contains("at most 1 MiB"),
            "the ceiling this build does allow is in the message: {detail}"
        );
        ensure_archive_fits(1024 * 1024, 1024 * 1024).expect("a file at the ceiling is allowed");
    }

    #[tokio::test]
    async fn a_directory_is_not_an_image_archive() {
        let directory = std::env::temp_dir().join(format!("7shell-archive-dir-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let provider = build_provider(ContainerRuntime::Docker, "", &Arc::new(RecordingRunner::default()));
        let error = provider
            .image_load_file(&directory)
            .await
            .expect_err("a directory has no archive in it");
        let _ = std::fs::remove_dir(&directory);
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        assert!(error.detail().unwrap_or_default().contains("regular file"));
    }

    #[tokio::test]
    async fn wslc_history_asks_for_the_image_inspect_it_does_have() {
        let recorder = Arc::new(RecordingRunner {
            output: "[{\"Id\":\"abc\",\"RootFS\":{\"Layers\":[\"sha256:aaaa\",\"sha256:bbbb\"]}}]".to_owned(),
            ..RecordingRunner::default()
        });
        let layers = build_provider(ContainerRuntime::Wslc, "", &recorder)
            .image_history("nginx:latest")
            .await
            .unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].id, "sha256:bbbb", "layers come out newest first");
        assert_eq!(
            recorder.calls(),
            vec![vec!["wslc", "image", "inspect", "--format", "json", "nginx:latest"]]
        );
    }

    #[tokio::test]
    async fn a_connection_with_an_impossible_namespace_is_refused_before_it_is_registered() {
        let manager = ContainerManager::new();
        let error = manager
            .connect("tab", ContainerRuntime::Nerdctl, "--namespace", Runner::Local)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        assert_eq!(
            manager.provider("tab").unwrap_err().code(),
            ErrorCode::ContainerNotConnected,
            "a refused connection must not be left registered"
        );
    }

    #[tokio::test]
    async fn a_provider_hands_out_the_runner_it_was_built_from_and_only_that() {
        let manager = ContainerManager::new();
        manager
            .connect("tab", ContainerRuntime::Nerdctl, "k8s.io", Runner::Local)
            .expect("a local connection is only a name until something runs");
        let provider = manager.provider("tab").unwrap();
        assert!(
            matches!(
                provider.runner().expect("a connected provider has a runner"),
                Runner::Local
            ),
            "the file browser must run on the connection the tab opened"
        );

        // Changing the namespace rebuilds the provider; the runner has to survive the rebuild, or
        // every file command would go somewhere the tab is not.
        manager.set_namespace("tab", "default").unwrap();
        assert!(matches!(
            manager.provider("tab").unwrap().runner().expect("still a runner"),
            Runner::Local
        ));

        let recorder = Arc::new(RecordingRunner::default());
        let recorder_provider = build_provider(ContainerRuntime::Docker, "", &recorder);
        let Err(error) = recorder_provider.runner() else {
            panic!("a provider built around a recorder has no runner, and must not pretend to");
        };
        assert_eq!(error.code(), ErrorCode::AppUnsupported);
    }

    /// Real Docker Desktop: `docker save` writes an archive, this crate loads it back.
    ///
    /// Ignored by default, and it skips with a sentence rather than failing when the engine is not
    /// reachable. The archive is saved from a tag of this test's own, so the removal that makes the
    /// load provable never touches an image the machine already had.
    ///
    /// ```text
    /// SEVENSHELL_TEST_DOCKER=1 cargo test -p sevenshell-container -- --ignored a_real_engine_loads
    /// ```
    #[tokio::test]
    #[ignore = "needs Docker Desktop and SEVENSHELL_TEST_DOCKER=1"]
    async fn a_real_engine_loads_a_saved_archive_and_lists_the_image_again() {
        if std::env::var("SEVENSHELL_TEST_DOCKER").as_deref() != Ok("1") {
            eprintln!("skipped: set SEVENSHELL_TEST_DOCKER=1 to run the live archive test");
            return;
        }
        if !docker(&["version"]).is_some_and(|output| output.status.success()) {
            eprintln!("skipped: the Docker engine is not reachable from this process");
            return;
        }
        // The archive is made from a small image every engine can fetch. A pull that fails is a
        // machine without a registry, not a defect in this crate.
        if !docker(&["image", "inspect", "busybox:1.36"]).is_some_and(|output| output.status.success())
            && !docker(&["pull", "busybox:1.36"]).is_some_and(|output| output.status.success())
        {
            eprintln!("skipped: busybox:1.36 is neither present nor pullable here");
            return;
        }

        let tag = format!("7shell-live-load:{}", std::process::id());
        let saved = temporary_file("live-load", b"");
        let archive = saved.to_string_lossy().into_owned();
        assert!(
            docker(&["tag", "busybox:1.36", &tag]).is_some_and(|output| output.status.success()),
            "the image could not be given a tag of its own"
        );
        assert!(
            docker(&["save", "-o", &archive, &tag]).is_some_and(|output| output.status.success()),
            "the archive could not be written to {archive}"
        );
        // Without this the assertion below would pass on an image that never left.
        assert!(
            docker(&["rmi", &tag]).is_some_and(|output| output.status.success()),
            "the tag could not be removed after saving"
        );

        let provider = Provider::new(ContainerRuntime::Docker, "", Runner::Local).expect("a provider");
        let before = provider.images().await.expect("the image list should answer");
        assert!(
            !before.iter().any(|row| row.repository == "7shell-live-load"),
            "the tag survived its own removal, so the load below proves nothing"
        );

        let loaded = provider
            .image_load_file(&saved)
            .await
            .expect("a saved archive should load");
        assert!(loaded.contains("7shell-live-load"), "the runtime said: {loaded}");

        let after = provider.images().await.expect("the image list should answer");
        assert!(
            after.iter().any(|row| row.repository == "7shell-live-load"),
            "the loaded image is not in the list: {after:?}"
        );

        assert!(
            provider.image_remove(&tag).await.is_ok(),
            "the tag this test made could not be removed again"
        );
        let _ = std::fs::remove_file(&saved);
    }

    fn docker(arguments: &[&str]) -> Option<std::process::Output> {
        std::process::Command::new("docker").args(arguments).output().ok()
    }

    /// A compressed archive, made without assuming this machine has a compressor of its own: the
    /// engine it already needs can run one.
    fn gzip_in_container(source: &Path, target: &Path) -> bool {
        use std::process::Stdio;
        let Ok(mut child) = std::process::Command::new("docker")
            .args(["run", "--rm", "-i", "busybox:1.36", "gzip", "-c"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let Ok(mut input) = std::fs::File::open(source) else {
            return false;
        };
        let Some(mut stdin) = child.stdin.take() else {
            return false;
        };
        // The archive is larger than a pipe buffer, so it has to be written while the output is
        // read; one after the other would deadlock on the first full pipe.
        let writing = std::thread::spawn(move || {
            let _ = std::io::copy(&mut input, &mut stdin);
        });
        let Ok(mut output) = std::fs::File::create(target) else {
            return false;
        };
        // stderr is discarded above, but a compressor that fails writes its reason there; the exit
        // status is what this reports.
        let copied = child
            .stdout
            .take()
            .is_some_and(|mut stdout| std::io::copy(&mut stdout, &mut output).is_ok());
        let status = child.wait().map(|status| status.success()).unwrap_or(false);
        let _ = writing.join();
        copied && status
    }

    /// Real Docker Desktop: a gzipped `docker save` archive streams into `docker load`.
    ///
    /// The archive is compressed by a container because this test may run on a machine without a
    /// gzip binary, which is the same reason the runtime is reached through its CLI everywhere else.
    /// Nothing about the archive is read here: the file is streamed into the runtime as it is.
    ///
    /// ```text
    /// SEVENSHELL_TEST_DOCKER=1 cargo test -p sevenshell-container -- --ignored a_real_engine_loads_a_gzipped
    /// ```
    #[tokio::test]
    #[ignore = "needs Docker Desktop and SEVENSHELL_TEST_DOCKER=1"]
    async fn a_real_engine_loads_a_gzipped_archive_as_standard_input() {
        if std::env::var("SEVENSHELL_TEST_DOCKER").as_deref() != Ok("1") {
            eprintln!("skipped: set SEVENSHELL_TEST_DOCKER=1 to run the live archive test");
            return;
        }
        if !docker(&["version"]).is_some_and(|output| output.status.success()) {
            eprintln!("skipped: the Docker engine is not reachable from this process");
            return;
        }
        if !docker(&["image", "inspect", "busybox:1.36"]).is_some_and(|output| output.status.success())
            && !docker(&["pull", "busybox:1.36"]).is_some_and(|output| output.status.success())
        {
            eprintln!("skipped: busybox:1.36 is neither present nor pullable here");
            return;
        }

        let tag = format!("7shell-live-gz:{}", std::process::id());
        let saved = temporary_file("live-gz-tar", b"");
        let compressed = temporary_file("live-gz", b"");
        let archive = saved.to_string_lossy().into_owned();
        assert!(
            docker(&["tag", "busybox:1.36", &tag]).is_some_and(|output| output.status.success()),
            "the image could not be given a tag of its own"
        );
        assert!(
            docker(&["save", "-o", &archive, &tag]).is_some_and(|output| output.status.success()),
            "the archive could not be written to {archive}"
        );
        assert!(
            docker(&["rmi", &tag]).is_some_and(|output| output.status.success()),
            "the tag could not be removed after saving"
        );
        assert!(
            gzip_in_container(&saved, &compressed),
            "the saved archive could not be compressed"
        );
        let magic = std::fs::read(&compressed).expect("the compressed archive");
        assert_eq!(&magic[..2], &[0x1f, 0x8b], "the fixture is not a gzip stream");

        let provider = Provider::new(ContainerRuntime::Docker, "", Runner::Local).expect("a provider");
        let before = provider.images().await.expect("the image list should answer");
        assert!(
            !before.iter().any(|row| row.repository == "7shell-live-gz"),
            "the tag survived its own removal, so the load below proves nothing"
        );

        let loaded = provider
            .image_load_file(&compressed)
            .await
            .expect("a gzipped archive should stream into the engine");
        assert!(loaded.contains("7shell-live-gz"), "the runtime said: {loaded}");

        let after = provider.images().await.expect("the image list should answer");
        assert!(
            after.iter().any(|row| row.repository == "7shell-live-gz"),
            "the loaded image is not in the list: {after:?}"
        );

        assert!(
            provider.image_remove(&tag).await.is_ok(),
            "the tag this test made could not be removed again"
        );
        let _ = std::fs::remove_file(&saved);
        let _ = std::fs::remove_file(&compressed);
    }
}

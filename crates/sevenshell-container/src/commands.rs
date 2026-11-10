//! The argument vector each runtime's command line takes.
//!
//! Nothing here is a shell string and nothing here starts a process: a builder returns an argv for
//! the runner, or it refuses the input. Refusal happens here, before any spawn, because a container
//! name that begins with `-` would otherwise be read as an option by the runtime's own CLI, and a
//! name is whatever the far side of a connection happens to be called.
//!
//! The differences between Docker, Podman, nerdctl and WSLC live in this file and nowhere else.
//! They are not stylistic: nerdctl takes its namespace as a global flag *before* the subcommand,
//! WSLC prints `--format json` where Docker wants a Go template, WSLC has no `history` subcommand,
//! and the two runtimes that support an insecure registry express it as two different flags in
//! two different positions.

use sevenshell_error::ShellError;
use sevenshell_types::{
    ContainerCommitRequest, ContainerCreateRequest, ContainerImageTransfer, ContainerPortMapping,
    ContainerRegistryLogin, ContainerRuntime,
};

/// The longest identifier (container id or name, tag, username) this build will pass on.
const MAX_IDENTIFIER_CHARS: usize = 255;
/// Docker's own limit on a reference; anything longer is refused by the CLI anyway.
const MAX_IMAGE_REFERENCE_CHARS: usize = 255;
const MAX_PLATFORM_CHARS: usize = 64;
const MAX_AUTHOR_CHARS: usize = 256;
const MAX_MESSAGE_CHARS: usize = 4096;

/// The template `ps` uses. `{{json .}}` is deliberately avoided: it includes `.Size`, which makes
/// the CLI ask the daemon for per-container filesystem statistics — tens of seconds on a busy host.
/// Listing the fields we draw is as fast as the table output.
const PS_FORMAT_DOCKER: &str = "{{.ID}}|{{.Image}}|{{.Names}}|{{.State}}|{{.Status}}|{{.Ports}}|{{.CreatedAt}}";
const JSON_FORMAT: &str = "{{json .}}";
/// WSLC's `--format` takes a fixed vocabulary, not a template.
const WSLC_FORMAT: &str = "json";

/// Which way an image is moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageTransfer {
    Pull,
    Push,
}

impl ImageTransfer {
    fn subcommand(self) -> &'static str {
        match self {
            Self::Pull => "pull",
            Self::Push => "push",
        }
    }
}

/// The runtime's name plus the flags that must precede every subcommand.
fn base(runtime: ContainerRuntime, namespace: &str) -> Vec<String> {
    let mut argv = vec![runtime.program().to_owned()];
    // nerdctl is the only runtime with namespaces, and its flag is global: after the subcommand it
    // is an error instead of an option.
    if runtime == ContainerRuntime::Nerdctl && !namespace.is_empty() {
        argv.push("--namespace".to_owned());
        argv.push(namespace.to_owned());
    }
    argv
}

fn format_flag(runtime: ContainerRuntime) -> Vec<String> {
    match runtime {
        ContainerRuntime::Docker => vec!["--format".to_owned(), PS_FORMAT_DOCKER.to_owned()],
        ContainerRuntime::Wslc => vec!["--format".to_owned(), WSLC_FORMAT.to_owned()],
        ContainerRuntime::Podman | ContainerRuntime::Nerdctl => {
            vec!["--format".to_owned(), JSON_FORMAT.to_owned()]
        }
    }
}

fn json_format_flag(runtime: ContainerRuntime) -> Vec<String> {
    match runtime {
        ContainerRuntime::Wslc => vec!["--format".to_owned(), WSLC_FORMAT.to_owned()],
        _ => vec!["--format".to_owned(), JSON_FORMAT.to_owned()],
    }
}

pub(crate) fn list(runtime: ContainerRuntime, namespace: &str) -> Vec<String> {
    let mut argv = base(runtime, namespace);
    argv.push("ps".to_owned());
    argv.push("-a".to_owned());
    argv.extend(format_flag(runtime));
    argv
}

pub(crate) fn inspect(runtime: ContainerRuntime, namespace: &str, id: &str) -> Result<Vec<String>, ShellError> {
    let id = identifier("the inspect request", "container id", id)?;
    let mut argv = base(runtime, namespace);
    argv.push("inspect".to_owned());
    argv.push(id.to_owned());
    Ok(argv)
}

pub(crate) fn logs(
    runtime: ContainerRuntime,
    namespace: &str,
    id: &str,
    tail: u32,
    follow: bool,
    timestamps: bool,
) -> Result<Vec<String>, ShellError> {
    let id = identifier("the logs request", "container id", id)?;
    let mut argv = base(runtime, namespace);
    argv.push("logs".to_owned());
    if runtime == ContainerRuntime::Wslc {
        argv.push("-n".to_owned());
        argv.push(tail.to_string());
        if follow {
            argv.push("-f".to_owned());
        }
        if timestamps {
            argv.push("-t".to_owned());
        }
    } else {
        argv.push("--tail".to_owned());
        argv.push(tail.to_string());
        if timestamps {
            argv.push("--timestamps".to_owned());
        }
        if follow {
            argv.push("-f".to_owned());
        }
    }
    argv.push(id.to_owned());
    Ok(argv)
}

pub(crate) fn stats(runtime: ContainerRuntime, namespace: &str) -> Vec<String> {
    let mut argv = base(runtime, namespace);
    argv.push("stats".to_owned());
    match runtime {
        ContainerRuntime::Wslc => argv.extend(json_format_flag(runtime)),
        _ => {
            argv.push("--no-stream".to_owned());
            argv.extend(json_format_flag(runtime));
        }
    }
    argv
}

/// The action verb as the runtime spells it: the interface says `remove`, the CLI says `rm`.
fn action_subcommand(runtime: ContainerRuntime, action: &str) -> Result<&'static str, ShellError> {
    let action = action.trim().to_ascii_lowercase();
    let (subcommand, supported) = match action.as_str() {
        "start" => ("start", true),
        "stop" => ("stop", true),
        "restart" => ("restart", runtime != ContainerRuntime::Wslc),
        "pause" => ("pause", runtime != ContainerRuntime::Wslc),
        "unpause" => ("unpause", runtime != ContainerRuntime::Wslc),
        "remove" | "rm" => ("rm", true),
        // `kill` is not offered. Stopping is the coarse action a person chooses from a list;
        // killing is a signal choice that belongs in the container's own terminal.
        _ => {
            return Err(ShellError::container_command_failed(
                "the action request",
                format!("unknown container action: {action}"),
            ));
        }
    };
    if !supported {
        return Err(ShellError::app_unsupported("this action on the WSLC runtime"));
    }
    Ok(subcommand)
}

pub(crate) fn action(
    runtime: ContainerRuntime,
    namespace: &str,
    action: &str,
    id: &str,
) -> Result<Vec<String>, ShellError> {
    let subcommand = action_subcommand(runtime, action)?;
    let id = identifier("the action request", "container id", id)?;
    let mut argv = base(runtime, namespace);
    argv.push(subcommand.to_owned());
    argv.push(id.to_owned());
    Ok(argv)
}

pub(crate) fn rename(
    runtime: ContainerRuntime,
    namespace: &str,
    id: &str,
    name: &str,
) -> Result<Vec<String>, ShellError> {
    if runtime == ContainerRuntime::Wslc {
        return Err(ShellError::app_unsupported("renaming a container on the WSLC runtime"));
    }
    let id = identifier("the rename request", "container id", id)?;
    let name = identifier("the rename request", "container name", name)?;
    let mut argv = base(runtime, namespace);
    argv.push("rename".to_owned());
    argv.push(id.to_owned());
    argv.push(name.to_owned());
    Ok(argv)
}

pub(crate) fn images(runtime: ContainerRuntime, namespace: &str) -> Vec<String> {
    let mut argv = base(runtime, namespace);
    argv.push("images".to_owned());
    argv.extend(json_format_flag(runtime));
    argv
}

pub(crate) fn image_inspect(
    runtime: ContainerRuntime,
    namespace: &str,
    image: &str,
) -> Result<Vec<String>, ShellError> {
    let image = image_reference("the image inspect request", image)?;
    let mut argv = base(runtime, namespace);
    argv.push("inspect".to_owned());
    argv.push(image.to_owned());
    Ok(argv)
}

/// WSLC's `--format` only accepts the word `json`, and its `image inspect` is the shape that
/// carries `RootFS.Layers`; the explicit `image` group avoids a container of the same name.
pub(crate) fn wslc_image_inspect(namespace: &str, image: &str) -> Result<Vec<String>, ShellError> {
    let image = image_reference("the image history request", image)?;
    let mut argv = base(ContainerRuntime::Wslc, namespace);
    argv.push("image".to_owned());
    argv.push("inspect".to_owned());
    argv.push("--format".to_owned());
    argv.push(WSLC_FORMAT.to_owned());
    argv.push(image.to_owned());
    Ok(argv)
}

pub(crate) fn image_history(
    runtime: ContainerRuntime,
    namespace: &str,
    image: &str,
) -> Result<Vec<String>, ShellError> {
    if runtime == ContainerRuntime::Wslc {
        return Err(ShellError::app_unsupported("image history on the WSLC runtime"));
    }
    let image = image_reference("the image history request", image)?;
    let mut argv = base(runtime, namespace);
    argv.push("history".to_owned());
    // Without it every layer's `CreatedBy` is cut at the terminal width.
    argv.push("--no-trunc".to_owned());
    argv.extend(json_format_flag(runtime));
    argv.push(image.to_owned());
    Ok(argv)
}

pub(crate) fn transfer(
    runtime: ContainerRuntime,
    namespace: &str,
    direction: ImageTransfer,
    request: &ContainerImageTransfer,
) -> Result<Vec<String>, ShellError> {
    let image = image_reference("the image transfer request", &request.image)?;
    let platform = match request.platform.trim() {
        "" => None,
        value => Some(platform_value("the image transfer request", value)?),
    };

    let mut argv = base(runtime, namespace);
    // nerdctl's insecure registry is a global flag whose value is the registry host, and it has to
    // sit before the subcommand. Docker and WSLC have no per-command equivalent; the caller is
    // expected to configure the daemon there.
    if request.insecure
        && runtime == ContainerRuntime::Nerdctl
        && let Some(host) = registry_host(image)
    {
        argv.push("--insecure-registry".to_owned());
        argv.push(host.to_owned());
    }
    argv.push(direction.subcommand().to_owned());
    // WSLC's pull does not recognise `--platform`; passing it is an error, not an ignored flag.
    if let Some(platform) = platform
        && runtime != ContainerRuntime::Wslc
    {
        argv.push("--platform".to_owned());
        argv.push(platform.to_owned());
    }
    if direction == ImageTransfer::Pull && request.all_tags {
        argv.push("--all-tags".to_owned());
    }
    if request.insecure && runtime == ContainerRuntime::Podman {
        argv.push("--tls-verify=false".to_owned());
    }
    argv.push(image.to_owned());
    Ok(argv)
}

pub(crate) fn remove_image(runtime: ContainerRuntime, namespace: &str, image: &str) -> Result<Vec<String>, ShellError> {
    let image = image_reference("the image removal request", image)?;
    let mut argv = base(runtime, namespace);
    argv.push("rmi".to_owned());
    argv.push(image.to_owned());
    Ok(argv)
}

pub(crate) fn image_prune(runtime: ContainerRuntime, namespace: &str) -> Vec<String> {
    let mut argv = base(runtime, namespace);
    argv.push("image".to_owned());
    argv.push("prune".to_owned());
    // Without `-f` the CLI opens a confirmation prompt on a channel nobody is typing into.
    argv.push("-f".to_owned());
    argv
}

pub(crate) fn tag(
    runtime: ContainerRuntime,
    namespace: &str,
    image: &str,
    repository_tag: &str,
) -> Result<Vec<String>, ShellError> {
    let image = image_reference("the tag request", image)?;
    let repository_tag = image_reference("the tag request", repository_tag)?;
    let mut argv = base(runtime, namespace);
    argv.push("tag".to_owned());
    argv.push(image.to_owned());
    argv.push(repository_tag.to_owned());
    Ok(argv)
}

/// `load` reads the archive from standard input; the bytes never appear in the argv.
pub(crate) fn load(runtime: ContainerRuntime, namespace: &str) -> Vec<String> {
    let mut argv = base(runtime, namespace);
    argv.push("load".to_owned());
    argv
}

pub(crate) fn commit(
    runtime: ContainerRuntime,
    namespace: &str,
    request: &ContainerCommitRequest,
) -> Result<Vec<String>, ShellError> {
    if runtime != ContainerRuntime::Docker {
        return Err(ShellError::app_unsupported(
            "image archives and commits on this runtime",
        ));
    }
    let id = identifier("the commit request", "container id", &request.container_id)?;
    let image = image_reference("the commit request", &request.image)?;
    let author = free_text("the commit request", "author", &request.author, MAX_AUTHOR_CHARS)?;
    let message = free_text("the commit request", "message", &request.message, MAX_MESSAGE_CHARS)?;

    let mut argv = base(runtime, namespace);
    argv.push("commit".to_owned());
    if let Some(author) = author {
        argv.push("--author".to_owned());
        argv.push(author.to_owned());
    }
    if let Some(message) = message {
        argv.push("--message".to_owned());
        argv.push(message.to_owned());
    }
    argv.push(id.to_owned());
    argv.push(image.to_owned());
    Ok(argv)
}

/// The password never appears here: the caller feeds it to `--password-stdin`.
pub(crate) fn login(runtime: ContainerRuntime, login: &ContainerRegistryLogin) -> Result<Vec<String>, ShellError> {
    let username = identifier("the login request", "username", &login.username)?;
    let registry = match login.registry.trim() {
        "" => None,
        value => Some(registry_address("the login request", value)?),
    };

    let mut argv = vec![runtime.program().to_owned()];
    if login.insecure
        && runtime == ContainerRuntime::Nerdctl
        && let Some(registry) = registry
    {
        argv.push("--insecure-registry".to_owned());
        argv.push(registry.to_owned());
    }
    argv.push("login".to_owned());
    argv.push("--password-stdin".to_owned());
    argv.push("-u".to_owned());
    argv.push(username.to_owned());
    if login.insecure && runtime == ContainerRuntime::Podman {
        argv.push("--tls-verify=false".to_owned());
    }
    if let Some(registry) = registry {
        argv.push(registry.to_owned());
    }
    Ok(argv)
}

/// Listing namespaces is itself a nerdctl command and takes no namespace: it is how you find out
/// which ones exist.
pub(crate) fn namespaces(runtime: ContainerRuntime) -> Vec<String> {
    vec![runtime.program().to_owned(), "namespace".to_owned(), "ls".to_owned()]
}

pub(crate) fn info(runtime: ContainerRuntime, namespace: &str) -> Vec<String> {
    let mut argv = base(runtime, namespace);
    argv.push("info".to_owned());
    argv.extend(json_format_flag(runtime));
    argv
}

pub(crate) fn version(runtime: ContainerRuntime, namespace: &str) -> Vec<String> {
    let mut argv = base(runtime, namespace);
    argv.push("version".to_owned());
    argv.extend(json_format_flag(runtime));
    argv
}

pub(crate) fn create(
    runtime: ContainerRuntime,
    namespace: &str,
    request: &ContainerCreateRequest,
) -> Result<Vec<String>, ShellError> {
    if runtime == ContainerRuntime::Wslc && (request.interactive || request.tty) {
        return Err(ShellError::app_unsupported(
            "an interactive container on the WSLC runtime",
        ));
    }
    let image = image_reference("the create request", &request.image)?;
    let name = match request.name.trim() {
        "" => None,
        value => Some(identifier("the create request", "container name", value)?),
    };

    let mut argv = base(runtime, namespace);
    argv.push("run".to_owned());
    argv.push("-d".to_owned());
    if request.interactive {
        argv.push("-i".to_owned());
    }
    if request.tty {
        argv.push("-t".to_owned());
    }
    if let Some(name) = name {
        argv.push("--name".to_owned());
        argv.push(name.to_owned());
    }
    for port in &request.ports {
        argv.push("-p".to_owned());
        argv.push(port_mapping("the create request", port)?);
    }
    for volume in &request.volumes {
        let source = identifier("the create request", "volume source", &volume.source)?;
        let destination = identifier("the create request", "volume destination", &volume.destination)?;
        argv.push("-v".to_owned());
        argv.push(format!("{source}:{destination}"));
    }
    for entry in &request.env {
        argv.push("-e".to_owned());
        argv.push(environment_entry("the create request", entry)?);
    }
    if runtime != ContainerRuntime::Wslc
        && let Some(policy) = restart_policy("the create request", &request.restart_policy)?
    {
        argv.push("--restart".to_owned());
        argv.push(policy.to_owned());
    }
    argv.push(image.to_owned());
    argv.extend(split_command_line("the create request", &request.command)?);
    Ok(argv)
}

pub(crate) fn exec(
    runtime: ContainerRuntime,
    namespace: &str,
    id: &str,
    shell: &str,
) -> Result<Vec<String>, ShellError> {
    let id = identifier("the exec request", "container id", id)?;
    let shell = match shell.trim() {
        "" => "sh",
        value => identifier("the exec request", "shell", value)?,
    };
    let mut argv = base(runtime, namespace);
    argv.push("exec".to_owned());
    // `-i` keeps stdin open, `-t` allocates a terminal; the terminal this crate opens needs both.
    argv.push("-it".to_owned());
    argv.push(id.to_owned());
    argv.push(shell.to_owned());
    Ok(argv)
}

/// `cp` into or out of a container. The file channel itself is a later phase; the argv shape is
/// frozen here so the one runtime that differs stays visible beside the others.
#[allow(dead_code)]
pub(crate) fn copy(
    runtime: ContainerRuntime,
    namespace: &str,
    from: &str,
    to: &str,
) -> Result<Vec<String>, ShellError> {
    let from = identifier("the copy request", "source", from)?;
    let to = identifier("the copy request", "destination", to)?;
    let mut argv = base(runtime, namespace);
    // WSLC has no top-level `cp`; its copy lives under the `container` group.
    if runtime == ContainerRuntime::Wslc {
        argv.push("container".to_owned());
    }
    argv.push("cp".to_owned());
    argv.push(from.to_owned());
    argv.push(to.to_owned());
    Ok(argv)
}

/// Packs a container path to stdout. WSLC's `cp` writes a local file named `-` instead of
/// streaming, so the only way to read a path out of it is `tar` inside the container.
#[allow(dead_code)]
pub(crate) fn archive_export(
    runtime: ContainerRuntime,
    namespace: &str,
    id: &str,
    path: &str,
) -> Result<Vec<String>, ShellError> {
    let id = identifier("the archive request", "container id", id)?;
    let path = path.trim().trim_end_matches('/');
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err(ShellError::container_command_failed(
            "the archive request",
            "container path is empty",
        ));
    }
    let (directory, name) = match path.rsplit_once('/') {
        Some((directory, name)) => (directory, name),
        None => ("/", path),
    };
    let script = format!(
        "tar cf - -C {} {}",
        posix_quote(if directory.is_empty() { "/" } else { directory }),
        posix_quote(name)
    );
    let mut argv = base(runtime, namespace);
    argv.push("exec".to_owned());
    argv.push(id.to_owned());
    argv.push("sh".to_owned());
    argv.push("-c".to_owned());
    argv.push(script);
    Ok(argv)
}

/// Quotes one word for the shell *inside* a container. Only used for scripts the container runs;
/// this machine never sees a shell.
#[allow(dead_code)]
fn posix_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

fn identifier<'a>(operation: &str, kind: &str, value: &'a str) -> Result<&'a str, ShellError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{kind} is empty"),
        ));
    }
    if value.chars().count() > MAX_IDENTIFIER_CHARS {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{kind} is longer than {MAX_IDENTIFIER_CHARS} characters"),
        ));
    }
    if value.starts_with('-') {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{kind} starts with '-', which the runtime would read as an option"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{kind} contains a control character"),
        ));
    }
    Ok(value)
}

fn free_text<'a>(operation: &str, kind: &str, value: &'a str, limit: usize) -> Result<Option<&'a str>, ShellError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > limit {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{kind} is longer than {limit} characters"),
        ));
    }
    if value.contains(['\n', '\r', '\0']) {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{kind} contains a line break"),
        ));
    }
    Ok(Some(value))
}

pub(crate) fn image_reference<'a>(operation: &str, value: &'a str) -> Result<&'a str, ShellError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ShellError::container_command_failed(
            operation,
            "image reference is empty",
        ));
    }
    if !valid_image_reference(value) {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{value} is not a container image reference"),
        ));
    }
    Ok(value)
}

fn platform_value<'a>(operation: &str, value: &'a str) -> Result<&'a str, ShellError> {
    let invalid = value.chars().count() > MAX_PLATFORM_CHARS
        || value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-')));
    if invalid {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{value} is not a platform"),
        ));
    }
    Ok(value)
}

/// Validates a namespace before it becomes a global flag. Empty means "the runtime's default".
pub(crate) fn validate_namespace(namespace: &str) -> Result<(), ShellError> {
    let namespace = namespace.trim();
    if namespace.is_empty() {
        return Ok(());
    }
    let valid = namespace.chars().count() <= MAX_IDENTIFIER_CHARS
        && !namespace.starts_with('-')
        && namespace.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && namespace
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(ShellError::container_command_failed(
            "the namespace request",
            format!("{namespace} is not a namespace name"),
        ))
    }
}

/// The registry part of an image reference, when it has one. Docker treats the first segment as a
/// host only when it looks like one (a dot, a port, or `localhost`); anything else is a repository
/// path, and pulling `library/nginx` from a registry called `library` would be wrong.
fn registry_host(reference: &str) -> Option<&str> {
    let (first, _) = reference.split_once('/')?;
    if first == "localhost" || first.contains('.') || first.contains(':') {
        Some(first)
    } else {
        None
    }
}

fn registry_address<'a>(operation: &str, value: &'a str) -> Result<&'a str, ShellError> {
    let value = value.trim();
    let valid = !value.is_empty()
        && value.chars().count() <= MAX_IDENTIFIER_CHARS
        && !value.starts_with('-')
        && !value.contains('/')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']' | '_'));
    if valid {
        Ok(value)
    } else {
        Err(ShellError::container_command_failed(
            operation,
            format!("{value} is not a registry address"),
        ))
    }
}

fn port_mapping(operation: &str, port: &ContainerPortMapping) -> Result<String, ShellError> {
    let protocol = port.protocol.trim().to_ascii_lowercase();
    if !protocol.is_empty() && !matches!(protocol.as_str(), "tcp" | "udp" | "sctp") {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{} is not a port protocol", port.protocol),
        ));
    }
    let host_ip = port.host_ip.trim();
    if host_ip.chars().any(|c| c.is_control() || c.is_whitespace() || c == ':') || host_ip.starts_with('-') {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{host_ip} is not a host address"),
        ));
    }
    let mut mapping = format!("{}:{}", port.host_port, port.container_port);
    if !host_ip.is_empty() {
        mapping = format!("{host_ip}:{mapping}");
    }
    if !protocol.is_empty() && protocol != "tcp" {
        mapping.push('/');
        mapping.push_str(&protocol);
    }
    Ok(mapping)
}

fn environment_entry(operation: &str, entry: &str) -> Result<String, ShellError> {
    let entry = entry.trim();
    let Some((key, _)) = entry.split_once('=') else {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{entry} is not KEY=VALUE"),
        ));
    };
    if key.is_empty()
        || key.starts_with('-')
        || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'))
    {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{entry} does not start with an environment variable name"),
        ));
    }
    if entry.chars().any(char::is_control) {
        return Err(ShellError::container_command_failed(
            operation,
            "an environment entry contains a control character",
        ));
    }
    Ok(entry.to_owned())
}

fn restart_policy<'a>(operation: &str, policy: &'a str) -> Result<Option<&'a str>, ShellError> {
    let policy = policy.trim();
    if policy.is_empty() || policy.eq_ignore_ascii_case("no") {
        return Ok(None);
    }
    let lower = policy.to_ascii_lowercase();
    let named = matches!(lower.as_str(), "always" | "unless-stopped" | "on-failure");
    let with_retries = lower
        .strip_prefix("on-failure:")
        .is_some_and(|retries| !retries.is_empty() && retries.chars().all(|c| c.is_ascii_digit()));
    if named || with_retries {
        return Ok(Some(policy));
    }
    Err(ShellError::container_command_failed(
        operation,
        format!("{policy} is not a restart policy"),
    ))
}

/// Splits the create dialog's one-line command into argv. There is no shell on either side: quotes
/// group a word, they do not run anything. An unbalanced quote is refused rather than guessed at,
/// because guessing would start a container with a different command than the one on screen.
fn split_command_line(operation: &str, line: &str) -> Result<Vec<String>, ShellError> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(Vec::new());
    }
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    word.push(character);
                }
            }
            Some(_) => {
                if character == '"' {
                    quote = None;
                } else if character == '\\' {
                    if let Some(escaped) = characters.next() {
                        word.push(escaped);
                    }
                } else {
                    word.push(character);
                }
            }
            None => match character {
                '\'' | '"' => {
                    quote = Some(character);
                    started = true;
                }
                '\\' => {
                    if let Some(escaped) = characters.next() {
                        word.push(escaped);
                        started = true;
                    }
                }
                c if c.is_whitespace() => {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                c => {
                    word.push(c);
                    started = true;
                }
            },
        }
    }
    if quote.is_some() {
        return Err(ShellError::container_command_failed(
            operation,
            "the command has an unbalanced quote",
        ));
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

/// Approximates Docker's reference grammar: `[registry/]path[:tag][@digest]`. It exists to refuse
/// what the daemon would refuse, before a process is started and before the error has to be read
/// out of the CLI's prose. IPv6 registry literals are not accepted: no runtime in this crate needs
/// the bracket form, and accepting it means a second quoting story.
fn valid_image_reference(reference: &str) -> bool {
    if reference.is_empty() || reference.chars().count() > MAX_IMAGE_REFERENCE_CHARS {
        return false;
    }
    if reference.starts_with('-') || reference.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let (name, digest) = match reference.split_once('@') {
        Some((name, digest)) => (name, Some(digest)),
        None => (reference, None),
    };
    if let Some(digest) = digest
        && !valid_digest(digest)
    {
        return false;
    }

    // A colon before the last slash belongs to a registry port. Searching only the last segment
    // separates `localhost:5000/nginx` (port) from `nginx:1.27` (tag).
    let last_slash = name.rfind('/').map_or(0, |index| index + 1);
    let (name, tag) = match name[last_slash..].find(':') {
        Some(offset) => (&name[..last_slash + offset], Some(&name[last_slash + offset + 1..])),
        None => (name, None),
    };
    if let Some(tag) = tag
        && !valid_tag(tag)
    {
        return false;
    }

    let mut segments = name.split('/');
    let Some(first) = segments.next() else {
        return false;
    };
    if first.is_empty() {
        return false;
    }
    let has_registry = first == "localhost" || first.contains('.') || first.contains(':');
    let mut path: Vec<&str> = Vec::new();
    if has_registry {
        if !valid_registry(first) {
            return false;
        }
    } else {
        path.push(first);
    }
    path.extend(segments);
    !path.is_empty() && path.iter().all(|segment| valid_path_component(segment))
}

fn valid_registry(registry: &str) -> bool {
    let (host, port) = match registry.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (registry, None),
    };
    if let Some(port) = port
        && (port.is_empty() || port.len() > 5 || !port.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return false;
    }
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

fn valid_path_component(component: &str) -> bool {
    let mut after_separator = true;
    for byte in component.bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => after_separator = false,
            b'.' | b'_' | b'-' => {
                if after_separator {
                    return false;
                }
                after_separator = true;
            }
            _ => return false,
        }
    }
    !component.is_empty() && !after_separator
}

fn valid_tag(tag: &str) -> bool {
    let mut characters = tag.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return false;
    }
    tag.chars().count() <= 128
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn valid_digest(digest: &str) -> bool {
    let Some((algorithm, hex)) = digest.split_once(':') else {
        return false;
    };
    let valid_algorithm = !algorithm.is_empty()
        && algorithm
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '.' | '_' | '-'))
        && !algorithm.starts_with(['+', '.', '_', '-'])
        && !algorithm.ends_with(['+', '.', '_', '-']);
    valid_algorithm && hex.len() >= 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use sevenshell_types::{ContainerImageTransfer, ContainerPortMapping, ContainerRegistryLogin, ContainerVolume};

    use super::*;

    fn transfer_request(image: &str) -> ContainerImageTransfer {
        ContainerImageTransfer {
            image: image.to_owned(),
            platform: String::new(),
            all_tags: false,
            insecure: false,
        }
    }

    #[test]
    fn docker_ps_asks_for_the_fields_the_list_draws_instead_of_json() {
        assert_eq!(
            list(ContainerRuntime::Docker, ""),
            vec!["docker", "ps", "-a", "--format", PS_FORMAT_DOCKER]
        );
    }

    #[test]
    fn nerdctl_takes_its_namespace_before_the_subcommand() {
        assert_eq!(
            list(ContainerRuntime::Nerdctl, "k8s.io"),
            vec!["nerdctl", "--namespace", "k8s.io", "ps", "-a", "--format", "{{json .}}"]
        );
    }

    #[test]
    fn wslc_asks_for_the_word_json_rather_than_a_template() {
        assert_eq!(
            list(ContainerRuntime::Wslc, ""),
            vec!["wslc", "ps", "-a", "--format", "json"]
        );
        assert_eq!(
            images(ContainerRuntime::Wslc, ""),
            vec!["wslc", "images", "--format", "json"]
        );
    }

    #[test]
    fn wslc_spells_the_log_options_differently() {
        assert_eq!(
            logs(ContainerRuntime::Wslc, "", "abc123", 50, true, true).unwrap(),
            vec!["wslc", "logs", "-n", "50", "-f", "-t", "abc123"]
        );
        assert_eq!(
            logs(ContainerRuntime::Docker, "", "abc123", 50, true, true).unwrap(),
            vec!["docker", "logs", "--tail", "50", "--timestamps", "-f", "abc123"]
        );
    }

    #[test]
    fn stats_take_one_sample_and_wslc_does_not_know_no_stream() {
        assert_eq!(
            stats(ContainerRuntime::Docker, ""),
            vec!["docker", "stats", "--no-stream", "--format", "{{json .}}"]
        );
        assert_eq!(
            stats(ContainerRuntime::Wslc, ""),
            vec!["wslc", "stats", "--format", "json"]
        );
    }

    #[test]
    fn exec_asks_for_a_terminal_and_a_shell() {
        assert_eq!(
            exec(ContainerRuntime::Docker, "", "abc123", "sh").unwrap(),
            vec!["docker", "exec", "-it", "abc123", "sh"]
        );
        assert_eq!(
            exec(ContainerRuntime::Podman, "", "abc123", "  ").unwrap(),
            vec!["podman", "exec", "-it", "abc123", "sh"]
        );
    }

    #[test]
    fn the_runtimes_disagree_about_actions_they_have() {
        assert_eq!(
            action(ContainerRuntime::Wslc, "", "start", "abc123").unwrap(),
            vec!["wslc", "start", "abc123"]
        );
        assert_eq!(
            action(ContainerRuntime::Wslc, "", "remove", "abc123").unwrap(),
            vec!["wslc", "rm", "abc123"]
        );
        for action_name in ["restart", "pause", "unpause"] {
            let error = action(ContainerRuntime::Wslc, "", action_name, "abc123").unwrap_err();
            assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
        }
        assert_eq!(
            action(ContainerRuntime::Docker, "", "remove", "abc123").unwrap(),
            vec!["docker", "rm", "abc123"]
        );
    }

    #[test]
    fn kill_is_not_an_action_this_crate_offers() {
        let error = action(ContainerRuntime::Docker, "", "kill", "abc123").unwrap_err();
        assert_eq!(error.code(), sevenshell_error::ErrorCode::ContainerCommandFailed);
    }

    #[test]
    fn rename_is_refused_on_the_runtime_that_has_no_rename() {
        assert_eq!(
            rename(ContainerRuntime::Docker, "", "abc123", "web").unwrap(),
            vec!["docker", "rename", "abc123", "web"]
        );
        let error = rename(ContainerRuntime::Wslc, "", "abc123", "web").unwrap_err();
        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
    }

    #[test]
    fn an_insecure_registry_is_a_global_flag_on_nerdctl_and_a_command_flag_on_podman() {
        let mut request = transfer_request("192.168.1.10:5000/foo:1");
        request.insecure = true;
        assert_eq!(
            transfer(ContainerRuntime::Nerdctl, "default", ImageTransfer::Pull, &request).unwrap(),
            vec![
                "nerdctl",
                "--namespace",
                "default",
                "--insecure-registry",
                "192.168.1.10:5000",
                "pull",
                "192.168.1.10:5000/foo:1"
            ]
        );
        assert_eq!(
            transfer(ContainerRuntime::Podman, "", ImageTransfer::Pull, &request).unwrap(),
            vec!["podman", "pull", "--tls-verify=false", "192.168.1.10:5000/foo:1"]
        );
        // Docker and WSLC have no per-command switch; the flag must not be invented.
        assert_eq!(
            transfer(ContainerRuntime::Docker, "", ImageTransfer::Pull, &request).unwrap(),
            vec!["docker", "pull", "192.168.1.10:5000/foo:1"]
        );
    }

    #[test]
    fn every_runtime_places_platform_and_all_tags_where_it_accepts_them() {
        let mut request = transfer_request("nginx");
        request.platform = "linux/arm64".to_owned();
        request.all_tags = true;
        assert_eq!(
            transfer(ContainerRuntime::Docker, "", ImageTransfer::Pull, &request).unwrap(),
            vec!["docker", "pull", "--platform", "linux/arm64", "--all-tags", "nginx"]
        );
        assert_eq!(
            transfer(ContainerRuntime::Wslc, "", ImageTransfer::Pull, &request).unwrap(),
            vec!["wslc", "pull", "--all-tags", "nginx"]
        );
        // `--all-tags` is a pull word; a push that carried it would fail at the CLI.
        assert_eq!(
            transfer(ContainerRuntime::Docker, "", ImageTransfer::Push, &request).unwrap(),
            vec!["docker", "push", "--platform", "linux/arm64", "nginx"]
        );
    }

    #[test]
    fn login_pushes_the_password_to_standard_input_and_never_to_the_argv() {
        let login = ContainerRegistryLogin {
            registry: "registry.example.com".to_owned(),
            username: "alice".to_owned(),
            secret: sevenshell_types::SecretInput {
                value: "hunter2".to_owned(),
            },
            insecure: false,
        };
        let argv = super::login(ContainerRuntime::Docker, &login).unwrap();
        assert_eq!(
            argv,
            vec![
                "docker",
                "login",
                "--password-stdin",
                "-u",
                "alice",
                "registry.example.com"
            ]
        );
        assert!(!argv.iter().any(|argument| argument.contains("hunter2")));
    }

    #[test]
    fn an_insecure_login_puts_the_registry_flag_where_each_runtime_reads_it() {
        let mut login = ContainerRegistryLogin {
            registry: "192.168.1.10:5000".to_owned(),
            username: "alice".to_owned(),
            secret: sevenshell_types::SecretInput {
                value: "hunter2".to_owned(),
            },
            insecure: true,
        };
        assert_eq!(
            super::login(ContainerRuntime::Nerdctl, &login).unwrap(),
            vec![
                "nerdctl",
                "--insecure-registry",
                "192.168.1.10:5000",
                "login",
                "--password-stdin",
                "-u",
                "alice",
                "192.168.1.10:5000"
            ]
        );
        assert_eq!(
            super::login(ContainerRuntime::Podman, &login).unwrap(),
            vec![
                "podman",
                "login",
                "--password-stdin",
                "-u",
                "alice",
                "--tls-verify=false",
                "192.168.1.10:5000"
            ]
        );
        login.registry.clear();
        assert_eq!(
            super::login(ContainerRuntime::Docker, &login).unwrap(),
            vec!["docker", "login", "--password-stdin", "-u", "alice"]
        );
    }

    #[test]
    fn wslc_history_reads_layers_out_of_image_inspect_instead() {
        assert_eq!(
            wslc_image_inspect("", "nginx:latest").unwrap(),
            vec!["wslc", "image", "inspect", "--format", "json", "nginx:latest"]
        );
        let error = image_history(ContainerRuntime::Wslc, "", "nginx:latest").unwrap_err();
        assert_eq!(error.code(), sevenshell_error::ErrorCode::AppUnsupported);
        assert_eq!(
            image_history(ContainerRuntime::Docker, "", "abc123").unwrap(),
            vec!["docker", "history", "--no-trunc", "--format", "{{json .}}", "abc123"]
        );
    }

    #[test]
    fn wslc_copy_needs_the_container_group_and_docker_does_not() {
        assert_eq!(
            copy(ContainerRuntime::Wslc, "", "cid:/etc/nginx", "C:\\Temp\\x").unwrap(),
            vec!["wslc", "container", "cp", "cid:/etc/nginx", "C:\\Temp\\x"]
        );
        assert_eq!(
            copy(ContainerRuntime::Docker, "", "cid:/etc", "/tmp/x").unwrap(),
            vec!["docker", "cp", "cid:/etc", "/tmp/x"]
        );
    }

    #[test]
    fn wslc_reads_a_path_with_tar_because_cp_cannot_write_to_stdout() {
        assert_eq!(
            archive_export(ContainerRuntime::Wslc, "", "cid", "/etc/nginx").unwrap(),
            vec!["wslc", "exec", "cid", "sh", "-c", "tar cf - -C '/etc' 'nginx'"]
        );
    }

    #[test]
    fn create_places_every_option_before_the_image_and_the_command_after_it() {
        let request = ContainerCreateRequest {
            image: "nginx:1.27".to_owned(),
            name: "web".to_owned(),
            ports: vec![ContainerPortMapping {
                host_ip: "127.0.0.1".to_owned(),
                host_port: 8080,
                container_port: 80,
                protocol: "TCP".to_owned(),
            }],
            volumes: vec![ContainerVolume {
                source: "/data".to_owned(),
                destination: "/usr/share/nginx/html".to_owned(),
            }],
            env: vec!["MODE=fast".to_owned()],
            restart_policy: "unless-stopped".to_owned(),
            command: "nginx -g 'daemon off;'".to_owned(),
            interactive: true,
            tty: true,
        };
        assert_eq!(
            create(ContainerRuntime::Docker, "", &request).unwrap(),
            vec![
                "docker",
                "run",
                "-d",
                "-i",
                "-t",
                "--name",
                "web",
                "-p",
                "127.0.0.1:8080:80",
                "-v",
                "/data:/usr/share/nginx/html",
                "-e",
                "MODE=fast",
                "--restart",
                "unless-stopped",
                "nginx:1.27",
                "nginx",
                "-g",
                "daemon off;"
            ]
        );
        // WSLC has no --restart and no interactive creation.
        assert_eq!(
            create(ContainerRuntime::Wslc, "", &request).unwrap_err().code(),
            sevenshell_error::ErrorCode::AppUnsupported
        );
    }

    #[test]
    fn create_refuses_a_restart_policy_it_does_not_know() {
        let request = ContainerCreateRequest {
            image: "nginx".to_owned(),
            name: String::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            env: Vec::new(),
            restart_policy: "--privileged".to_owned(),
            command: String::new(),
            interactive: false,
            tty: false,
        };
        let error = create(ContainerRuntime::Docker, "", &request).unwrap_err();
        assert_eq!(error.code(), sevenshell_error::ErrorCode::ContainerCommandFailed);
    }

    #[test]
    fn wslc_creation_leaves_out_the_restart_policy_it_cannot_take() {
        let request = ContainerCreateRequest {
            image: "nginx".to_owned(),
            name: "web".to_owned(),
            ports: Vec::new(),
            volumes: Vec::new(),
            env: Vec::new(),
            restart_policy: "always".to_owned(),
            command: String::new(),
            interactive: false,
            tty: false,
        };
        let argv = create(ContainerRuntime::Wslc, "", &request).unwrap();
        assert!(
            !argv.iter().any(|argument| argument == "--restart"),
            "WSLC's run does not recognise --restart: {argv:?}"
        );
        assert_eq!(argv, vec!["wslc", "run", "-d", "--name", "web", "nginx"]);
    }

    #[test]
    fn an_option_like_container_id_is_refused_before_it_can_be_an_option() {
        for id in ["", "   ", "--help", "-x", "abc\ndef"] {
            for error in [
                inspect(ContainerRuntime::Docker, "", id).unwrap_err(),
                logs(ContainerRuntime::Docker, "", id, 10, false, false).unwrap_err(),
                action(ContainerRuntime::Docker, "", "start", id).unwrap_err(),
                rename(ContainerRuntime::Docker, "", id, "web").unwrap_err(),
                exec(ContainerRuntime::Docker, "", id, "sh").unwrap_err(),
            ] {
                assert_eq!(error.code(), sevenshell_error::ErrorCode::ContainerCommandFailed);
                assert!(error.detail().unwrap_or_default().contains("container id"), "{error:?}");
            }
        }
    }

    #[test]
    fn an_option_like_image_reference_is_refused_before_it_can_be_an_option() {
        for image in ["", "  ", "--help", "-x", "UPPER/repo", "nginx:", "nginx:ta g", "a//b"] {
            assert!(
                image_reference("the pull request", image).is_err(),
                "{image} was accepted"
            );
        }
        for image in [
            "nginx",
            "nginx:1.27",
            "library/nginx",
            "registry.example.com/team/app:sha-1234",
            "localhost:5000/app",
            "192.168.1.10:5000/app:latest",
            "app@sha256:0123456789abcdef0123456789abcdef",
        ] {
            assert_eq!(image_reference("the pull request", image).unwrap(), image);
        }
    }

    #[test]
    fn a_namespace_that_could_be_an_option_is_refused() {
        for namespace in ["-r", "--namespace", "a/b", "has space", "k8s\nio"] {
            assert!(validate_namespace(namespace).is_err(), "{namespace} was accepted");
        }
        for namespace in ["", "default", "k8s.io", "build_1"] {
            assert!(validate_namespace(namespace).is_ok(), "{namespace} was refused");
        }
    }

    #[test]
    fn a_platform_that_could_be_an_option_is_refused() {
        let mut request = transfer_request("nginx");
        request.platform = "--privileged".to_owned();
        assert!(transfer(ContainerRuntime::Docker, "", ImageTransfer::Pull, &request).is_err());
        request.platform = "linux/arm/v7".to_owned();
        assert!(transfer(ContainerRuntime::Docker, "", ImageTransfer::Pull, &request).is_ok());
    }

    #[test]
    fn the_create_dialog_command_is_split_without_a_shell() {
        let words = split_command_line("the create request", "sh -c \"echo $HOME\" 'a b'").unwrap();
        assert_eq!(words, vec!["sh", "-c", "echo $HOME", "a b"]);
        let words = split_command_line("the create request", "echo \"\"").unwrap();
        assert_eq!(words, vec!["echo", ""]);
        assert!(split_command_line("the create request", "echo 'oops").is_err());
        assert_eq!(
            split_command_line("the create request", "   ").unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn commit_and_load_are_docker_only() {
        let request = ContainerCommitRequest {
            container_id: "abc123".to_owned(),
            image: "team/web:built".to_owned(),
            author: "ops".to_owned(),
            message: "snapshot".to_owned(),
        };
        assert_eq!(
            commit(ContainerRuntime::Docker, "", &request).unwrap(),
            vec![
                "docker",
                "commit",
                "--author",
                "ops",
                "--message",
                "snapshot",
                "abc123",
                "team/web:built"
            ]
        );
        assert_eq!(
            commit(ContainerRuntime::Podman, "", &request).unwrap_err().code(),
            sevenshell_error::ErrorCode::AppUnsupported
        );
        assert_eq!(load(ContainerRuntime::Docker, ""), vec!["docker", "load"]);
    }

    #[test]
    fn namespaces_are_listed_without_a_namespace_flag() {
        assert_eq!(
            namespaces(ContainerRuntime::Nerdctl),
            vec!["nerdctl", "namespace", "ls"]
        );
    }

    #[test]
    fn a_renamed_container_cannot_carry_a_flag_for_a_name() {
        assert!(rename(ContainerRuntime::Docker, "", "abc123", "-x").is_err());
        assert!(rename(ContainerRuntime::Docker, "", "abc123", " ").is_err());
    }

    #[test]
    fn tags_are_validated_as_references() {
        assert_eq!(
            tag(ContainerRuntime::Docker, "", "abc123", "repo/name:v2").unwrap(),
            vec!["docker", "tag", "abc123", "repo/name:v2"]
        );
        assert!(tag(ContainerRuntime::Docker, "", "abc123", "--force").is_err());
    }

    #[test]
    fn a_registry_address_cannot_be_a_command_or_a_path() {
        let login = ContainerRegistryLogin {
            registry: "https://registry.example.com/v2".to_owned(),
            username: "alice".to_owned(),
            secret: sevenshell_types::SecretInput {
                value: "hunter2".to_owned(),
            },
            insecure: false,
        };
        assert!(super::login(ContainerRuntime::Docker, &login).is_err());
    }

    #[test]
    fn a_container_id_cannot_be_smuggled_in_through_the_namespace() {
        assert!(validate_namespace("--namespace").is_err());
        assert!(validate_namespace("-r").is_err());
    }
}

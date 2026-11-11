//! Files inside a container, reached through a shell in it.
//!
//! A container has no SFTP subsystem and no path this machine can open: its filesystem exists only
//! while a process inside it looks at it. Every operation here is therefore one `sh -c` script run
//! through the runtime's own `exec`, with each path quoted before it becomes part of that script,
//! and with no unquoted text interpolated anywhere. The path is the hostile input of this module:
//! it names something inside a container whose filesystem may have been built by anyone.
//!
//! Two consequences follow from the runner contract and are deliberate:
//!
//! * A download is `tar cf - … | base64`, because the runner answers with text and a tar stream is
//!   bytes. That costs a base64 round trip inside the container and caps a download at the runner's
//!   capture ceiling ([`MAX_TRANSFER_BYTES`]). An upload travels the other way as raw tar on
//!   standard input, which the runner does carry byte for byte.
//! * Everything except [`ContainerFiles::list`] needs the container running, because `exec` needs
//!   it. A stopped container can be exported by a runtime's own `cp`, but that command writes to
//!   this machine's disk (or, over SSH, to a machine this process cannot read), so it is not a
//!   channel this crate has. A stopped container answers with the runtime's own refusal.
//!
//! Nothing here decides policy: a deletion is a deletion, and whether the person meant it is a
//! question for the layer that offers it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sevenshell_error::ShellError;
use sevenshell_types::{ContainerRuntime, EntryKind, SftpEntry, SftpListing};

use crate::Provider;
use crate::commands;
use crate::runner::Runner;

/// The longest path this module will send into a container. Linux's own `PATH_MAX` is 4096.
const MAX_PATH_CHARS: usize = 4096;
/// The same ceiling `commands.rs` puts on a container identifier, restated here because a file
/// operation takes no builder from that module.
const MAX_CONTAINER_ID_CHARS: usize = 255;
/// How much text the editor will read or write in one call. A file of this size is already past
/// what a text area can usefully edit.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// How many bytes of tar this channel will carry in one transfer.
///
/// The runner collects a command's output as text and caps it at 32 MiB, and a download spends a
/// third of that ceiling on base64; a tree larger than this is refused here so the person is told
/// why instead of receiving a truncated archive.
pub const MAX_TRANSFER_BYTES: u64 = 32 * 1024 * 1024;
const TAR_BLOCK: usize = 512;

/// The two ways a file command runs: an answer as text, or an answer with bytes on standard input.
///
/// A trait rather than calls straight to [`Runner`] so the tests can hand this module a recorder
/// and prove that a refused path never reached the step that starts a process.
#[async_trait]
trait CommandTransport: Send + Sync {
    async fn run(&self, argv: &[String]) -> Result<String, ShellError>;
    async fn run_stdin(&self, argv: &[String], input: Vec<u8>) -> Result<String, ShellError>;
}

#[async_trait]
impl CommandTransport for Runner {
    async fn run(&self, argv: &[String]) -> Result<String, ShellError> {
        Runner::run(self, argv).await
    }

    async fn run_stdin(&self, argv: &[String], input: Vec<u8>) -> Result<String, ShellError> {
        Runner::run_stdin(self, argv, input).await
    }
}

/// The files of one container, addressed by the same paths a listing shows.
///
/// The runtime and namespace come from the connection that produced the provider; the runner is
/// passed beside it because a provider keeps its runner to itself. A host that wires this up keeps
/// the same [`Runner`] value it connected with (a second `Arc` of the same SSH session is the same
/// connection), or takes one from the provider once the provider can hand it out.
pub struct ContainerFiles {
    runtime: ContainerRuntime,
    namespace: String,
    container: String,
    transport: Arc<dyn CommandTransport>,
}

impl ContainerFiles {
    /// Files of `container` reached through `runner`.
    pub fn new(
        runtime: ContainerRuntime,
        namespace: &str,
        container: &str,
        runner: Runner,
    ) -> Result<Self, ShellError> {
        Self::from_transport(runtime, namespace, container, Arc::new(runner))
    }

    /// Files of `container` on the connection `provider` describes.
    pub fn for_provider(provider: &Provider, container: &str, runner: Runner) -> Result<Self, ShellError> {
        Self::new(provider.runtime(), provider.namespace(), container, runner)
    }

    fn from_transport(
        runtime: ContainerRuntime,
        namespace: &str,
        container: &str,
        transport: Arc<dyn CommandTransport>,
    ) -> Result<Self, ShellError> {
        commands::validate_namespace(namespace)?;
        let container = container_id(container)?;
        Ok(Self {
            runtime,
            namespace: namespace.trim().to_owned(),
            container: container.to_owned(),
            transport,
        })
    }

    /// The container this handle addresses, as it was given.
    pub fn container(&self) -> &str {
        &self.container
    }

    pub fn runtime(&self) -> ContainerRuntime {
        self.runtime
    }

    /// The directory as the container resolved it, with one entry per name `ls -la` printed.
    ///
    /// The resolved path travels with the entries because it is the one the breadcrumbs should
    /// show: a listing of a symlinked directory would otherwise keep naming the link. `.` and `..`
    /// are not entries: a file browser that grows a parent row from every listing draws them twice.
    pub async fn list(&self, dir: &str) -> Result<SftpListing, ShellError> {
        let dir = container_path("the file listing", dir)?;
        // `LC_ALL=C` because a localized `ls` prints localized month names, and an unparsed date
        // is a file with no timestamp in the interface.
        let script = format!("cd {} && pwd -P && LC_ALL=C ls -la", quote(&dir));
        let output = self.transport.run(&self.exec_argv(&script, false)).await?;
        Ok(parse_listing(&dir, &output))
    }

    /// `mkdir -p`: a file browser creates one level, and a person who typed two expects both.
    pub async fn create_dir(&self, path: &str) -> Result<(), ShellError> {
        let path = container_path("the create-directory request", path)?;
        self.run_script(&format!("mkdir -p {}", quote(&path))).await
    }

    /// An empty file. `touch` rather than a redirection: a redirection would truncate a file that
    /// is already there, which is the one thing "create" must not do by surprise.
    pub async fn create_file(&self, path: &str) -> Result<(), ShellError> {
        let path = container_path("the create-file request", path)?;
        require_not_root("the create-file request", &path)?;
        self.run_script(&format!("touch {}", quote(&path))).await
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), ShellError> {
        let from = container_path("the rename request", from)?;
        let to = container_path("the rename request", to)?;
        require_not_root("the rename request", &from)?;
        require_not_root("the rename request", &to)?;
        self.run_script(&format!("mv {} {}", quote(&from), quote(&to))).await
    }

    /// A copy inside the container. `-a` keeps modes, times and links, so a copied tree is the tree
    /// that was copied.
    pub async fn copy(&self, from: &str, to: &str) -> Result<(), ShellError> {
        let from = container_path("the copy request", from)?;
        let to = container_path("the copy request", to)?;
        require_not_root("the copy request", &from)?;
        require_not_root("the copy request", &to)?;
        self.run_script(&format!("cp -a {} {}", quote(&from), quote(&to))).await
    }

    /// `-f` for a file and `-rf` for a tree. A directory without `recursive` is refused by `rm`
    /// itself, which is the honest answer: the person sees it was not removed rather than losing a
    /// tree they meant to keep.
    pub async fn remove(&self, path: &str, recursive: bool) -> Result<(), ShellError> {
        let path = container_path("the removal request", path)?;
        require_not_root("the removal request", &path)?;
        let flag = if recursive { "-rf" } else { "-f" };
        self.run_script(&format!("rm {flag} {}", quote(&path))).await
    }

    /// The permission bits, as the three or four octal digits `chmod` takes.
    pub async fn set_mode(&self, path: &str, mode: u32) -> Result<(), ShellError> {
        let path = container_path("the permission request", path)?;
        require_not_root("the permission request", &path)?;
        if mode > 0o7777 {
            return Err(ShellError::container_command_failed(
                "the permission request",
                format!("{mode:o} is not a set of permission bits"),
            ));
        }
        self.run_script(&format!("chmod {mode:04o} {}", quote(&path))).await
    }

    /// A file as text, bounded by [`MAX_TEXT_BYTES`] and refused unless it is UTF-8.
    pub async fn read_text(&self, path: &str) -> Result<String, ShellError> {
        let path = container_path("the file read", path)?;
        require_not_root("the file read", &path)?;
        let (directory, name) = split_parent(&path);
        let script = format!("tar cf - -C {} {} | base64", quote(directory), quote(name));
        let encoded = self.transport.run(&self.exec_argv(&script, false)).await?;
        let archive = decode_base64(&encoded)?;
        let entries = read_tar(&archive)?;
        let entry = entries.first().ok_or_else(|| archive_error("the archive was empty"))?;
        if entry.name.trim_end_matches('/') != name || entry.kind != b'0' {
            return Err(ShellError::container_command_failed(
                "the file read",
                format!("{path} is not a regular file"),
            ));
        }
        if entry.data.len() > MAX_TEXT_BYTES {
            return Err(ShellError::container_command_failed(
                "the file read",
                format!("{path} is larger than the editor's {MAX_TEXT_BYTES}-byte limit"),
            ));
        }
        String::from_utf8(entry.data.to_vec())
            .map_err(|_| ShellError::container_command_failed("the file read", format!("{path} is not UTF-8 text")))
    }

    /// Replaces a file's content, creating the folders above it when they are missing: saving into
    /// a folder that does not exist yet is what "save as" means.
    pub async fn write_text(&self, path: &str, content: &str) -> Result<(), ShellError> {
        let path = container_path("the file write", path)?;
        require_not_root("the file write", &path)?;
        if content.len() > MAX_TEXT_BYTES {
            return Err(ShellError::container_command_failed(
                "the file write",
                format!("the text is larger than the editor's {MAX_TEXT_BYTES}-byte limit"),
            ));
        }
        let (directory, name) = split_parent(&path);
        let mut builder = TarBuilder::new();
        builder.file(name, 0o644, 0, content.as_bytes())?;
        let archive = builder.finish();
        let script = format!("mkdir -p {} && tar xf - -C {}", quote(directory), quote(directory));
        self.transport
            .run_stdin(&self.exec_argv(&script, true), archive)
            .await
            .map(|_| ())
    }

    /// Writes the file or tree at `path` into `destination` on this machine.
    ///
    /// `destination` is the path itself: a file is written there, a directory is created there and
    /// its children land inside. Every entry the archive carries is checked to stay under that
    /// path; an archive with an entry that would land somewhere else is refused whole rather than
    /// half applied.
    pub async fn download(&self, path: &str, destination: &Path) -> Result<(), ShellError> {
        let path = container_path("the download", path)?;
        require_not_root("the download", &path)?;
        let (directory, name) = split_parent(&path);
        let script = format!("tar cf - -C {} {} | base64", quote(directory), quote(name));
        let encoded = self.transport.run(&self.exec_argv(&script, false)).await?;
        let archive = decode_base64(&encoded)?;
        ensure_transfer_fits(archive.len() as u64)?;
        extract_archive(&archive, name, destination)
    }

    /// Packs a local file or tree and extracts it inside `destination` in the container.
    ///
    /// The archive names the chosen file after its own name, so uploading `site` into `/srv` adds
    /// `/srv/site/…`, which is what the person selected. The folder is created when missing.
    pub async fn upload(&self, local: &Path, destination: &str) -> Result<(), ShellError> {
        let destination = container_path("the upload", destination)?;
        let archive = tar_local(local)?;
        let script = format!(
            "mkdir -p {} && tar xf - -C {}",
            quote(&destination),
            quote(&destination)
        );
        self.transport
            .run_stdin(&self.exec_argv(&script, true), archive)
            .await
            .map(|_| ())
    }

    fn exec_argv(&self, script: &str, stdin: bool) -> Vec<String> {
        let mut argv = vec![self.runtime.program().to_owned()];
        // nerdctl's namespace is a global flag and has to precede the subcommand.
        if self.runtime == ContainerRuntime::Nerdctl && !self.namespace.is_empty() {
            argv.push("--namespace".to_owned());
            argv.push(self.namespace.clone());
        }
        argv.push("exec".to_owned());
        // `-i` and never `-t`: a terminal would merge the two streams and translate line endings,
        // and every byte of a tar stream matters.
        if stdin {
            argv.push("-i".to_owned());
        }
        argv.push(self.container.clone());
        argv.push("sh".to_owned());
        argv.push("-c".to_owned());
        argv.push(script.to_owned());
        argv
    }

    async fn run_script(&self, script: &str) -> Result<(), ShellError> {
        self.transport.run(&self.exec_argv(script, false)).await.map(|_| ())
    }
}

/// The container identifier as an argv element: never empty, never an option, never a control
/// character. It travels outside the script, so quoting alone would not make `-x` harmless.
fn container_id(container: &str) -> Result<&str, ShellError> {
    let container = container.trim();
    if container.is_empty() {
        return Err(ShellError::container_command_failed(
            "the file request",
            "container id is empty",
        ));
    }
    if container.chars().count() > MAX_CONTAINER_ID_CHARS {
        return Err(ShellError::container_command_failed(
            "the file request",
            format!("container id is longer than {MAX_CONTAINER_ID_CHARS} characters"),
        ));
    }
    if container.starts_with('-') {
        return Err(ShellError::container_command_failed(
            "the file request",
            "container id starts with '-', which the runtime would read as an option",
        ));
    }
    if container.chars().any(char::is_control) {
        return Err(ShellError::container_command_failed(
            "the file request",
            "container id contains a control character",
        ));
    }
    Ok(container)
}

/// A path as this module will send it: absolute, canonical enough to quote, and with no component
/// that could climb. A relative path is refused rather than resolved against an `exec` working
/// directory that is not part of this API's contract.
fn container_path(operation: &str, path: &str) -> Result<String, ShellError> {
    if path.is_empty() {
        return Err(ShellError::container_command_failed(operation, "path is empty"));
    }
    if path.chars().count() > MAX_PATH_CHARS {
        return Err(ShellError::container_command_failed(
            operation,
            format!("path is longer than {MAX_PATH_CHARS} characters"),
        ));
    }
    if path.chars().any(char::is_control) {
        return Err(ShellError::container_command_failed(
            operation,
            "path contains a control character",
        ));
    }
    let Some(rest) = path.strip_prefix('/') else {
        return Err(ShellError::container_command_failed(
            operation,
            format!("{path} is not an absolute path"),
        ));
    };
    if rest.is_empty() {
        return Ok("/".to_owned());
    }
    let mut components = Vec::new();
    for component in rest.split('/') {
        if component.is_empty() {
            return Err(ShellError::container_command_failed(
                operation,
                format!("{path} has an empty path component"),
            ));
        }
        if component == "." || component == ".." {
            return Err(ShellError::container_command_failed(
                operation,
                format!("{path} has a relative path component"),
            ));
        }
        components.push(component);
    }
    Ok(format!("/{}", components.join("/")))
}

/// The container's own root is a directory nobody browses in order to edit it, and the four
/// commands that could destroy a running system all name a path. Refusing `/` is cheap insurance.
fn require_not_root(operation: &str, path: &str) -> Result<(), ShellError> {
    if path == "/" {
        return Err(ShellError::container_command_failed(
            operation,
            "the container's root directory cannot be changed or removed",
        ));
    }
    Ok(())
}

/// One word, quoted for the shell *inside* the container. Only scripts that container runs see
/// this; this machine never has a shell in the path.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The directory and the last component. A caller has already refused `/`, so the name is never
/// empty.
fn split_parent(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((parent, name)) if !parent.is_empty() => (parent, name),
        _ => ("/", &path[1..]),
    }
}

// --- Listing -----------------------------------------------------------------

/// One entry as `ls -la` printed it.
struct ListedEntry {
    name: String,
    kind: EntryKind,
    mode: u32,
    size: u64,
    modified_at: Option<u64>,
    owner: String,
    group: String,
    link_target: Option<String>,
}

/// Reads the answer to `cd … && pwd -P && LC_ALL=C ls -la`. The first line is the physical path, because
/// that is what this module asked for; every later line is one entry, and a line that does not
/// parse is skipped — `ls` is not necessarily the only thing that ever wrote to this shell.
fn parse_listing(requested: &str, output: &str) -> SftpListing {
    let mut lines = output.split('\n');
    let resolved = lines
        .next()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| line.starts_with('/'))
        .unwrap_or(requested)
        .to_owned();
    let mut entries = Vec::new();
    for line in lines {
        let Some(entry) = parse_ls_line(line) else {
            continue;
        };
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        entries.push(SftpEntry {
            path: join_path(&resolved, &entry.name),
            name: entry.name,
            kind: entry.kind,
            size: entry.size,
            modified_at: entry.modified_at,
            mode: Some(entry.mode),
            owner: Some(entry.owner),
            group: Some(entry.group),
            uid: None,
            gid: None,
            link_target: entry.link_target,
        });
    }
    SftpListing {
        path: resolved,
        entries,
    }
}

/// One line: `type-and-permissions links owner group size date name`.
///
/// The date is three tokens in the classic form (`Nov 10 09:15`, `Nov 10  2020`) and two in the
/// ISO form busybox prints (`2026-11-10 10:11`). Only one space after the date belongs to the
/// format; anything beyond it is part of a name that begins with spaces.
fn parse_ls_line(line: &str) -> Option<ListedEntry> {
    let mut rest = line.trim_end_matches('\r');
    if rest.trim().is_empty() {
        return None;
    }
    let mode_field = take_field(&mut rest)?;
    let type_flag = mode_field.chars().next()?;
    let mode = permission_bits(mode_field)?;
    // The link count is what separates a real listing line from a banner a login script wrote.
    let _links: u64 = take_field(&mut rest)?.parse().ok()?;
    let owner = take_field(&mut rest)?.to_owned();
    let group = take_field(&mut rest)?.to_owned();
    let size: u64 = take_field(&mut rest)?.parse().ok()?;
    let modified_at = take_date(&mut rest)?;
    rest = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('\t'))?;
    if rest.is_empty() {
        return None;
    }
    let (name, link_target) = if type_flag == 'l' {
        // Only a link's suffix is a target. A regular file may be named `report -> final.txt`.
        match rest.split_once(" -> ") {
            Some((name, target)) => (name, Some(target.to_owned())),
            None => (rest, None),
        }
    } else {
        (rest, None)
    };
    let kind = match type_flag {
        'd' => EntryKind::Directory,
        'l' => EntryKind::Symlink,
        '-' => EntryKind::File,
        _ => EntryKind::Other,
    };
    Some(ListedEntry {
        name: name.to_owned(),
        kind,
        mode,
        size,
        modified_at,
        owner,
        group,
        link_target,
    })
}

/// The next whitespace-delimited field, leaving the rest at the delimiter that followed it.
fn take_field<'a>(rest: &mut &'a str) -> Option<&'a str> {
    let trimmed = rest.trim_start_matches([' ', '\t']);
    if trimmed.is_empty() {
        *rest = trimmed;
        return None;
    }
    match trimmed.find([' ', '\t']) {
        Some(index) => {
            let field = &trimmed[..index];
            *rest = &trimmed[index..];
            Some(field)
        }
        None => {
            *rest = "";
            Some(trimmed)
        }
    }
}

fn first_field(rest: &str) -> &str {
    let trimmed = rest.trim_start_matches([' ', '\t']);
    match trimmed.find([' ', '\t']) {
        Some(index) => &trimmed[..index],
        None => trimmed,
    }
}

/// The date, consumed from `rest`, or `None` when this line does not carry one.
fn take_date(rest: &mut &str) -> Option<Option<u64>> {
    let first = first_field(rest);
    if is_iso_date(first) {
        let date = take_field(rest)?;
        let time = take_field(rest)?;
        return Some(iso_timestamp(date, time));
    }
    let month = month_index(first)?;
    let _month = take_field(rest)?;
    let day = take_field(rest)?.parse::<u32>().ok()?;
    let third = take_field(rest)?;
    Some(classic_timestamp(month, day, third))
}

fn is_iso_date(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(|byte| byte.is_ascii_digit())
        && bytes[5..7].iter().all(|byte| byte.is_ascii_digit())
        && bytes[8..].iter().all(|byte| byte.is_ascii_digit())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn month_index(token: &str) -> Option<u32> {
    MONTHS
        .iter()
        .position(|month| *month == token)
        .map(|index| index as u32 + 1)
}

/// A date without a year is a file `ls` considers recent: the year is this machine's, which is the
/// one `ls` meant. A date with a year carries no time, and midnight is the honest answer — the date
/// is right and the time was never printed.
fn classic_timestamp(month: u32, day: u32, third: &str) -> Option<u64> {
    if third.contains(':') {
        let (hour, minute, second) = parse_clock(third)?;
        timestamp(current_utc_year(), month, day, hour, minute, second)
    } else {
        let year = third.parse::<i64>().ok()?;
        timestamp(year, month, day, 0, 0, 0)
    }
}

fn iso_timestamp(date: &str, time: &str) -> Option<u64> {
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let (hour, minute, second) = parse_clock(time)?;
    timestamp(year, month, day, hour, minute, second)
}

fn parse_clock(clock: &str) -> Option<(u32, u32, u32)> {
    let mut parts = clock.split(':');
    let hour = parts.next()?.parse::<u32>().ok()?;
    let minute = parts.next()?.parse::<u32>().ok()?;
    let second = match parts.next() {
        Some(second) => second.parse::<u32>().ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((hour, minute, second))
}

/// The permission bits in `rwx` form, including the setuid, setgid and sticky positions.
fn permission_bits(mode: &str) -> Option<u32> {
    let flags: Vec<char> = mode.chars().collect();
    if flags.len() < 10 {
        return None;
    }
    let mut bits = 0u32;
    for (start, shift) in [(1usize, 6u32), (4, 3), (7, 0)] {
        if flags[start] == 'r' {
            bits |= 4 << shift;
        }
        if flags[start + 1] == 'w' {
            bits |= 2 << shift;
        }
        match flags[start + 2] {
            'x' => bits |= 1 << shift,
            's' if start == 1 => bits |= 0o4000 | 0o100,
            's' if start == 4 => bits |= 0o2000 | 0o010,
            'S' if start == 1 => bits |= 0o4000,
            'S' if start == 4 => bits |= 0o2000,
            't' => bits |= 0o1000 | 0o001,
            'T' => bits |= 0o1000,
            _ => {}
        }
    }
    Some(bits)
}

fn join_path(directory: &str, name: &str) -> String {
    if directory == "/" {
        format!("/{name}")
    } else {
        format!("{directory}/{name}")
    }
}

fn current_utc_year() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(seconds.div_euclid(86_400)).0
}

/// The day `year-month-day` is, counting from 1970-01-01, and back. Arithmetic rather than a date
/// crate: this is the only calendar this crate needs, and it is four lines of division.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Seconds since the epoch, or nothing when the listing printed a date that cannot be one. The
/// container's timezone is not part of `ls -la`'s answer, so the date is read as UTC rather than
/// guessed at with this machine's offset.
fn timestamp(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Option<u64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, i64::from(month), i64::from(day));
    let seconds = days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    u64::try_from(seconds).ok()
}

// --- Archives ----------------------------------------------------------------

/// One entry of a tar stream, its bytes borrowed from the stream.
struct TarEntry<'a> {
    name: String,
    /// The `typeflag` byte; a regular file is `0`.
    kind: u8,
    mode: u32,
    /// The target of a link, empty for anything else.
    link: String,
    data: &'a [u8],
}

fn archive_error(detail: impl AsRef<str>) -> ShellError {
    ShellError::container_command_failed("the archive", detail)
}

/// Reads a tar stream into entries.
///
/// GNU long names (`L`, `K`) and PAX records (`x`) are understood because busybox tar writes them
/// for paths a ustar header cannot hold, and a container's paths are not this module's choice. A
/// header whose checksum does not check out ends the parse: that is a stream this build misread,
/// not a valid archive with an odd entry.
fn read_tar(archive: &[u8]) -> Result<Vec<TarEntry<'_>>, ShellError> {
    let mut entries = Vec::new();
    let mut offset = 0usize;
    let mut pending_name: Option<String> = None;
    let mut pending_link: Option<String> = None;
    let mut pax_name: Option<String> = None;
    let mut pax_link: Option<String> = None;
    while offset + TAR_BLOCK <= archive.len() {
        let block = &archive[offset..offset + TAR_BLOCK];
        if block.iter().all(|byte| *byte == 0) {
            break;
        }
        if !tar_checksum_ok(block) {
            return Err(archive_error(
                "an entry header does not check out; the stream was not a tar archive",
            ));
        }
        let size = usize::try_from(tar_number(&block[124..136])?)
            .map_err(|_| archive_error("an entry is larger than this machine can address"))?;
        let kind = block[156];
        let data_start = offset + TAR_BLOCK;
        let data_end = data_start
            .checked_add(size)
            .filter(|end| *end <= archive.len())
            .ok_or_else(|| archive_error("the archive ends in the middle of an entry"))?;
        let data = &archive[data_start..data_end];
        offset = data_start + size.div_ceil(TAR_BLOCK) * TAR_BLOCK;
        match kind {
            b'L' => {
                pending_name = Some(tar_text(data));
                continue;
            }
            b'K' => {
                pending_link = Some(tar_text(data));
                continue;
            }
            b'x' | b'X' => {
                for (key, value) in pax_records(data) {
                    match key.as_str() {
                        "path" => pax_name = Some(value),
                        "linkpath" => pax_link = Some(value),
                        _ => {}
                    }
                }
                continue;
            }
            // A global PAX header changes defaults this reader does not keep.
            b'g' => continue,
            _ => {}
        }
        let name = pending_name
            .take()
            .or_else(|| pax_name.take())
            .unwrap_or_else(|| tar_header_name(block));
        let link = pending_link
            .take()
            .or_else(|| pax_link.take())
            .unwrap_or_else(|| tar_text(&block[157..257]));
        let mode = u32::try_from(tar_number(&block[100..108])?).unwrap_or(0);
        entries.push(TarEntry {
            name,
            kind: if kind == 0 { b'0' } else { kind },
            mode,
            link,
            data,
        });
    }
    Ok(entries)
}

fn tar_checksum_ok(block: &[u8]) -> bool {
    let Ok(stored) = tar_number(&block[148..156]) else {
        return false;
    };
    let computed: u64 = block
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum();
    stored == computed
}

fn tar_number(field: &[u8]) -> Result<u64, ShellError> {
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(field.len());
    let digits: Vec<u8> = field[..end].iter().copied().filter(|byte| *byte != b' ').collect();
    if digits.is_empty() {
        return Ok(0);
    }
    let text = std::str::from_utf8(&digits).map_err(|_| archive_error("a header field is not a number"))?;
    u64::from_str_radix(text, 8).map_err(|_| archive_error("a header field is not a number"))
}

fn tar_text(field: &[u8]) -> String {
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn tar_header_name(block: &[u8]) -> String {
    let name = tar_text(&block[..100]);
    let prefix = tar_text(&block[345..500]);
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

fn pax_records(data: &[u8]) -> Vec<(String, String)> {
    let mut records = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let Some(space) = rest.iter().position(|byte| *byte == b' ') else {
            break;
        };
        let Some(length) = std::str::from_utf8(&rest[..space])
            .ok()
            .and_then(|text| text.parse::<usize>().ok())
        else {
            break;
        };
        if length <= space + 1 || length > rest.len() {
            break;
        }
        let mut record = &rest[space + 1..length];
        if let Some(without_newline) = record.strip_suffix(b"\n") {
            record = without_newline;
        }
        if let Some(equals) = record.iter().position(|byte| *byte == b'=') {
            records.push((
                String::from_utf8_lossy(&record[..equals]).into_owned(),
                String::from_utf8_lossy(&record[equals + 1..]).into_owned(),
            ));
        }
        rest = &rest[length..];
    }
    records
}

/// A ustar archive built entry by entry. ustar rather than the GNU extensions: GNU tar and busybox
/// tar both read it, and its 255-byte path ceiling is far past anything a file browser addresses.
struct TarBuilder {
    bytes: Vec<u8>,
}

impl TarBuilder {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn directory(&mut self, name: &str, mode: u32, modified_at: u64) -> Result<(), ShellError> {
        let name = format!("{}/", name.trim_end_matches('/'));
        self.header(&name, b'5', mode, modified_at, 0, "")
    }

    fn file(&mut self, name: &str, mode: u32, modified_at: u64, data: &[u8]) -> Result<(), ShellError> {
        self.header(name, b'0', mode, modified_at, data.len() as u64, "")?;
        self.bytes.extend_from_slice(data);
        let remainder = data.len() % TAR_BLOCK;
        if remainder != 0 {
            self.bytes.resize(self.bytes.len() + TAR_BLOCK - remainder, 0);
        }
        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        self.bytes.resize(self.bytes.len() + 2 * TAR_BLOCK, 0);
        self.bytes
    }

    fn header(
        &mut self,
        name: &str,
        kind: u8,
        mode: u32,
        modified_at: u64,
        size: u64,
        link: &str,
    ) -> Result<(), ShellError> {
        // Eleven octal digits hold 8 GiB and dates past the year 2242; the transfer bound is far
        // below the first and a file's mtime is long before the second.
        if size > 0o77_777_777_777 || modified_at > 0o77_777_777_777 {
            return Err(archive_error("an entry is too large for this build's archive format"));
        }
        let (short_name, prefix) = split_tar_name(name)?;
        let mut block = [0u8; TAR_BLOCK];
        block[..short_name.len()].copy_from_slice(short_name.as_bytes());
        set_octal(&mut block[100..108], u64::from(mode));
        set_octal(&mut block[108..116], 0); // uid
        set_octal(&mut block[116..124], 0); // gid
        set_octal(&mut block[124..136], size);
        set_octal(&mut block[136..148], modified_at);
        block[148..156].fill(b' ');
        block[156] = kind;
        let link_bytes = link.as_bytes();
        if link_bytes.len() >= 100 {
            return Err(archive_error("a link target is too long for this build's archive"));
        }
        block[157..157 + link_bytes.len()].copy_from_slice(link_bytes);
        block[257..263].copy_from_slice(b"ustar\0");
        block[263..265].copy_from_slice(b"00");
        block[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        let checksum: u64 = block.iter().map(|byte| u64::from(*byte)).sum();
        let text = format!("{checksum:06o}\0 ");
        block[148..156].copy_from_slice(text.as_bytes());
        self.bytes.extend_from_slice(&block);
        Ok(())
    }
}

fn set_octal(field: &mut [u8], value: u64) {
    let width = field.len() - 1;
    let text = format!("{value:0width$o}");
    debug_assert_eq!(text.len(), width, "the caller checked the ceiling");
    field[..width].copy_from_slice(text.as_bytes());
    field[width] = 0;
}

/// ustar keeps 100 bytes for the name and 155 for a prefix joined back with a slash; a longer path
/// has no representation here.
fn split_tar_name(name: &str) -> Result<(&str, &str), ShellError> {
    if name.len() <= 100 {
        return Ok((name, ""));
    }
    for (index, _) in name.match_indices('/').rev() {
        let (prefix, rest) = name.split_at(index);
        let suffix = &rest[1..];
        if !suffix.is_empty() && suffix.len() <= 100 && prefix.len() <= 155 {
            return Ok((suffix, prefix));
        }
    }
    Err(archive_error(
        "a path is longer than 255 bytes, which this build's archive cannot carry",
    ))
}

fn ensure_transfer_fits(bytes: u64) -> Result<(), ShellError> {
    if bytes > MAX_TRANSFER_BYTES {
        return Err(ShellError::container_command_failed(
            "the transfer",
            format!(
                "the tree is larger than the {} MiB this channel carries",
                MAX_TRANSFER_BYTES / (1024 * 1024)
            ),
        ));
    }
    Ok(())
}

fn charge(budget: &mut u64, bytes: u64) -> Result<(), ShellError> {
    *budget = budget.checked_sub(bytes).ok_or_else(|| {
        ShellError::container_command_failed("the transfer", "the upload is too large for this channel")
    })?;
    Ok(())
}

// --- Base64 ------------------------------------------------------------------

/// Decodes what the container's `base64` printed. busybox wraps lines, so whitespace is ignored;
/// anything else that is not in the alphabet is refused rather than skipped.
fn decode_base64(text: &str) -> Result<Vec<u8>, ShellError> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let mut group = [0u8; 4];
    let mut count = 0usize;
    let mut padding = 0usize;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 64,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => {
                return Err(archive_error(format!(
                    "the container printed {:?}, which is not base64",
                    char::from(byte)
                )));
            }
        };
        if padding > 0 && value != 64 {
            return Err(archive_error("the base64 stream carries data after its padding"));
        }
        if value == 64 {
            padding += 1;
        }
        group[count] = value;
        count += 1;
        if count == 4 {
            bytes.push(group[0] << 2 | group[1] >> 4);
            if group[2] != 64 {
                bytes.push(group[1] << 4 | group[2] >> 2);
            }
            if group[3] != 64 {
                bytes.push(group[2] << 6 | group[3]);
            }
            count = 0;
        }
    }
    if count != 0 {
        return Err(archive_error("the base64 stream ended in the middle of a group"));
    }
    if padding > 2 {
        return Err(archive_error("the base64 stream has too much padding"));
    }
    Ok(bytes)
}

// --- Uploading a local tree --------------------------------------------------

fn tar_local(root: &Path) -> Result<Vec<u8>, ShellError> {
    let metadata =
        std::fs::symlink_metadata(root).map_err(|error| local_failure("the upload", "the chosen path", &error))?;
    let base = root
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ShellError::container_command_failed("the upload", "the chosen path has no file name"))?
        .to_owned();
    let mut builder = TarBuilder::new();
    let mut budget = MAX_TRANSFER_BYTES;
    if metadata.is_dir() {
        add_directory(&mut builder, root, &base, &mut budget)?;
    } else if metadata.is_file() {
        add_file(&mut builder, root, &base, &metadata, &mut budget)?;
    } else {
        return Err(ShellError::container_command_failed(
            "the upload",
            "only a file or a folder can be uploaded",
        ));
    }
    Ok(builder.finish())
}

fn add_directory(builder: &mut TarBuilder, local: &Path, name: &str, budget: &mut u64) -> Result<(), ShellError> {
    let metadata = std::fs::symlink_metadata(local).map_err(|error| local_failure("the upload", name, &error))?;
    builder.directory(name, permission_mode(&metadata), modified_at(&metadata))?;
    let read = std::fs::read_dir(local).map_err(|error| local_failure("the upload", name, &error))?;
    let mut children: Vec<PathBuf> = read
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| local_failure("the upload", name, &error))?;
    // Sorted so that two uploads of the same folder produce the same archive; the order is not a
    // contract the container sees, it is what makes a test able to name the entries.
    children.sort();
    for child in children {
        let Some(child_name) = child.file_name().and_then(std::ffi::OsStr::to_str) else {
            return Err(ShellError::container_command_failed(
                "the upload",
                format!("a name inside {name} is not UTF-8"),
            ));
        };
        let path = format!("{name}/{child_name}");
        let child_metadata =
            std::fs::symlink_metadata(&child).map_err(|error| local_failure("the upload", &path, &error))?;
        if child_metadata.file_type().is_symlink() {
            // A link to a file travels as its content, which is what choosing a tree means. A link
            // to a folder is skipped: following one is how an upload walks in circles.
            match std::fs::metadata(&child) {
                Ok(target) if target.is_file() => add_file(builder, &child, &path, &target, budget)?,
                Ok(target) if target.is_dir() => {
                    log::warn!("skipping the folder link {path} while packing an upload");
                }
                _ => log::warn!("skipping the link {path} while packing an upload"),
            }
        } else if child_metadata.is_dir() {
            add_directory(builder, &child, &path, budget)?;
        } else if child_metadata.is_file() {
            add_file(builder, &child, &path, &child_metadata, budget)?;
        }
        // A socket, a fifo or a device has no bytes to carry and is skipped.
    }
    Ok(())
}

fn add_file(
    builder: &mut TarBuilder,
    local: &Path,
    name: &str,
    metadata: &std::fs::Metadata,
    budget: &mut u64,
) -> Result<(), ShellError> {
    charge(budget, metadata.len())?;
    let data = std::fs::read(local).map_err(|error| local_failure("the upload", name, &error))?;
    builder.file(name, permission_mode(metadata), modified_at(metadata), &data)
}

#[cfg(unix)]
fn permission_mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn permission_mode(metadata: &std::fs::Metadata) -> u32 {
    if metadata.is_dir() { 0o755 } else { 0o644 }
}

fn modified_at(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

// --- Downloading into this machine -------------------------------------------

fn extract_archive(archive: &[u8], root: &str, destination: &Path) -> Result<(), ShellError> {
    let entries = read_tar(archive)?;
    let first = entries.first().ok_or_else(|| archive_error("the archive was empty"))?;
    if first.name.trim_end_matches('/') != root {
        return Err(archive_error("the archive does not carry the entry it was asked for"));
    }
    match first.kind {
        b'5' => std::fs::create_dir_all(destination).map_err(|error| local_failure("the download", root, &error))?,
        b'0' => {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(|error| local_failure("the download", root, &error))?;
            }
        }
        _ => {
            return Err(archive_error(
                "the entry is not a file or a folder, and is not downloaded",
            ));
        }
    }
    let mut extracted: HashMap<String, PathBuf> = HashMap::new();
    for entry in &entries {
        let relative = entry_path(&entry.name, root)?;
        let local = if relative.is_empty() {
            destination.to_path_buf()
        } else {
            destination.join(&relative)
        };
        match entry.kind {
            b'5' => {
                if relative.is_empty() {
                    std::fs::create_dir_all(&local).map_err(|error| local_failure("the download", root, &error))?;
                } else {
                    create_directories(destination, &local, &entry.name)?;
                }
            }
            b'0' => {
                // The root file's parent was created before the loop; only a child's parent is
                // below `destination`.
                if !relative.is_empty()
                    && let Some(parent) = local.parent()
                {
                    create_directories(destination, parent, &entry.name)?;
                }
                refuse_existing_symlink(&local, &entry.name)?;
                std::fs::write(&local, entry.data)
                    .map_err(|error| local_failure("the download", &entry.name, &error))?;
                set_local_mode(&local, entry.mode);
            }
            b'2' => {
                // The check runs on every platform, so an archive that points outside the chosen
                // folder is refused the same way everywhere; whether the link is then created is
                // the platform's business.
                if !safe_link_target(&entry.link) {
                    return Err(archive_error(format!(
                        "{} points outside the folder it was downloaded into",
                        entry.name
                    )));
                }
                extract_symlink(destination, &local, &entry.link, &entry.name)?;
            }
            b'1' => {
                // A hard link is a second name for bytes already written; copying them keeps the
                // tree usable on a filesystem that has no such idea (or is not this machine's).
                if let Some(source) = extracted.get(entry.link.trim_end_matches('/')) {
                    std::fs::copy(source, &local)
                        .map_err(|error| local_failure("the download", &entry.name, &error))?;
                }
            }
            _ => {}
        }
        extracted.insert(entry.name.trim_end_matches('/').to_owned(), local);
    }
    Ok(())
}

/// The entry's path relative to the archive root, or `""` for the root itself. A tar may carry any
/// name, including one that walks out of the folder the person chose, so this is the check that
/// decides what may be written at all.
fn entry_path(name: &str, root: &str) -> Result<String, ShellError> {
    let name = name.trim_end_matches('/');
    if name == root {
        return Ok(String::new());
    }
    let relative = name
        .strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| archive_error(format!("the archive carries {name}, which is outside {root}")))?;
    for component in relative.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(archive_error(format!("the archive carries a relative name: {name}")));
        }
        // A backslash and a colon are ordinary characters inside a container and path separators or
        // a drive on Windows; refusing both keeps the check the same on every machine.
        if component.contains('\\') || component.contains(':') || component.chars().any(char::is_control) {
            return Err(archive_error(format!(
                "the archive carries a name this build will not write: {name}"
            )));
        }
    }
    Ok(relative.to_owned())
}

/// Creates `target` below `root`, refusing to walk through a link. A tar can carry
/// `dir -> /` followed by `dir/etc/passwd`; creating the second through the first is how an
/// archive writes outside the folder it was downloaded into.
fn create_directories(root: &Path, target: &Path, entry_name: &str) -> Result<(), ShellError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| archive_error("an entry lands outside the folder it was downloaded into"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(archive_error(format!("{entry_name} would be written through a link")));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(archive_error(format!(
                    "{entry_name} needs a folder where a file already is"
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| local_failure("the download", entry_name, &error))?;
            }
            Err(error) => return Err(local_failure("the download", entry_name, &error)),
        }
    }
    Ok(())
}

fn refuse_existing_symlink(local: &Path, entry_name: &str) -> Result<(), ShellError> {
    match std::fs::symlink_metadata(local) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(archive_error(format!(
            "{entry_name} would be written through an existing link"
        ))),
        _ => Ok(()),
    }
}

/// A link is created with its own target; the target was already checked to stay inside the tree.
#[cfg(unix)]
fn extract_symlink(root: &Path, local: &Path, target: &str, entry_name: &str) -> Result<(), ShellError> {
    if let Some(parent) = local.parent() {
        create_directories(root, parent, entry_name)?;
    }
    std::os::unix::fs::symlink(target, local).map_err(|error| local_failure("the download", entry_name, &error))
}

#[cfg(not(unix))]
fn extract_symlink(_root: &Path, _local: &Path, _target: &str, entry_name: &str) -> Result<(), ShellError> {
    log::warn!("skipping the link {entry_name}: links are not created on this platform");
    Ok(())
}

fn safe_link_target(target: &str) -> bool {
    !target.is_empty()
        && !target.starts_with('/')
        && !target.contains('\\')
        && !target.contains(':')
        && !target.chars().any(char::is_control)
        && !target.split('/').any(|component| component == "..")
}

#[cfg(unix)]
fn set_local_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777));
}

#[cfg(not(unix))]
fn set_local_mode(_path: &Path, _mode: u32) {}

/// An error about a local file. The entry's own name travels; the path it is being written to does
/// not, because the interface is never told where files on this machine are.
fn local_failure(operation: &str, name: &str, error: &std::io::Error) -> ShellError {
    ShellError::container_command_failed(operation, format!("{name}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    use sevenshell_error::ErrorCode;

    use super::*;

    // --- A recorder standing where a runtime would be -------------------------

    #[derive(Default)]
    struct FakeTransport {
        calls: Mutex<Vec<Vec<String>>>,
        inputs: Mutex<Vec<Vec<u8>>>,
        reply: String,
        fail: bool,
    }

    impl FakeTransport {
        fn with(reply: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                reply: reply.into(),
                ..Self::default()
            })
        }

        fn failing() -> Arc<Self> {
            Arc::new(Self {
                fail: true,
                ..Self::default()
            })
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }

        fn inputs(&self) -> Vec<Vec<u8>> {
            self.inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl CommandTransport for FakeTransport {
        async fn run(&self, argv: &[String]) -> Result<String, ShellError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(argv.to_vec());
            if self.fail {
                return Err(ShellError::container_command_failed(
                    "the container command",
                    "the engine refused",
                ));
            }
            Ok(self.reply.clone())
        }

        async fn run_stdin(&self, argv: &[String], input: Vec<u8>) -> Result<String, ShellError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(argv.to_vec());
            self.inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(input);
            if self.fail {
                return Err(ShellError::container_command_failed(
                    "the container command",
                    "the engine refused",
                ));
            }
            Ok(self.reply.clone())
        }
    }

    fn files(transport: &Arc<FakeTransport>) -> ContainerFiles {
        ContainerFiles {
            runtime: ContainerRuntime::Docker,
            namespace: String::new(),
            container: "cid".to_owned(),
            transport: Arc::clone(transport) as Arc<dyn CommandTransport>,
        }
    }

    fn script(call: &[String]) -> &str {
        call.last().expect("a command has a script")
    }

    // --- Temporary folders ----------------------------------------------------

    static TEMP_SEQUENCE: AtomicU32 = AtomicU32::new(0);

    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("7shell-container-files-{}-{sequence}", std::process::id()));
            std::fs::create_dir_all(&path).expect("a temporary folder");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // --- Base64 fixtures ------------------------------------------------------

    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::new();
        for chunk in bytes.chunks(3) {
            let first = u32::from(chunk[0]);
            let second = chunk.get(1).copied().map_or(0, u32::from);
            let third = chunk.get(2).copied().map_or(0, u32::from);
            encoded.push(char::from(ALPHABET[(first >> 2) as usize]));
            encoded.push(char::from(ALPHABET[((first & 3) << 4 | second >> 4) as usize]));
            if chunk.len() > 1 {
                encoded.push(char::from(ALPHABET[((second & 15) << 2 | third >> 6) as usize]));
            } else {
                encoded.push('=');
            }
            if chunk.len() > 2 {
                encoded.push(char::from(ALPHABET[(third & 63) as usize]));
            } else {
                encoded.push('=');
            }
        }
        encoded
    }

    fn archive_of(build: impl FnOnce(&mut TarBuilder)) -> Vec<u8> {
        let mut builder = TarBuilder::new();
        build(&mut builder);
        builder.finish()
    }

    // --- Listing --------------------------------------------------------------

    const LISTING: &str = "/srv/app\r\n\
total 24\r\n\
drwxr-xr-x    2 root     root          4096 Nov 10 09:15 assets\r\n\
-rw-r--r--    1 root     root            42 Nov 10 09:15 index.html\r\n\
lrwxrwxrwx    1 root     root             9 Nov 10 09:15 current -> releases/v2\r\n\
-rw-r--r--    1 1000     1000            12 2026-11-10 10:11 iso.txt\r\n\
-rw-r--r--    1 root     root            12 Nov 10  2020 report -> final.txt\r\n\
prw-r--r--    1 root     root             0 Nov 10 09:15 pipe\r\n\
-rw-r--r--    1 root     root             5 Nov 10 09:15   leading.txt\r\n";

    #[tokio::test]
    async fn a_listing_reads_the_resolved_path_and_every_field_ls_printed() {
        let transport = FakeTransport::with(LISTING);
        let listing = files(&transport).list("/srv/current").await.unwrap();

        assert_eq!(listing.path, "/srv/app", "the resolved path is what breadcrumbs show");
        assert_eq!(listing.entries.len(), 7, "{:?}", listing.entries);
        assert_eq!(
            script(&transport.calls()[0]),
            "cd '/srv/current' && pwd -P && LC_ALL=C ls -la",
            "the script changes directory and asks for the physical path"
        );

        let assets = &listing.entries[0];
        assert_eq!(assets.name, "assets");
        assert_eq!(assets.kind, EntryKind::Directory);
        assert_eq!(assets.mode, Some(0o755));
        assert_eq!(assets.size, 4096);
        assert_eq!(assets.path, "/srv/app/assets");
        assert_eq!(assets.owner.as_deref(), Some("root"));

        let index = &listing.entries[1];
        assert_eq!(index.kind, EntryKind::File);
        assert_eq!(index.size, 42);

        let current = &listing.entries[2];
        assert_eq!(current.kind, EntryKind::Symlink);
        assert_eq!(current.link_target.as_deref(), Some("releases/v2"));

        // An ISO date carries its own year; this is 2026-11-10 10:11 UTC.
        assert_eq!(listing.entries[3].modified_at, Some(1_794_305_460));

        // A classic date with a year is midnight on that day, and a regular file named with an
        // arrow keeps the whole name.
        let report = &listing.entries[4];
        assert_eq!(report.name, "report -> final.txt");
        assert_eq!(report.kind, EntryKind::File);
        assert_eq!(report.modified_at, Some(1_604_966_400));

        assert_eq!(listing.entries[5].kind, EntryKind::Other);
    }

    #[test]
    fn a_recent_classic_date_is_read_with_this_machines_year() {
        let answer = "/srv\n-rw-r--r-- 1 root root 5 Nov 10 09:15 now.txt\n";
        let listing = parse_listing("/srv", answer);
        let entry = &listing.entries[0];
        assert_eq!(entry.name, "now.txt");
        assert!(entry.modified_at.is_some(), "a recent date still has a timestamp");
    }

    #[test]
    fn a_name_with_leading_spaces_keeps_them() {
        let listing = parse_listing("/srv", "/srv\n-rw-r--r-- 1 root root 5 Nov 10 09:15   leading.txt\n");
        assert_eq!(listing.entries[0].name, "  leading.txt");
    }

    #[tokio::test]
    async fn a_directory_name_with_a_quote_or_a_substitution_arrives_quoted() {
        let transport = FakeTransport::with("/tmp\n");
        files(&transport).list("/tmp/it's $(id)").await.unwrap();
        assert_eq!(
            script(&transport.calls()[0]),
            "cd '/tmp/it'\\''s $(id)' && pwd -P && LC_ALL=C ls -la"
        );
    }

    #[tokio::test]
    async fn nerdctl_takes_its_namespace_before_the_exec_subcommand() {
        let transport = FakeTransport::with("/\n");
        let files = ContainerFiles {
            runtime: ContainerRuntime::Nerdctl,
            namespace: "k8s.io".to_owned(),
            container: "cid".to_owned(),
            transport: Arc::clone(&transport) as Arc<dyn CommandTransport>,
        };
        files.list("/").await.unwrap();
        assert_eq!(
            transport.calls()[0],
            vec![
                "nerdctl",
                "--namespace",
                "k8s.io",
                "exec",
                "cid",
                "sh",
                "-c",
                "cd '/' && pwd -P && LC_ALL=C ls -la"
            ]
        );
    }

    // --- Refusals -------------------------------------------------------------

    #[tokio::test]
    async fn paths_that_could_leave_the_container_are_refused_before_anything_runs() {
        let transport = FakeTransport::with("");
        let files = files(&transport);
        for path in ["", "relative", "/a/../b", "/a/./b", "/a//b", "/a\nb", " /a"] {
            assert!(files.list(path).await.is_err(), "list accepted {path:?}");
            assert!(files.create_dir(path).await.is_err(), "create_dir accepted {path:?}");
            assert!(files.create_file(path).await.is_err(), "create_file accepted {path:?}");
            assert!(files.read_text(path).await.is_err(), "read_text accepted {path:?}");
            assert!(
                files.write_text(path, "x").await.is_err(),
                "write_text accepted {path:?}"
            );
            assert!(files.rename(path, "/ok").await.is_err(), "rename accepted {path:?}");
            assert!(files.copy(path, "/ok").await.is_err(), "copy accepted {path:?}");
            assert!(files.set_mode(path, 0o644).await.is_err(), "set_mode accepted {path:?}");
            assert!(
                files.download(path, Path::new("x")).await.is_err(),
                "download accepted {path:?}"
            );
        }
        assert!(
            transport.calls().is_empty(),
            "a refused path reached the transport: {:?}",
            transport.calls()
        );
    }

    #[tokio::test]
    async fn the_root_directory_is_never_removed_copied_renamed_or_chmodded() {
        let transport = FakeTransport::with("");
        let files = files(&transport);
        assert!(files.remove("/", true).await.is_err());
        assert!(files.copy("/", "/x").await.is_err());
        assert!(files.rename("/", "/x").await.is_err());
        assert!(files.set_mode("/", 0o644).await.is_err());
        assert!(files.create_file("/").await.is_err());
        assert!(files.read_text("/").await.is_err());
        assert!(files.write_text("/", "x").await.is_err());
        assert!(files.download("/", Path::new("x")).await.is_err());
        assert!(transport.calls().is_empty(), "{:?}", transport.calls());
    }

    #[test]
    fn a_container_id_that_could_be_an_option_is_refused() {
        for container in ["", "  ", "-x", "--help", "a\nb"] {
            assert!(
                ContainerFiles::new(ContainerRuntime::Docker, "", container, Runner::Local).is_err(),
                "{container:?} was accepted"
            );
        }
        for namespace in ["--namespace", "a/b"] {
            assert!(
                ContainerFiles::new(ContainerRuntime::Docker, namespace, "cid", Runner::Local).is_err(),
                "{namespace:?} was accepted"
            );
        }
    }

    #[tokio::test]
    async fn a_failed_command_keeps_its_error_instead_of_becoming_an_empty_listing() {
        let transport = FakeTransport::failing();
        let error = files(&transport).list("/").await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
    }

    // --- Metadata commands ----------------------------------------------------

    #[tokio::test]
    async fn the_metadata_commands_are_one_quoted_script_each() {
        let transport = FakeTransport::with("");
        let files = files(&transport);

        files.create_dir("/srv/my app").await.unwrap();
        files.create_file("/srv/my app/empty").await.unwrap();
        files.rename("/srv/a", "/srv/b c").await.unwrap();
        files.copy("/srv/a b", "/srv/c").await.unwrap();
        files.remove("/srv/a b", false).await.unwrap();
        files.remove("/srv/a b", true).await.unwrap();
        files.set_mode("/srv/a b", 0o750).await.unwrap();
        files.set_mode("/srv/a b", 0o4755).await.unwrap();

        let calls = transport.calls();
        let scripts: Vec<&str> = calls.iter().map(|call| script(call)).collect();
        assert_eq!(
            scripts,
            vec![
                "mkdir -p '/srv/my app'",
                "touch '/srv/my app/empty'",
                "mv '/srv/a' '/srv/b c'",
                "cp -a '/srv/a b' '/srv/c'",
                "rm -f '/srv/a b'",
                "rm -rf '/srv/a b'",
                "chmod 0750 '/srv/a b'",
                "chmod 4755 '/srv/a b'",
            ]
        );
        assert!(
            calls.iter().all(|call| call[..2] == ["docker", "exec"]),
            "every file command runs through the runtime's exec: {calls:?}"
        );
    }

    #[tokio::test]
    async fn set_mode_refuses_a_number_that_is_not_permissions() {
        let transport = FakeTransport::with("");
        assert!(files(&transport).set_mode("/srv/a", 0o10000).await.is_err());
        assert!(transport.calls().is_empty());
    }

    // --- Reading and writing text --------------------------------------------

    #[tokio::test]
    async fn write_text_sends_one_file_archive_on_standard_input() {
        let transport = FakeTransport::with("");
        files(&transport)
            .write_text("/etc/app/config file.txt", "hello")
            .await
            .unwrap();
        let calls = transport.calls();
        assert_eq!(
            calls[0],
            vec![
                "docker",
                "exec",
                "-i",
                "cid",
                "sh",
                "-c",
                "mkdir -p '/etc/app' && tar xf - -C '/etc/app'"
            ],
            "an upload needs a standard input, so the exec takes -i"
        );
        let archive = transport.inputs().remove(0);
        let entries = read_tar(&archive).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "config file.txt");
        assert_eq!(entries[0].kind, b'0');
        assert_eq!(entries[0].mode, 0o644);
        assert_eq!(entries[0].data, b"hello");
    }

    #[tokio::test]
    async fn write_text_refuses_more_than_the_editor_limit() {
        let transport = FakeTransport::with("");
        let content = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(files(&transport).write_text("/a.txt", &content).await.is_err());
        assert!(transport.calls().is_empty());
    }

    #[tokio::test]
    async fn read_text_returns_the_file_the_archive_carried() {
        let archive = archive_of(|builder| {
            builder.file("x.txt", 0o644, 0, b"first line\n").unwrap();
        });
        let transport = FakeTransport::with(base64(&archive));
        let text = files(&transport).read_text("/etc/app/x.txt").await.unwrap();
        assert_eq!(text, "first line\n");
        assert_eq!(script(&transport.calls()[0]), "tar cf - -C '/etc/app' 'x.txt' | base64");
    }

    #[tokio::test]
    async fn read_text_refuses_what_is_not_utf8_text() {
        let archive = archive_of(|builder| {
            builder.file("x.bin", 0o644, 0, &[0xff, 0xfe, 0x00]).unwrap();
        });
        let transport = FakeTransport::with(base64(&archive));
        let error = files(&transport).read_text("/etc/x.bin").await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        assert!(error.detail().unwrap_or_default().contains("UTF-8"));
    }

    #[tokio::test]
    async fn read_text_refuses_a_file_over_the_editor_limit() {
        let archive = archive_of(|builder| {
            builder
                .file("big.txt", 0o644, 0, &vec![b'x'; MAX_TEXT_BYTES + 1])
                .unwrap();
        });
        let transport = FakeTransport::with(base64(&archive));
        let error = files(&transport).read_text("/etc/big.txt").await.unwrap_err();
        assert!(error.detail().unwrap_or_default().contains("limit"));
    }

    #[tokio::test]
    async fn read_text_refuses_a_directory() {
        let archive = archive_of(|builder| {
            builder.directory("etc", 0o755, 0).unwrap();
        });
        let transport = FakeTransport::with(base64(&archive));
        assert!(files(&transport).read_text("/etc").await.is_err());
    }

    // --- Transfers ------------------------------------------------------------

    #[tokio::test]
    async fn download_writes_the_tree_the_archive_carried() {
        let archive = archive_of(|builder| {
            builder.directory("app", 0o755, 0).unwrap();
            builder.file("app/index.html", 0o644, 0, b"<h1>hi</h1>\n").unwrap();
            builder.directory("app/assets", 0o755, 0).unwrap();
            builder.file("app/assets/app.css", 0o644, 0, b"body {}\n").unwrap();
            builder.file("app/binary.bin", 0o600, 0, &[0x00, 0x01, 0xff]).unwrap();
        });
        let transport = FakeTransport::with(base64(&archive));
        let temp = TempTree::new();
        let destination = temp.path().join("downloaded");
        files(&transport).download("/srv/app", &destination).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("index.html")).unwrap(),
            "<h1>hi</h1>\n"
        );
        assert_eq!(
            std::fs::read(destination.join("binary.bin")).unwrap(),
            vec![0x00, 0x01, 0xff]
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("assets/app.css")).unwrap(),
            "body {}\n"
        );
        assert_eq!(script(&transport.calls()[0]), "tar cf - -C '/srv' 'app' | base64");
    }

    #[tokio::test]
    async fn download_of_a_single_file_writes_the_destination_path_itself() {
        let archive = archive_of(|builder| {
            builder.file("notes.txt", 0o640, 0, b"one\n").unwrap();
        });
        let transport = FakeTransport::with(base64(&archive));
        let temp = TempTree::new();
        let destination = temp.path().join("folder/notes.txt");
        files(&transport)
            .download("/srv/notes.txt", &destination)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"one\n");
        assert_eq!(script(&transport.calls()[0]), "tar cf - -C '/srv' 'notes.txt' | base64");
    }

    #[tokio::test]
    async fn a_download_entry_that_leaves_the_destination_is_refused() {
        for escaping in ["app/../../evil", "app/../evil", "app\\evil"] {
            let archive = archive_of(|builder| {
                builder.directory("app", 0o755, 0).unwrap();
                builder.file(escaping, 0o644, 0, b"x").unwrap();
            });
            let transport = FakeTransport::with(base64(&archive));
            let temp = TempTree::new();
            let destination = temp.path().join("app");
            let error = files(&transport).download("/srv/app", &destination).await.unwrap_err();
            assert_eq!(error.code(), ErrorCode::ContainerCommandFailed, "{escaping}");
            assert!(
                !temp.path().join("evil").exists(),
                "{escaping} was written outside the destination"
            );
        }
    }

    #[tokio::test]
    async fn an_archive_that_does_not_carry_the_asked_entry_is_refused() {
        let archive = archive_of(|builder| {
            builder.directory("other", 0o755, 0).unwrap();
        });
        let transport = FakeTransport::with(base64(&archive));
        let temp = TempTree::new();
        let error = files(&transport)
            .download("/srv/app", &temp.path().join("app"))
            .await
            .unwrap_err();
        assert!(error.detail().unwrap_or_default().contains("does not carry"));
    }

    #[tokio::test]
    async fn an_empty_archive_is_an_error() {
        let transport = FakeTransport::with("");
        let temp = TempTree::new();
        assert!(
            files(&transport)
                .download("/srv/app", &temp.path().join("app"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn upload_packs_the_tree_and_extracts_it_in_the_destination() {
        let temp = TempTree::new();
        let root = temp.path().join("site");
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), b"<h1>hi</h1>\n").unwrap();
        std::fs::write(root.join("assets/app.css"), b"body {}\n").unwrap();

        let transport = FakeTransport::with("");
        files(&transport).upload(&root, "/srv/app").await.unwrap();
        assert_eq!(
            transport.calls()[0],
            vec![
                "docker",
                "exec",
                "-i",
                "cid",
                "sh",
                "-c",
                "mkdir -p '/srv/app' && tar xf - -C '/srv/app'"
            ]
        );
        let archive = transport.inputs().remove(0);
        let entries = read_tar(&archive).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["site/", "site/assets/", "site/assets/app.css", "site/index.html"],
            "the folder's own name is the archive root, so the tree lands as it was selected"
        );
        assert_eq!(entries[0].kind, b'5');
        assert_eq!(entries[2].data, b"body {}\n");
        assert_eq!(entries[3].data, b"<h1>hi</h1>\n");
    }

    #[tokio::test]
    async fn upload_of_one_file_names_it_after_itself() {
        let temp = TempTree::new();
        let file = temp.path().join("notes.txt");
        std::fs::write(&file, b"one\n").unwrap();
        let transport = FakeTransport::with("");
        files(&transport).upload(&file, "/srv/app").await.unwrap();
        let archive = transport.inputs().remove(0);
        let entries = read_tar(&archive).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "notes.txt");
        assert_eq!(entries[0].kind, b'0');
        assert_eq!(entries[0].data, b"one\n");
    }

    #[tokio::test]
    async fn an_upload_of_a_path_that_does_not_exist_is_refused() {
        let transport = FakeTransport::with("");
        let error = files(&transport)
            .upload(Path::new("no-such-file-anywhere.txt"), "/srv")
            .await
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContainerCommandFailed);
        assert!(transport.calls().is_empty());
    }

    // --- The archive format ---------------------------------------------------

    #[test]
    fn a_tar_name_longer_than_the_short_field_travels_in_the_prefix() {
        let long = format!("{}/{}", "a".repeat(100), "b".repeat(100));
        let archive = archive_of(|builder| {
            builder.file(&long, 0o644, 0, b"x").unwrap();
        });
        let entries = read_tar(&archive).unwrap();
        assert_eq!(entries[0].name, long);
        assert_eq!(entries[0].data, b"x");
    }

    #[test]
    fn a_path_that_cannot_fit_a_ustar_header_is_refused() {
        let too_long = format!("{}/{}", "a".repeat(160), "b".repeat(100));
        assert!(split_tar_name(&too_long).is_err());
    }

    #[test]
    fn a_truncated_archive_is_an_error_not_a_short_tree() {
        let archive = archive_of(|builder| {
            builder.file("x.bin", 0o644, 0, &[7u8; 300]).unwrap();
        });
        let truncated = &archive[..TAR_BLOCK + 100];
        let error = read_tar(truncated).err().expect("a truncated archive is not a listing");
        assert!(error.detail().unwrap_or_default().contains("middle"));
    }

    #[test]
    fn a_corrupted_header_is_refused() {
        let mut archive = archive_of(|builder| {
            builder.file("x.txt", 0o644, 0, b"x").unwrap();
        });
        archive[0] = b'z';
        assert!(read_tar(&archive).is_err());
    }

    #[test]
    fn base64_whose_lines_wrap_is_decoded() {
        assert_eq!(decode_base64("aGVs\nbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("").unwrap(), Vec::<u8>::new());
        assert!(decode_base64("aGVsbG8*").is_err());
        assert!(decode_base64("aGVsbG8").is_err(), "an incomplete group is not text");
    }

    #[test]
    fn permissions_are_read_out_of_the_mode_column() {
        assert_eq!(permission_bits("-rw-r--r--"), Some(0o644));
        assert_eq!(permission_bits("drwxr-x---"), Some(0o750));
        assert_eq!(permission_bits("-rwsr-xr-t"), Some(0o5755));
        assert_eq!(permission_bits("-rw-r--r--."), Some(0o644));
        assert_eq!(permission_bits("rw-r--r--"), None);
    }

    #[test]
    fn the_conversion_between_civil_dates_and_epoch_days_is_stable() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 1, 1), 10_957);
        assert_eq!(days_from_civil(2026, 11, 10), 20_767);
        assert_eq!(civil_from_days(20_767), (2026, 11, 10));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(timestamp(2026, 11, 10, 10, 11, 0), Some(1_794_305_460));
        assert_eq!(timestamp(2026, 13, 1, 0, 0, 0), None);
    }

    #[test]
    fn a_transfer_larger_than_the_bound_is_refused() {
        assert!(ensure_transfer_fits(MAX_TRANSFER_BYTES).is_ok());
        assert!(ensure_transfer_fits(MAX_TRANSFER_BYTES + 1).is_err());
    }

    #[test]
    fn a_link_target_that_climbs_out_of_the_folder_is_not_written() {
        assert!(safe_link_target("releases/v2"));
        for target in [
            "/etc/passwd",
            "../sibling",
            "..",
            "../../etc/passwd",
            "a\\b",
            "C:window",
            "",
        ] {
            assert!(!safe_link_target(target), "{target:?} was accepted");
        }
    }

    #[test]
    fn an_entry_name_this_build_will_not_write_is_refused() {
        assert_eq!(entry_path("app/index.html", "app").unwrap(), "index.html");
        assert_eq!(entry_path("app/", "app").unwrap(), "");
        for name in ["app/../x", "app/./x", "other/x", "app/a\\b", "app/a:b"] {
            assert!(entry_path(name, "app").is_err(), "{name:?} was accepted");
        }
    }
}

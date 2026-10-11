//! What the machine on the other end is, asked once when a connection opens.
//!
//! The session list draws a mark beside a host, and the only thing that can say which mark is the
//! host itself. So this runs one script over the connection that is already open, reads what the
//! far side keeps about itself, and stops: one round trip per connection, read-only, nothing
//! repeating. It is deliberately not part of [`crate::monitor`]: a sample is polled while a panel
//! is on screen and describes a moment, and this is asked once and describes the machine.
//!
//! Two things about the script are not style:
//!
//! * **The markers are unquoted.** `echo @@uname` prints `@@uname` under a POSIX shell and under
//!   `cmd.exe` alike, while a quoted marker keeps its quotes on Windows and stops being a marker.
//! * **`ver` is asked, and asked last.** A Windows host answers no `uname` and keeps no
//!   `/etc/os-release`, so a probe of only those two would leave the one system that has a mark of
//!   its own looking exactly like a network appliance. On every host that is not Windows the
//!   command fails into stderr, which nothing here reads.
//!
//! No answer is an error. A host that will not say what it is keeps the mark it had — the plain
//! terminal one — and that is the whole of the consequence, which is why nothing here reports a
//! failure upward: the interface has nothing to do with an empty answer but draw it.

use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::HostSystem;

use crate::Core;
use crate::monitor::{first, groups, os_release_value};

/// What the identity probe runs on the far side.
///
/// Written without redirections on purpose: stdout and stderr arrive as separate streams, so the
/// errors of a host that does not know these commands are already somebody else's problem, and
/// `2>/dev/null` is the one piece of this that a Windows shell would read as a *file* named
/// `/dev/null` in the current drive's root.
const SYSTEM_COMMAND: &str = "\
echo @@uname
uname -s
echo @@os
cat /etc/os-release
echo @@windows
ver
";

/// How long the probe may take.
///
/// Shorter than a sample's window, because there is less to wait for: one word and one small file,
/// asked while somebody waits for a row to draw. A host slower than this is a host that keeps the
/// plain terminal mark, which is a smaller loss than a connection that looks like it is hanging.
const SYSTEM_TIMEOUT: Duration = Duration::from_secs(5);

impl Core {
    /// Asks the host behind a terminal what it is.
    ///
    /// By handle rather than by session: the question rides a connection that is up, and the
    /// registry is the only thing that knows one is. A terminal that has already ended is refused
    /// exactly as a handle that never existed is, and the interface draws the row without a mark.
    pub async fn host_system(&self, handle: &str) -> Result<HostSystem, ShellError> {
        let session = self.registry.session_of(handle).await?;
        let outcome = session.exec(SYSTEM_COMMAND, SYSTEM_TIMEOUT).await?;

        Ok(parse_system(&outcome.stdout))
    }
}

/// The identity an answer describes.
///
/// Pure, and split out from the call above so the answers this has to survive — an `os-release`
/// with quoting and `ID_LIKE`, a BSD that keeps no such file, a Windows `ver` line, a host that
/// answered nothing at all — can be tested against text rather than against a lab full of machines.
fn parse_system(stdout: &str) -> HostSystem {
    let groups = groups(stdout);

    let kernel = text(first(&groups, "uname"));
    let lines = groups.get("os").map(Vec::as_slice).unwrap_or(&[]);

    // The one system that has neither of the usual two answers. Read as an identity rather than as
    // a name, because `windows` is what the interface can key a mark on and the line `ver` printed
    // is what a person reads.
    let windows = first(&groups, "windows").filter(|line| line.to_ascii_lowercase().contains("windows"));

    // The host's own id, lower case the way `/etc/os-release` publishes it: a file that says
    // `ID=Ubuntu` and a person who types `ubuntu` mean the same system.
    let distro = os_release_value(lines, "ID")
        .map(|id| id.to_ascii_lowercase())
        .or_else(|| windows.map(|_| "windows".to_owned()));

    let distro_like = os_release_value(lines, "ID_LIKE")
        .map(|list| list.split_whitespace().map(|word| word.to_ascii_lowercase()).collect())
        .unwrap_or_default();

    // What a person reads, in the order the answers are worth reading: the distribution's own
    // pretty name, the kernel's name for a host that keeps no os-release, and the `ver` line for
    // the host that has neither.
    let name = os_release_value(lines, "PRETTY_NAME")
        .or_else(|| kernel.clone())
        .or_else(|| windows.map(str::to_owned));

    HostSystem {
        name,
        kernel,
        distro,
        distro_like,
    }
}

/// One group's line as a value: white space trimmed off, nothing left of an empty line.
fn text(line: Option<&str>) -> Option<String> {
    line.map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real answer from a Linux host: `uname` says what the kernel is called, and
    /// the distribution introduces itself in a file of quoted `KEY=value` lines.
    const UBUNTU: &str = "\
@@uname
Linux
@@os
PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"
NAME=\"Ubuntu\"
VERSION_ID=\"24.04\"
ID=ubuntu
ID_LIKE=debian
@@windows
";

    #[test]
    fn a_distribution_is_read_by_id_and_named_by_its_pretty_name() {
        let system = parse_system(UBUNTU);

        assert_eq!(system.kernel.as_deref(), Some("Linux"));
        assert_eq!(system.distro.as_deref(), Some("ubuntu"), "ID is the id");
        assert_eq!(system.name.as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(system.distro_like, ["debian"]);
    }

    #[test]
    fn an_id_keeps_the_spelling_of_the_file_and_not_the_hosts_shouting() {
        // Nothing in the file format promises lower case, and a table of marks is keyed on the
        // spelling every distribution publishes.
        let stdout = "@@uname\nLinux\n@@os\nID=Ubuntu\n@@windows\n";

        assert_eq!(parse_system(stdout).distro.as_deref(), Some("ubuntu"));
    }

    #[test]
    fn id_like_is_read_as_the_list_it_is() {
        let stdout = "@@uname\nLinux\n@@os\nID=centos\nID_LIKE=\"rhel fedora\"\n@@windows\n";
        let system = parse_system(stdout);

        assert_eq!(system.distro.as_deref(), Some("centos"));
        assert_eq!(system.distro_like, ["rhel", "fedora"], "in the order published");
    }

    #[test]
    fn a_host_without_an_os_release_is_named_by_its_kernel() {
        // Every BSD, and every busybox: `uname` answers and the file is simply not there.
        for kernel in ["FreeBSD", "Darwin", "OpenBSD"] {
            let stdout = format!("@@uname\n{kernel}\n@@os\n@@windows\n");
            let system = parse_system(&stdout);

            assert_eq!(system.kernel.as_deref(), Some(kernel));
            assert_eq!(system.name.as_deref(), Some(kernel), "the kernel is the name there is");
            assert_eq!(system.distro, None, "and there is no id to be had");
        }
    }

    #[test]
    fn a_windows_answer_is_read_even_though_uname_and_os_release_said_nothing() {
        let stdout = "@@uname\n@@os\n@@windows\nMicrosoft Windows [Version 10.0.19045.3803]\n";
        let system = parse_system(stdout);

        assert_eq!(system.distro.as_deref(), Some("windows"));
        assert_eq!(
            system.name.as_deref(),
            Some("Microsoft Windows [Version 10.0.19045.3803]")
        );
        assert_eq!(system.kernel, None, "Windows answers no uname, and none is invented");
    }

    #[test]
    fn a_host_that_answered_nothing_has_nothing() {
        // A network appliance, or a shell that does not know any of the commands: the markers
        // arrive and nothing stands between them.
        let system = parse_system("@@uname\n@@os\n@@windows\n");

        assert_eq!(system, HostSystem::default());
        assert_eq!(system.name, None);
        assert!(system.distro_like.is_empty());
    }

    #[test]
    fn an_error_on_the_wrong_stream_is_not_an_identity() {
        // `ver` fails on every host that is not Windows, and one that prints a complaint rather
        // than failing must not be read as a Windows host.
        let stdout = "@@uname\nLinux\n@@os\nID=alpine\n@@windows\nsh: ver: not found\n";
        let system = parse_system(stdout);

        assert_eq!(system.distro.as_deref(), Some("alpine"));
        assert_eq!(system.name.as_deref(), Some("Linux"));
    }

    #[test]
    fn an_empty_id_is_not_an_id() {
        // A file that keeps the key and leaves it blank has said as little as one that omits it.
        let stdout = "@@uname\nLinux\n@@os\nID=\nPRETTY_NAME=\"\"\n@@windows\n";
        let system = parse_system(stdout);

        assert_eq!(system.distro, None);
        assert_eq!(system.name.as_deref(), Some("Linux"), "the kernel is still an answer");
    }
}

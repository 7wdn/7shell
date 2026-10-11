//! What the host on the other end is, as the interface reads it.
//!
//! This is the machine's own account of itself, asked once when a connection opens. It is not a
//! sample (`HostMetrics`, next door): a sample is polled while a panel is on screen and describes a
//! moment, while the mark beside a session is about *which machine this is* — an answer that does
//! not change while somebody works on it, and that nothing has to keep asking for.
//!
//! Every field is optional for the same reason it is in a sample: the answers come from commands run
//! on the far side, and the far side is not always the same operating system. A BSD keeps no
//! `/etc/os-release`, a Windows host answers no `uname` at all, and a host that answered one of the
//! questions has still said something worth drawing. `None` means "this host did not say".
//!
//! What is deliberately not here is any decision about which picture to draw. The host says what it
//! calls itself; which mark stands for that — and whether this build has one — belongs to the
//! interface, which owns the pictures. A backend that chose from a table it keeps would be handing
//! the interface a decision it cannot check.

use serde::{Deserialize, Serialize};

/// What one host said it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSystem {
    /// The name a person reads: `PRETTY_NAME` from `/etc/os-release` when the host keeps one, the
    /// kernel's own name when it does not, and on Windows the line `ver` answered with.
    pub name: Option<String>,
    /// The kernel's own name — the answer to `uname -s`: `Linux`, `Darwin`, `FreeBSD`, and on a
    /// Windows host whose shell is a POSIX one, `MINGW64_NT-10.0`.
    pub kernel: Option<String>,
    /// What the host calls its system, in lower case: `ID=` from `/etc/os-release`, and `windows`
    /// on the host that keeps no such file but answered `ver`.
    pub distro: Option<String>,
    /// What that system says it is like: `ID_LIKE=` from `/etc/os-release`, split on white space
    /// and in the order the host published it (`rhel centos fedora`). Absent on most hosts, and
    /// worth carrying anyway: it is how a derived distribution is recognised as one.
    pub distro_like: Vec<String>,
}

//! One sample of what a host is doing, as the interface reads it.
//!
//! Every field is optional, and that is the design rather than a hedge. A sample is taken by
//! running commands on the far side, and the far side is not always the same operating system:
//! what a Linux host reports from `/proc` is not what a BSD reports from anywhere, and a host
//! that answers some of the questions has still answered something worth showing. `None` means
//! "this host did not say", which a panel draws as a dash — visibly different from a zero, which
//! is a real reading and the one a gauge must never invent.
//!
//! Nothing here is stored: a sample describes the moment it was taken, and the counters that only
//! make sense as a difference — CPU ticks, bytes through an interface — are carried as they came
//! off the host. Whoever holds two samples computes the rate between them, so the interface can
//! show a live reading without this crate inventing a clock or a history.

use serde::{Deserialize, Serialize};

/// How many mounted file systems a sample carries.
pub const MAX_DISKS: usize = 16;

/// How many processes a sample carries. The panel's table is five rows: enough to see what is
/// busy at a glance. A reader who wants more opens the process page, which asks for its own list —
/// a longer one is a different question, asked at a different rate.
pub const MAX_PROCESSES: usize = 5;

/// The most a process page may ask for.
///
/// A safety valve rather than a page size: the list is cut on the far side because it is sent, and
/// a host with more processes than this answers with the ones its own CPU column rates highest.
/// Anywhere below it — which is every machine this has been run against — the page has the whole
/// table and sorts it itself.
pub const MAX_PAGE_PROCESSES: usize = 400;

/// The most environment variables a process detail carries. A shell with a long history can export
/// thousands of them, and a page that takes a minute to open is not a page anybody reads.
pub const MAX_ENVIRONMENT_ENTRIES: usize = 200;

/// How much of a command line a row keeps. Long enough to tell two `java` processes apart, short
/// enough that two hundred of them are still a payload and not a dump.
pub const MAX_COMMAND_CHARS: usize = 240;

/// How many sockets a page carries.
///
/// A busy host has thousands of established connections and a handful that anybody is looking for.
/// The list is cut with the listening sockets kept first, because those are what the page is about:
/// dropping one of them to make room for a connection in `TIME-WAIT` would be the wrong trade.
pub const MAX_SOCKETS: usize = 512;

/// One reading of a host, taken at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostMetrics {
    /// Milliseconds since the Unix epoch, on the machine that asked. The interface uses it to
    /// date the sample rather than the moment an answer happened to arrive.
    pub sampled_at: u64,
    /// How long the sample took from this side: the round trip plus the commands. A slow link and
    /// a busy host both show up here, which is the honest half of what a "latency" reading means
    /// for something that is not a ping.
    pub round_trip_ms: u64,
    pub hostname: Option<String>,
    /// What the host calls itself: `PRETTY_NAME` from `/etc/os-release`, or the kernel's own name.
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub arch: Option<String>,
    /// How many CPUs the counters cover, as the host counts them.
    pub cores: Option<u32>,
    /// What the host says its processors are, when it says anything.
    pub cpu_model: Option<CpuModel>,
    pub uptime_seconds: Option<f64>,
    pub load: Option<LoadAverage>,
    /// Cumulative CPU ticks, for a rate between two samples.
    pub cpu: Option<CpuCounters>,
    pub memory: Option<MemoryUsage>,
    /// Cumulative interface counters, loopback included: the panel picks the busiest one itself,
    /// and the network page lists them all.
    pub network: Vec<NetworkCounters>,
    pub disks: Vec<DiskUsage>,
    /// The busiest processes, as the host's own `ps` reported them.
    pub processes: Vec<ProcessSample>,
}

/// What the host calls its processor.
///
/// Described rather than measured: these are the strings `/proc/cpuinfo` publishes, and a host
/// that publishes none of them — or publishes them somewhere else, as the BSDs do — has no
/// `cpu_model` at all rather than an invented one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuModel {
    pub name: String,
    /// The nominal clock of the first processor, in megahertz, when the kernel reports one.
    pub frequency_mhz: Option<f64>,
    pub cache_kb: Option<u64>,
    pub bogomips: Option<f64>,
}

/// The three numbers everyone reads a host by.
///
/// Unitless on purpose: they are run-queue lengths, not percentages, and dividing them by the CPU
/// count is a reading the panel does where it can say so.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

/// The kernel's own CPU counters, in whatever unit it counts them.
///
/// Differences and sums are all that may be done with these: the unit is typically hundredths of a
/// second, but nothing here depends on that, because a rate is a ratio of two of these numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuCounters {
    /// Every field of the CPU line added together.
    pub total_ticks: u64,
    /// The part that was idle, including time spent waiting on I/O: a disk-bound host is not a
    /// busy CPU, and reporting it as one sends people looking in the wrong place.
    pub idle_ticks: u64,
    /// The same readings, still split by what the processor was doing. The panel needs the two
    /// numbers above; the CPU page shows this, because "the CPU is at 40%" is a question people
    /// immediately follow with "doing what".
    pub states: CpuStates,
}

/// The columns of the kernel's CPU line, in the kernel's own ticks.
///
/// Raw and unlabelled by percentage on purpose: a share of the interval is a difference between
/// two samples, and this crate does not hold two. Note that `idle_ticks` here is the kernel's own
/// idle column, while [`CpuCounters::idle_ticks`] is idle *plus* waiting on I/O — the two answer
/// different questions and are deliberately not the same field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuStates {
    pub user_ticks: u64,
    pub nice_ticks: u64,
    pub system_ticks: u64,
    pub idle_ticks: u64,
    pub io_wait_ticks: u64,
    pub interrupt_ticks: u64,
    pub soft_interrupt_ticks: u64,
    /// Time taken by the hypervisor this host runs under, when it is a guest.
    pub steal_ticks: u64,
}

/// Memory and swap, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUsage {
    pub total_bytes: u64,
    /// What could still be handed out without swapping. The kernel's own answer where there is
    /// one, because "free" alone counts the page cache as used and reads as a full machine on an
    /// idle one.
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_free_bytes: u64,
}

/// One interface's counters, cumulative since it came up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkCounters {
    pub name: String,
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
}

/// One mounted file system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    /// Where it is mounted: what a person recognises, and what the panel lists.
    pub mount_point: String,
    /// The device as the far side names it, which is what tells two mounts of one size apart.
    pub filesystem: String,
    pub size_bytes: u64,
    pub available_bytes: u64,
}

/// One process, as the far side described it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSample {
    pub pid: u32,
    /// The account the process runs as, which is what tells two services apart on a shared host.
    pub user: String,
    /// The executable's own name — what a process is called, rather than how it was invoked.
    pub name: String,
    /// How it was invoked, capped at [`MAX_COMMAND_CHARS`]. Empty when the host reports no
    /// command line at all, which is the normal case for a kernel thread.
    pub command: String,
    /// Where the program is on disk, when the read asked for it: `None` from a read that did not.
    ///
    /// This is the host's own answer — a link that may point at a file that has since been replaced
    /// or deleted — and it is drawn as a dash rather than as the name of the command when absent.
    pub executable: Option<String>,
    /// Percent of one CPU as `ps` measures it, which is an average since the process started.
    ///
    /// Kept because it is the one CPU figure that comes without a second reading, and because it is
    /// what the host's own column says. It is *not* what a live table shows: a service that has been
    /// idle for a week reads the same number every time it is asked, which is what a frozen page
    /// looks like. The interval is measured from [`ProcessSample::cpu_ticks`] instead, where the
    /// read asked for them.
    pub cpu_percent: f64,
    /// Percent of memory, as `ps` reports it.
    pub memory_percent: f64,
    /// Resident set size in bytes.
    pub resident_bytes: u64,
    /// Cumulative CPU time this process has used, in the kernel's ticks.
    ///
    /// A counter, not a rate: what the processors spent on this process between two reads is the
    /// difference between two of them, which is the same rule the host's own CPU columns follow.
    /// `None` from a read that did not ask.
    pub cpu_ticks: Option<u64>,
}

/// The whole of a process page, as the host answered it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessPage {
    /// How many ticks a second the host's counters are in — its `USER_HZ`, almost always 100.
    ///
    /// Carried rather than assumed: it is what turns a difference of ticks into a percentage, and a
    /// host that counts differently would otherwise make every process on the page look wrong by
    /// the same factor.
    pub ticks_per_second: u32,
    pub processes: Vec<ProcessSample>,
}

/// One process, looked at closely: how it was started, from where, and with what in its environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDetail {
    pub pid: u32,
    pub name: String,
    /// The full command line, joined from the NUL-separated bytes the kernel keeps. Empty for a
    /// kernel thread, which was started without one.
    pub command_line: String,
    pub executable: String,
    /// Where the process's working directory is now, which for a daemon started by an init script
    /// is often somewhere nobody would guess.
    pub working_directory: String,
    /// The environment it was started with, in the order the kernel lists it.
    ///
    /// Read only when somebody opens this page for a process: it can hold credentials, and a
    /// sample that collected every process's environment every two seconds would be gathering
    /// secrets nobody asked for.
    pub environment: Vec<EnvironmentEntry>,
}

/// One variable a process was started with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentEntry {
    pub name: String,
    pub value: String,
}

/// One socket the host has open, listening or connected.
///
/// One row per socket rather than one per service, because that is what the host reports and what
/// the page shows: a daemon listening on IPv4 and IPv6 is two sockets, and a service somebody
/// cannot reach is often a service whose second socket never came up.
///
/// `state` is the host's own word for what the socket is doing, normalised to hyphens, and the
/// interface treats two of them as "listening": `LISTEN` for TCP and `UNCONN` for UDP, which has no
/// listening state because it does not accept anything — a UDP socket that names no peer is
/// waiting for datagrams, which is the same thing to a reader.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SocketRow {
    /// `tcp`, `udp`, or the host's own spelling of a variant of them.
    pub protocol: String,
    pub state: String,
    /// The local address, numeric. `0.0.0.0` or `::` for "every address on this host".
    pub address: String,
    pub port: u16,
    /// The process holding the socket, when the host would say. Zero and empty when it would not:
    /// attributing a socket to a process takes privileges, and a host that refuses is still a host
    /// with a socket on a port.
    pub pid: u32,
    pub process: String,
    /// The other end, empty for a socket that is not connected to anything.
    pub peer_address: String,
    pub peer_port: u16,
    /// What this socket has carried, where the host reports it. `ss` does, `netstat` does not — and
    /// the difference is drawn as a dash rather than as a zero, which would mean "nothing yet".
    pub sent_bytes: Option<u64>,
    pub received_bytes: Option<u64>,
}

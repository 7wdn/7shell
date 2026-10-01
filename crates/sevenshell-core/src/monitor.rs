//! What a host is doing: one command, one sample, no state kept.
//!
//! The panel this feeds is a live view of a machine somebody is already connected to, so the
//! sample rides the connection that is already up — the same connection the terminal is typing
//! into — rather than opening one of its own. That is a deliberate limit: a sample can only be
//! taken for a session that has a terminal open, because the alternative is a fresh handshake
//! every couple of seconds, and a monitor that authenticates in a loop is a worse neighbour than
//! one that says it has nothing to show.
//!
//! Three decisions worth reading before changing anything here:
//!
//! 1. **One command per sample, not one per figure.** A round trip costs more than the reading
//!    does, so everything the panel shows comes back from a single invocation of `/bin/sh`, with
//!    each group of lines introduced by a marker line. The markers are the parse; there is no
//!    second connection to keep in step. There is exactly one exception, and it is by design: the
//!    process page asks for a list of its own ([`Core::list_processes`]), because two hundred
//!    command lines are a different question from five rows beside a terminal, and only the page
//!    that is reading them should pay for them.
//! 2. **The counters are carried, not the rates.** Ticks and cumulative bytes are what the host
//!    actually reported, and a rate needs two of them a known distance apart. This side does not
//!    keep a history per session: whoever holds two samples subtracts them. That also means the
//!    very first sample after a connect has no rate to show, which is true — the alternative is a
//!    CPU percentage invented from a single reading of a counter since boot.
//! 3. **What cannot be read is absent, not zero.** The parsing is written against `/proc`, which
//!    is Linux; another operating system answers some of the questions and not others, and a host
//!    with a `df` that hangs on a stale NFS mount answers none of them. Every section is optional
//!    and a failure to read one is not a failure of the sample: the interface draws a dash where
//!    a number would be, and never a zero that looks like a measurement.
//!
//! The command is read-only: it contains no redirection that writes, and nothing that changes the
//! far side's state. That is a property worth keeping true, because it runs unattended while the
//! panel is open.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sevenshell_error::ShellError;
use sevenshell_types::{
    CpuCounters, CpuModel, CpuStates, DiskUsage, EnvironmentEntry, HostMetrics, LoadAverage, MAX_COMMAND_CHARS,
    MAX_DISKS, MAX_ENVIRONMENT_ENTRIES, MAX_PAGE_PROCESSES, MAX_PROCESSES, MAX_SOCKETS, MemoryUsage, NetworkCounters,
    ProcessDetail, ProcessPage, ProcessSample, SocketRow,
};

use crate::Core;

/// How long one sample may take.
///
/// Generous for a handful of `cat`s, because the command also runs `df` and `ps` and those can be
/// slow on a loaded machine or a hung mount — and a timeout here has to mean "this host is not
/// answering", not "this host was busy for a moment".
pub const SAMPLE_TIMEOUT: Duration = Duration::from_secs(10);

/// The columns a process is read by, in one place.
///
/// `comm` and `args` are both asked for on purpose: one is what the process is called and the other
/// is how it was started, and a table of five rows has room for both while a page of hundreds needs
/// them separated to be readable at all. Every command below uses this constant, so the sample's
/// rows and the page's rows cannot come to mean different things.
const PS_COLUMNS: &str = "pid=,user=,pcpu=,pmem=,rss=,comm=,args=";

/// The process table, busiest first, cut to `rows`.
///
/// The cap is on the far side because that is where the cost is: the table with its command lines
/// is tens of kilobytes on an ordinary desktop, and sending all of it to draw five rows beside a
/// terminal spends somebody else's upload on nothing. Sorting before cutting is what makes the cut
/// mean "the busiest" rather than "the first ones the kernel listed", which is by process id.
fn ps_busiest(rows: usize) -> String {
    format!("ps -eo {PS_COLUMNS} 2>/dev/null | sort -k3 -nr | head -n {rows}")
}

/// How many rows of the table the panel's sample asks the host for.
///
/// More than the five the panel draws — a row or two of slack costs nothing, and the host's CPU
/// column is what picks them — and far fewer than the table, because a sample every two seconds is
/// not where a full process list belongs.
const SAMPLE_PROCESS_ROWS: usize = 20;

/// What the process page runs: the table, and the two files that say more about each row.
///
/// Three reads in one round trip, because they are three questions about one list: what the
/// processes are, how much processor time each has used, and where each program lives on disk. The
/// time is a counter — the page subtracts two readings to get a percentage — and the executable
/// comes from one `ls` over every `/proc/<pid>/exe` rather than a `readlink` per process, which
/// would fork once per row on somebody's server.
fn process_page_command(limit: usize) -> String {
    format!(
        "echo '@@ps'
{}
echo '@@ticks'
cat /proc/[0-9]*/stat 2>/dev/null
echo '@@exe'
ls -l /proc/[0-9]*/exe 2>/dev/null
echo '@@hz'
getconf CLK_TCK 2>/dev/null || echo 100
",
        ps_busiest(limit)
    )
}

/// What one process is asked about when somebody opens it.
///
/// The environment is read here and nowhere else: it can hold credentials, and a sample that
/// collected every process's environment every couple of seconds would be gathering secrets nobody
/// asked to see. Opening the page for one process is somebody asking.
///
/// The pid arrives as a number and is formatted as one, so nothing a person types reaches a shell.
fn process_detail_command(pid: u32) -> String {
    format!(
        "echo '@@name'
cat /proc/{pid}/comm 2>/dev/null
echo '@@cmdline'
tr '\\0' ' ' < /proc/{pid}/cmdline 2>/dev/null
echo
echo '@@exe'
readlink /proc/{pid}/exe 2>/dev/null
echo '@@cwd'
readlink /proc/{pid}/cwd 2>/dev/null
echo '@@environ'
tr '\\0' '\\n' < /proc/{pid}/environ 2>/dev/null
"
    )
}

/// What the network page runs: the host's sockets, listening and connected.
///
/// `ss` is asked first and asked for the most: `-i` is what adds the byte counters, which is the
/// difference between a list of ports and a list somebody can see traffic in. A host without `ss`
/// — a container with busybox, most of all — falls through to `netstat`, which knows the same
/// sockets and none of the counters; the page draws those columns as dashes rather than as zeroes,
/// because a host that did not say is not a host with no traffic.
///
/// `-n` is not optional in either: without it the tools turn port numbers into service names and
/// addresses into host names, which is a name lookup per row and a table of `ssh` where the whole
/// point of the page is which port.
const SOCKET_COMMAND: &str = "ss -H -tunapi 2>/dev/null || ss -H -tunap 2>/dev/null || netstat -tunap 2>/dev/null";

/// What one sample runs on the far side.
///
/// `2>/dev/null` on every read is not tidiness: a host without `/proc` is a normal host, and the
/// errors would otherwise be the whole of the stdout. Every group is introduced by a marker line
/// the parser looks for, so a missing group and an empty group are told apart by whether the
/// marker arrived.
///
/// Assembled rather than written out as one literal only because of the process command: the
/// sample and the page have to ask `ps` the same question, and two copies of a column list is how
/// the second copy comes to be missing a column.
fn snapshot_command() -> String {
    format!(
        "\
echo '@@hostname'
hostname 2>/dev/null
echo '@@uname'
uname -srm 2>/dev/null
echo '@@os'
cat /etc/os-release 2>/dev/null
echo '@@cpuinfo'
head -n 40 /proc/cpuinfo 2>/dev/null
echo '@@uptime'
cat /proc/uptime 2>/dev/null
echo '@@load'
cat /proc/loadavg 2>/dev/null
echo '@@mem'
cat /proc/meminfo 2>/dev/null
echo '@@cpu'
head -n 1 /proc/stat 2>/dev/null
echo '@@cores'
(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null) | head -n 1
echo '@@net'
cat /proc/net/dev 2>/dev/null
echo '@@ps'
{}
echo '@@df'
df -kP 2>/dev/null
",
        ps_busiest(SAMPLE_PROCESS_ROWS)
    )
}

impl Core {
    /// Takes one sample of the host behind an open terminal.
    ///
    /// The handle is a terminal's, not a session id: a sample needs a connection that is up, and
    /// the registry is the only thing that knows one is. A terminal that has ended is refused by
    /// the registry with the same answer as a handle that never existed, which is the right one —
    /// there is nothing to ask.
    pub async fn sample_host(&self, handle: &str) -> Result<HostMetrics, ShellError> {
        let session = self.registry.session_of(handle).await?;
        let outcome = session.exec(&snapshot_command(), SAMPLE_TIMEOUT).await?;

        Ok(parse_snapshot(&outcome.stdout, now_millis(), outcome.duration))
    }

    /// The process list for the page of that name, ordered by what was asked for.
    ///
    /// A command and a call of its own rather than part of the sample, because the two are asked
    /// at different rates by different things: the panel wants five rows beside a terminal, and a
    /// page somebody has opened wants a list they can read — and pays for it only while it is the
    /// page on screen.
    pub async fn list_processes(&self, handle: &str, limit: usize) -> Result<ProcessPage, ShellError> {
        let limit = clamp_process_limit(limit);
        let session = self.registry.session_of(handle).await?;
        let outcome = session.exec(&process_page_command(limit), SAMPLE_TIMEOUT).await?;

        Ok(process_page(&outcome.stdout, limit))
    }

    /// One process, looked at closely: how it was started, from where, and with what environment.
    pub async fn process_detail(&self, handle: &str, pid: u32) -> Result<ProcessDetail, ShellError> {
        let session = self.registry.session_of(handle).await?;
        let outcome = session.exec(&process_detail_command(pid), SAMPLE_TIMEOUT).await?;

        Ok(process_detail(&outcome.stdout, pid))
    }

    /// The host's sockets, for the page of that name.
    ///
    /// Its own command for the same reason the process list has one: nobody needs a list of every
    /// established connection every two seconds beside a terminal, and the page that shows it is
    /// the only thing that pays for it.
    pub async fn list_sockets(&self, handle: &str) -> Result<Vec<SocketRow>, ShellError> {
        let session = self.registry.session_of(handle).await?;
        let outcome = session.exec(SOCKET_COMMAND, SAMPLE_TIMEOUT).await?;

        Ok(sockets(&outcome.stdout))
    }
}

/// The sample a command's output describes.
///
/// Pure, and split out from the call above so the parsing can be tested against text a real host
/// produced rather than against a live one — which is the only way to cover the operating systems
/// and the odd filesystem names this has to survive.
fn parse_snapshot(stdout: &str, sampled_at: u64, round_trip: Duration) -> HostMetrics {
    let groups = groups(stdout);

    let uname = first(&groups, "uname").unwrap_or_default();
    let mut uname_fields = uname.split_whitespace();
    let system_name = uname_fields.next().map(str::to_owned);
    let kernel = uname_fields.next().map(str::to_owned);
    let arch = uname_fields.next().map(str::to_owned);

    HostMetrics {
        sampled_at,
        round_trip_ms: u64::try_from(round_trip.as_millis()).unwrap_or(u64::MAX),
        hostname: first(&groups, "hostname")
            .filter(|name| !name.is_empty())
            .map(str::to_owned),
        // The distribution's own name is what a person recognises; the kernel's is the fallback
        // for a host that does not keep `/etc/os-release`, which is every BSD and every busybox.
        os: os_release(&groups).or(system_name),
        kernel,
        arch,
        cores: first(&groups, "cores").and_then(|line| line.trim().parse().ok()),
        cpu_model: groups.get("cpuinfo").and_then(|lines| cpu_model(lines)),
        uptime_seconds: first(&groups, "uptime").and_then(uptime_seconds),
        load: first(&groups, "load").and_then(load_average),
        cpu: first(&groups, "cpu").and_then(cpu_counters),
        memory: groups
            .get("mem")
            .map(|lines| memory_usage(lines))
            .filter(|usage| usage.total_bytes > 0),
        network: groups.get("net").map(|lines| interfaces(lines)).unwrap_or_default(),
        disks: groups.get("df").map(|lines| disks(lines)).unwrap_or_default(),
        processes: groups
            .get("ps")
            .map(|lines| processes(lines, MAX_PROCESSES))
            .unwrap_or_default(),
    }
}

/// The output, split into the groups its marker lines introduce.
///
/// A group's lines are the non-empty ones between its marker and the next, so a blank line from a
/// command that printed nothing is not mistaken for a reading.
fn groups(stdout: &str) -> HashMap<&str, Vec<&str>> {
    let mut out: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut current: Option<&str> = None;

    for line in stdout.lines() {
        if let Some(marker) = line.strip_prefix("@@") {
            let marker = marker.trim();
            out.entry(marker).or_default();
            current = Some(marker);
            continue;
        }

        if let Some(marker) = current
            && !line.trim().is_empty()
        {
            out.entry(marker).or_default().push(line);
        }
    }

    out
}

fn first<'a>(groups: &HashMap<&str, Vec<&'a str>>, name: &str) -> Option<&'a str> {
    groups.get(name)?.first().copied()
}

/// `PRETTY_NAME` from `/etc/os-release`, without the quotes the shell would have stripped.
fn os_release(groups: &HashMap<&str, Vec<&str>>) -> Option<String> {
    let lines = groups.get("os")?;

    for line in lines {
        let Some(value) = line.strip_prefix("PRETTY_NAME=") else {
            continue;
        };
        let name = value.trim().trim_matches(['"', '\'']).trim();
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }

    None
}

/// The first number of `/proc/uptime`: seconds since the machine came up.
fn uptime_seconds(line: &str) -> Option<f64> {
    let seconds: f64 = line.split_whitespace().next()?.parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds)
}

fn load_average(line: &str) -> Option<LoadAverage> {
    let mut fields = line.split_whitespace();
    let one = fields.next()?.parse().ok()?;
    let five = fields.next()?.parse().ok()?;
    let fifteen = fields.next()?.parse().ok()?;
    Some(LoadAverage { one, five, fifteen })
}

/// The CPU line of `/proc/stat`, as counters.
///
/// The first eight fields are user, nice, system, idle, iowait, irq, softirq and steal; the two
/// that may follow them, guest and guest_nice, are *already counted inside* user and nice, so
/// adding them would count a virtual machine's time twice. `idle` and `iowait` are one figure in
/// the summary, because a host waiting on a disk is not a busy host — but they are kept apart in
/// the breakdown, because a disk that cannot keep up is exactly what somebody reading that page is
/// looking for.
fn cpu_counters(line: &str) -> Option<CpuCounters> {
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;

    if fields.len() < 4 {
        return None;
    }

    let at = |index: usize| fields.get(index).copied().unwrap_or_default();
    let states = CpuStates {
        user_ticks: at(0),
        nice_ticks: at(1),
        system_ticks: at(2),
        idle_ticks: at(3),
        io_wait_ticks: at(4),
        interrupt_ticks: at(5),
        soft_interrupt_ticks: at(6),
        steal_ticks: at(7),
    };

    Some(CpuCounters {
        total_ticks: fields.iter().sum(),
        idle_ticks: states.idle_ticks + states.io_wait_ticks,
        states,
    })
}

/// What `/proc/cpuinfo` says the processor is.
///
/// The first value of each key wins, because the file repeats every key once per core and the
/// first block is the first processor — which is the one whose clock and cache are representative
/// of what it is, even though the clock of any given core drifts a moment later.
///
/// A host that names none of this has no `cpu_model`: the BSDs keep the same facts in `sysctl`, and
/// inventing a name out of the kernel string would be worse than the page saying it does not know.
fn cpu_model(lines: &[&str]) -> Option<CpuModel> {
    let mut name: Option<String> = None;
    let mut frequency_mhz: Option<f64> = None;
    let mut cache_kb: Option<u64> = None;
    let mut bogomips: Option<f64> = None;

    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();

        match key.trim() {
            "model name" if name.is_none() => name = Some(value.to_owned()),
            "cpu MHz" if frequency_mhz.is_none() => frequency_mhz = value.parse().ok(),
            // `36864 KB`, and on some kernels `36864 KBytes`: the unit is a word, not a suffix.
            "cache size" if cache_kb.is_none() => {
                cache_kb = value.split_whitespace().next().and_then(|size| size.parse().ok());
            }
            "bogomips" if bogomips.is_none() => bogomips = value.parse().ok(),
            _ => {}
        }
    }

    Some(CpuModel {
        name: name.filter(|name| !name.is_empty())?,
        frequency_mhz,
        cache_kb,
        bogomips,
    })
}

/// Memory and swap, from `/proc/meminfo`, whose values are in kibibytes.
fn memory_usage(lines: &[&str]) -> MemoryUsage {
    let mut values: HashMap<&str, u64> = HashMap::new();
    for line in lines {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(value) = rest
            .split_whitespace()
            .next()
            .and_then(|field| field.parse::<u64>().ok())
        else {
            continue;
        };
        values.insert(key.trim(), value);
    }

    let kib = |key: &str| values.get(key).copied().unwrap_or_default().saturating_mul(1024);
    let total_bytes = kib("MemTotal");

    // `MemAvailable` is the kernel's own answer and the one to use. Where it is missing — a
    // kernel older than 3.14 — free memory plus the page cache is the same thing computed by
    // hand, and either is better than reporting cache as used.
    let available_bytes = if values.contains_key("MemAvailable") {
        kib("MemAvailable")
    } else {
        kib("MemFree") + kib("Buffers") + kib("Cached") + kib("SReclaimable")
    };

    MemoryUsage {
        total_bytes,
        available_bytes: available_bytes.min(total_bytes),
        swap_total_bytes: kib("SwapTotal"),
        swap_free_bytes: kib("SwapFree"),
    }
}

/// The interfaces `/proc/net/dev` lists, loopback included.
///
/// Loopback is a real interface and the network page lists it like any other — a host that talks to
/// itself is doing something somebody may want to see. What it is *not* is the interface the panel
/// picks on its own, and that decision lives where the picking happens: `lib/monitor/rates.ts`
/// prefers anything else, because "the busiest interface is loopback" is a true answer to a
/// question nobody asked.
fn interfaces(lines: &[&str]) -> Vec<NetworkCounters> {
    let mut out = Vec::new();

    for line in lines {
        let Some((name, counters)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }

        let fields: Vec<&str> = counters.split_whitespace().collect();
        // Received bytes is the first column and transmitted bytes the ninth; anything shorter is
        // not an interface line, and the header of the file is exactly that.
        let (Some(received), Some(transmitted)) = (
            fields.first().and_then(|field| field.parse::<u64>().ok()),
            fields.get(8).and_then(|field| field.parse::<u64>().ok()),
        ) else {
            continue;
        };

        out.push(NetworkCounters {
            name: name.to_owned(),
            received_bytes: received,
            transmitted_bytes: transmitted,
        });
    }

    out
}

/// The mounts `df -kP` listed, in the order it listed them.
///
/// The numbers are read from the right rather than the left, because the first column is a device
/// name and device names contain spaces on some systems — `map -hosts` is one word to `df` and two
/// to a split. Working back from the mount point costs nothing and survives it.
fn disks(lines: &[&str]) -> Vec<DiskUsage> {
    let mut out = Vec::new();

    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }

        let Some(size_kib) = fields[fields.len() - 5].parse::<u64>().ok() else {
            // The header, whose block column is the literal `1024-blocks`.
            continue;
        };
        let Some(available_kib) = fields[fields.len() - 3].parse::<u64>().ok() else {
            continue;
        };

        let mount_point = fields[fields.len() - 1];
        if mount_point.is_empty() {
            continue;
        }

        out.push(DiskUsage {
            mount_point: mount_point.to_owned(),
            filesystem: fields[..fields.len() - 5].join(" "),
            size_bytes: size_kib.saturating_mul(1024),
            available_bytes: available_kib.saturating_mul(1024),
        });

        if out.len() == MAX_DISKS {
            break;
        }
    }

    out
}

/// The processes `ps` listed, ordered by what was asked for and cut to fit.
///
/// The processes `ps` listed, capped to what was asked for.
///
/// The order is the host's: the command sorts the table by CPU and cuts it before sending, so the
/// rows that arrive are the busiest ones and the first few of them are the busiest few. Nothing is
/// re-sorted here — a second opinion about the order would be a second thing to keep in step with
/// the first, and the page that wants a different order has the whole table to order itself.
fn processes(lines: &[&str], limit: usize) -> Vec<ProcessSample> {
    let mut out: Vec<ProcessSample> = lines
        .iter()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 6 {
                return None;
            }

            Some(ProcessSample {
                pid: fields[0].parse().ok()?,
                user: fields[1].to_owned(),
                cpu_percent: fields[2].parse().ok()?,
                memory_percent: fields[3].parse().ok()?,
                // `ps` reports the resident set in kibibytes on every system this has been run
                // against, including the BSDs, which is not something the column heading admits.
                resident_bytes: fields[4].parse::<u64>().ok()?.saturating_mul(1024),
                name: fields[5].to_owned(),
                command: cap_command(&fields[6..].join(" ")),
                // Neither of these is in `ps`: what is not asked for stays unset rather than
                // becoming a zero, which would read as "this process has used no processor time".
                executable: None,
                cpu_ticks: None,
            })
        })
        .collect();

    out.truncate(limit);
    out
}

/// The process page, from the three reads it makes.
fn process_page(stdout: &str, limit: usize) -> ProcessPage {
    let groups = groups(stdout);
    let ticks = groups
        .get("ticks")
        .map(|lines| process_ticks(lines))
        .unwrap_or_default();
    let executables = groups.get("exe").map(|lines| executables(lines)).unwrap_or_default();

    // Almost always 100. Read rather than assumed, because every percentage on the page is divided
    // by it, and a host that counts differently would make every row wrong by the same factor.
    let ticks_per_second = first(&groups, "hz")
        .and_then(|line| line.trim().parse().ok())
        .filter(|hz| *hz > 0)
        .unwrap_or(100);

    let processes = groups
        .get("ps")
        .map(|lines| processes(lines, limit))
        .unwrap_or_default()
        .into_iter()
        .map(|mut row| {
            row.cpu_ticks = ticks.get(&row.pid).copied();
            row.executable = executables.get(&row.pid).cloned();
            row
        })
        .collect();

    ProcessPage {
        ticks_per_second,
        processes,
    }
}

/// How much processor time each process has used, from `/proc/<pid>/stat`.
///
/// The second field of that file is the program's name in parentheses, and a name may contain
/// spaces and parentheses of its own — so the fields are counted from the *last* `)` rather than
/// from the start of the line, which is the only reading that survives a process called `(foo) bar`
/// and the one the kernel's own documentation asks for.
///
/// The two figures added together are user and system time, in the kernel's ticks. A counter rather
/// than a rate: what the page shows is the difference between two of these, over the time between
/// the two reads.
fn process_ticks(lines: &[&str]) -> HashMap<u32, u64> {
    let mut out = HashMap::new();

    for line in lines {
        let Some((head, rest)) = line.split_once(" (") else {
            continue;
        };
        let Some(close) = rest.rfind(')') else {
            continue;
        };
        let Ok(pid) = head.trim().parse::<u32>() else {
            continue;
        };

        let fields: Vec<&str> = rest[close + 1..].split_whitespace().collect();
        let (Some(user), Some(system)) = (fields.get(11), fields.get(12)) else {
            continue;
        };
        let (Ok(user), Ok(system)) = (user.parse::<u64>(), system.parse::<u64>()) else {
            continue;
        };

        out.insert(pid, user.saturating_add(system));
    }

    out
}

/// Where each process's program is on disk, from one `ls` over `/proc/<pid>/exe`.
///
/// The answer is the host's own: a link may point at a file that has since been replaced, and a
/// binary that was deleted while running is reported as such rather than as the name it had. That
/// is more useful than a tidy answer, because it is what is true of the process right now.
fn executables(lines: &[&str]) -> HashMap<u32, String> {
    let mut out = HashMap::new();

    for line in lines {
        let Some((link, target)) = line.split_once(" -> ") else {
            continue;
        };
        let Some(path) = link.split_whitespace().last() else {
            continue;
        };
        let Some(pid) = path.strip_prefix("/proc/").and_then(|rest| rest.strip_suffix("/exe")) else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };

        out.insert(pid, target.trim().to_owned());
    }

    out
}

/// One process, from the four files that describe it.
fn process_detail(stdout: &str, pid: u32) -> ProcessDetail {
    let groups = groups(stdout);

    ProcessDetail {
        pid,
        name: first(&groups, "name").unwrap_or_default().trim().to_owned(),
        command_line: first(&groups, "cmdline").unwrap_or_default().trim().to_owned(),
        executable: first(&groups, "exe").unwrap_or_default().trim().to_owned(),
        working_directory: first(&groups, "cwd").unwrap_or_default().trim().to_owned(),
        environment: groups
            .get("environ")
            .map(|lines| environment(lines))
            .unwrap_or_default(),
    }
}

/// The variables a process was started with, in the order the kernel lists them, capped.
///
/// A line that is not `NAME=value` is dropped rather than shown as a name with nothing after it:
/// the kernel's format has no other kind of line, so one is a decoding artefact from a value with
/// bytes that are not text, and putting that in the table would be inventing a variable.
fn environment(lines: &[&str]) -> Vec<EnvironmentEntry> {
    lines
        .iter()
        .take(MAX_ENVIRONMENT_ENTRIES)
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            Some(EnvironmentEntry {
                name: name.to_owned(),
                value: value.to_owned(),
            })
        })
        .collect()
}

/// A command line, cut to what a table can show.
///
/// Truncation is by characters rather than bytes, so a command line with a non-ASCII path in it is
/// cut where a person reading it would cut it and not in the middle of a character.
fn cap_command(command: &str) -> String {
    if command.chars().count() <= MAX_COMMAND_CHARS {
        return command.to_owned();
    }
    let kept: String = command.chars().take(MAX_COMMAND_CHARS - 1).collect();
    format!("{kept}…")
}

/// At least one process, never more than a page is allowed, and never more than was asked for.
fn clamp_process_limit(asked: usize) -> usize {
    asked.clamp(1, MAX_PAGE_PROCESSES)
}

/// The states a socket can be in, as either tool spells them.
///
/// A closed list rather than a guess: it is what tells one tool's output from the other's, since
/// `ss` puts the state where `netstat` puts a queue length, and a column of numbers read as a state
/// would turn every socket on the page into a connection.
const SOCKET_STATES: &[&str] = &[
    "LISTEN",
    "ESTAB",
    "UNCONN",
    "SYN-SENT",
    "SYN-RECV",
    "FIN-WAIT-1",
    "FIN-WAIT-2",
    "TIME-WAIT",
    "CLOSE-WAIT",
    "LAST-ACK",
    "CLOSING",
    "CLOSED",
];

/// Whether a word names a state a socket can be in. The two tools disagree about the separator —
/// `ss` writes `TIME-WAIT` and `netstat` writes `TIME_WAIT` — so the comparison happens on one.
fn is_socket_state(word: &str) -> bool {
    let normalized = word.replace('_', "-").to_ascii_uppercase();
    SOCKET_STATES.contains(&normalized.as_str())
}

/// Whether a socket is waiting for something rather than talking to something.
///
/// Two states mean that, not one: TCP listens, and UDP — which accepts nothing and so has no
/// listening state — is `UNCONN` when it names no peer. To a reader they are the same fact.
fn is_listening(state: &str) -> bool {
    state == "LISTEN" || state == "UNCONN"
}

/// The sockets a host's own tool listed.
///
/// One parser for two tools, because they print the same three facts — where the socket is, where
/// its peer is, and who holds it — in different clothes:
///
/// ```text
/// ss -tunapi:  tcp  LISTEN 0  4096  0.0.0.0:22  0.0.0.0:*  users:(("sshd",pid=1,fd=3))
///              <tab> cubic … bytes_sent:1400 bytes_received:560
/// netstat:     tcp    0     0 0.0.0.0:22     0.0.0.0:*    LISTEN   1/sshd
/// ```
///
/// The indented line `ss` prints under a socket with `-i` belongs to the socket above it and
/// carries the only byte counters either tool produces. It is read here rather than asked for
/// separately, because it arrives attached to the row it is about and there is nothing to join.
fn sockets(stdout: &str) -> Vec<SocketRow> {
    let mut out: Vec<SocketRow> = Vec::new();

    for line in stdout.lines() {
        if line.starts_with(char::is_whitespace) {
            // An info line, or a heading that happens to be indented. Either way it is not a socket,
            // and the counters it may carry belong to the row above.
            if let Some(row) = out.last_mut() {
                apply_counters(row, line);
            }
            continue;
        }

        if let Some(row) = socket_row(line) {
            out.push(row);
        }
    }

    // Listening first, then by port: the page is about what is waiting for a connection, and the
    // list is cut before it is sent — so what survives the cut has to be what somebody came for.
    out.sort_by(|left, right| {
        is_listening(&right.state)
            .cmp(&is_listening(&left.state))
            .then_with(|| left.protocol.cmp(&right.protocol))
            .then_with(|| left.port.cmp(&right.port))
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.peer_address.cmp(&right.peer_address))
            .then_with(|| left.peer_port.cmp(&right.peer_port))
    });
    out.truncate(MAX_SOCKETS);
    out
}

/// One line of either tool's output, as a socket — or nothing, for a line that is not one.
fn socket_row(line: &str) -> Option<SocketRow> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let protocol = fields.first()?.to_ascii_lowercase();
    if !protocol.starts_with("tcp") && !protocol.starts_with("udp") {
        // A heading, a blank, or a unix socket: `netstat -a` prints those too, and none of them is
        // what this page is about.
        return None;
    }

    // `ss` puts the state where `netstat` puts the first queue length, and that is the whole of the
    // difference in where the columns after it sit.
    let (state, local, peer, owner): (&str, &str, &str, Option<String>) =
        if fields.get(1).is_some_and(|field| is_socket_state(field)) {
            (
                fields[1],
                fields.get(4)?,
                fields.get(5)?,
                fields.get(6..).map(|rest| rest.join(" ")),
            )
        } else {
            let local = fields.get(3)?;
            let peer = fields.get(4)?;
            // A TCP row carries a state and a UDP row does not, so what follows the peer is one or
            // the other — and the words that mean a state are a closed list.
            match fields.get(5) {
                Some(next) if is_socket_state(next) => (next, local, peer, fields.get(6..).map(|rest| rest.join(" "))),
                _ => ("", local, peer, fields.get(5..).map(|rest| rest.join(" "))),
            }
        };

    let (address, port) = split_endpoint(local)?;
    let peer = split_endpoint(peer);
    let (pid, process) = socket_owner(owner.as_deref().unwrap_or_default());

    Some(SocketRow {
        protocol,
        // One spelling for one state: the two tools disagree about the separator and nothing else.
        state: state.replace('_', "-").to_ascii_uppercase(),
        address,
        port,
        pid,
        process,
        peer_address: peer.as_ref().map(|(address, _)| address.clone()).unwrap_or_default(),
        peer_port: peer.map(|(_, port)| port).unwrap_or_default(),
        sent_bytes: None,
        received_bytes: None,
    })
}

/// An endpoint as the tools print it: `0.0.0.0:22`, `127.0.0.53:53`, `[::]:22`, `:::2222`.
///
/// The last of those is busybox's spelling of the IPv6 wildcard, and the brackets are everyone
/// else's: both are the same address, and the page shows the same thing for either. A peer of
/// `0.0.0.0:*` is not an endpoint at all — a socket that is waiting has no other end — which is why
/// this answers with `None` rather than with port zero.
fn split_endpoint(endpoint: &str) -> Option<(String, u16)> {
    let (address, port) = endpoint.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;

    let address = address.trim_start_matches('[').trim_end_matches(']');
    let address = if address.is_empty() || address.chars().all(|character| character == ':') {
        "::".to_owned()
    } else {
        address.to_owned()
    };

    Some((address, port))
}

/// The process behind a socket, in whichever way the tool named it.
///
/// Three spellings for one fact: `ss` writes `users:(("sshd",pid=1,fd=3))`, `netstat` writes
/// `1/sshd`, and busybox writes `-` when it will not say — which is the same as saying nothing, and
/// is drawn as a dash rather than as a process called `-`.
fn socket_owner(text: &str) -> (u32, String) {
    if let Some((_, rest)) = text.split_once("users:((") {
        let name = rest.split('"').nth(1).unwrap_or_default().to_owned();
        let pid = rest
            .split_once("pid=")
            .and_then(|(_, tail)| tail.split(|character: char| !character.is_ascii_digit()).next())
            .and_then(|digits| digits.parse().ok())
            .unwrap_or_default();
        return (pid, name);
    }

    match text.split_once('/') {
        Some((pid, name)) => (pid.trim().parse().unwrap_or_default(), name.trim().to_owned()),
        None => (0, String::new()),
    }
}

/// The byte counters an `ss -i` info line carries, when it carries them.
fn apply_counters(row: &mut SocketRow, line: &str) {
    if let Some(sent) = counter(line, "bytes_sent:") {
        row.sent_bytes = Some(sent);
    }
    if let Some(received) = counter(line, "bytes_received:") {
        row.received_bytes = Some(received);
    }
}

fn counter(line: &str, key: &str) -> Option<u64> {
    let (_, tail) = line.split_once(key)?;
    let digits: String = tail
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real answer, from a Linux host: `/etc/os-release`, `/proc` and the three
    /// commands that come from elsewhere.
    const SNAPSHOT: &str = "\
@@hostname
web-1
@@uname
Linux 5.15.0-91-generic x86_64
@@os
NAME=\"Ubuntu\"
VERSION=\"22.04.3 LTS (Jammy Jellyfish)\"
PRETTY_NAME=\"Ubuntu 22.04.3 LTS\"
@@cpuinfo
processor\t: 0
vendor_id\t: GenuineIntel
model name\t: Intel(R) Core(TM) i9-14900HX
cpu MHz\t\t: 2419.198
cache size\t: 36864 KB
bogomips\t: 4838.39
@@uptime
123456.78 987654.32
@@load
0.02 0.01 0.00 1/234 5678
@@mem
MemTotal:        8009984 kB
MemFree:         1000000 kB
MemAvailable:    6543210 kB
Buffers:          123456 kB
Cached:          2345678 kB
SwapTotal:       4194304 kB
SwapFree:        4194304 kB
@@cpu
cpu  1000 200 300 40000 500 60 70 80 0 0
@@cores
4
@@net
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 5000       50    0    0    0     0          0         0     5000      50    0    0    0     0       0          0
  eth0: 12345678  9000    0    0    0     0          0         0  2345678    8000    0    0    0     0       0          0
@@ps
    1 root      0.7  1.2  11878 systemd         /sbin/init splash
   42 root      0.3  0.6   6656 top             top -b -d 1
  900 root      0.0  1.4  14848 systemd-journal /lib/systemd/systemd-journald
  901 root      0.0  0.0      0 kthreadd        [kthreadd]
@@df
Filesystem     1024-blocks     Used Available Capacity Mounted on
overlay           97552384 39781448  57770936      41% /
tmpfs                65536        0     65536       0% /dev
/dev/sda1        104857600 12345678  87234567      13% /data
";

    fn parse(stdout: &str) -> HostMetrics {
        parse_snapshot(stdout, 1_700_000_000_000, Duration::from_millis(37))
    }

    #[test]
    fn a_linux_snapshot_is_read_group_by_group() {
        let sample = parse(SNAPSHOT);

        assert_eq!(sample.sampled_at, 1_700_000_000_000);
        assert_eq!(sample.round_trip_ms, 37);
        assert_eq!(sample.hostname.as_deref(), Some("web-1"));
        assert_eq!(sample.os.as_deref(), Some("Ubuntu 22.04.3 LTS"));
        assert_eq!(sample.kernel.as_deref(), Some("5.15.0-91-generic"));
        assert_eq!(sample.arch.as_deref(), Some("x86_64"));
        assert_eq!(sample.cores, Some(4));
        assert_eq!(sample.uptime_seconds, Some(123456.78));

        let load = sample.load.expect("the load line was there");
        assert_eq!((load.one, load.five, load.fifteen), (0.02, 0.01, 0.00));

        let memory = sample.memory.expect("meminfo was there");
        assert_eq!(memory.total_bytes, 8_009_984 * 1024);
        assert_eq!(memory.available_bytes, 6_543_210 * 1024);
        assert_eq!(memory.swap_total_bytes, 4_194_304 * 1024);
        assert_eq!(memory.swap_free_bytes, memory.swap_total_bytes);

        let disks: Vec<&str> = sample.disks.iter().map(|disk| disk.mount_point.as_str()).collect();
        assert_eq!(disks, ["/", "/dev", "/data"]);
        assert_eq!(sample.disks[0].filesystem, "overlay");
        assert_eq!(sample.disks[0].size_bytes, 97_552_384 * 1024);
        assert_eq!(sample.disks[0].available_bytes, 57_770_936 * 1024);
    }

    /// The counter that a rate is computed from. Guest time is inside user and nice already, so
    /// the total is the first eight fields and nothing beyond them.
    #[test]
    fn the_cpu_counters_exclude_the_guest_time_that_is_already_counted() {
        let counters = cpu_counters("cpu  1000 200 300 40000 500 60 70 80 999 999").expect("counters");

        assert_eq!(counters.total_ticks, 1000 + 200 + 300 + 40000 + 500 + 60 + 70 + 80);
        assert_eq!(counters.idle_ticks, 40000 + 500, "waiting on a disk is not being busy");

        // The breakdown keeps the two apart, because a host waiting on a disk is what somebody
        // reading that page is looking for.
        assert_eq!(counters.states.io_wait_ticks, 500);
        assert_eq!(counters.states.idle_ticks, 40000);
        assert_eq!(counters.states.user_ticks, 1000);
        assert_eq!(counters.states.system_ticks, 300);
        assert_eq!(counters.states.steal_ticks, 80);
    }

    /// What the host calls its processor, read from the first block of `/proc/cpuinfo`: the file
    /// repeats every key once per core, and the first one is the one to believe.
    #[test]
    fn the_processor_is_read_from_the_first_block_of_cpuinfo() {
        let model = parse(SNAPSHOT).cpu_model.expect("cpuinfo was there");

        assert_eq!(model.name, "Intel(R) Core(TM) i9-14900HX");
        assert_eq!(model.frequency_mhz, Some(2419.198));
        assert_eq!(
            model.cache_kb,
            Some(36864),
            "the unit after the number is a word, not a suffix"
        );
        assert_eq!(model.bogomips, Some(4838.39));
    }

    /// A host that names no processor has no model rather than a made-up one — the BSDs keep these
    /// facts elsewhere, and a page that invents them is worse than a page that says it does not know.
    #[test]
    fn a_host_that_does_not_name_its_processor_has_no_model() {
        let lines = ["processor\t: 0", "cpu MHz\t\t: 2400.000", "BogoMIPS\t: 4800.00"];
        let borrowed: Vec<&str> = lines.to_vec();

        assert_eq!(cpu_model(&borrowed), None);
    }

    #[test]
    fn every_interface_is_listed_loopback_included() {
        let sample = parse(SNAPSHOT);
        let names: Vec<&str> = sample.network.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(names, ["lo", "eth0"], "the network page lists them all");
        assert_eq!(sample.network[1].received_bytes, 12_345_678);
        assert_eq!(sample.network[1].transmitted_bytes, 2_345_678);
    }

    #[test]
    fn the_busiest_processes_come_first_and_the_list_is_capped() {
        let sample = parse(SNAPSHOT);
        let names: Vec<&str> = sample.processes.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(names, ["systemd", "top", "systemd-journal", "kthreadd"]);

        let first = &sample.processes[0];
        assert_eq!(first.user, "root");
        assert_eq!(first.resident_bytes, 11_878 * 1024);
        assert_eq!(first.cpu_percent, 0.7);
        assert_eq!(
            first.command, "/sbin/init splash",
            "how it was started, not what it is called"
        );

        let many: String = (1..=40)
            .map(|pid| format!("{pid} root 0.0 0.0 100 process{pid} /usr/bin/process{pid}\n"))
            .collect();
        assert_eq!(
            processes(&many.lines().collect::<Vec<_>>(), MAX_PROCESSES).len(),
            MAX_PROCESSES,
            "the panel's table is five rows"
        );
    }

    /// The list is cut where it is sent, and the order it is cut in is the host's: the command
    /// sorts by CPU before cutting. Nothing is re-sorted here, so what arrives in the first two rows
    /// is the two that are shown — a second opinion about the order would be a second thing to keep
    /// in step with the first.
    #[test]
    fn the_process_list_is_cut_to_what_was_asked_for_and_keeps_the_order_it_arrived_in() {
        let lines = [
            "  1 root 90.0  0.1   1000 busy   /usr/bin/busy",
            "  3 root  5.0  5.0 250000 middle /usr/bin/middle",
            "  2 root  0.0  9.9 500000 fat    /usr/bin/fat",
        ];
        let borrowed: Vec<&str> = lines.to_vec();

        let cut = processes(&borrowed, 2);
        assert_eq!(cut.iter().map(|entry| entry.pid).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(cut.len(), 2, "and no more than were asked for");
    }

    /// A read that did not ask for them leaves the two fields it did not read unset — which is not
    /// the same as a process that has used no processor time, or one that lives nowhere.
    #[test]
    fn a_process_row_carries_only_what_the_read_asked_for() {
        let lines = ["  1 root 0.7 1.2 11878 systemd /sbin/init splash"];

        let row = &processes(&lines, 5)[0];
        assert_eq!(row.cpu_ticks, None);
        assert_eq!(row.executable, None);
        assert_eq!(row.cpu_percent, 0.7, "which is what the host's own column said");
    }

    /// The three reads joined into one page: what `ps` said, with the processor time and the path
    /// each pid's other two lines carry. A join by the wrong key would put one process's counters on
    /// another's row, which is the kind of mistake that reads as a plausible number.
    #[test]
    fn a_process_page_joins_the_three_reads_by_pid() {
        let stdout = "@@ps
   42 root      3.5  0.6   6656 top             top -b -d 1
    1 root      0.1  0.2   1024 systemd         /sbin/init splash
@@ticks
42 (top) R 1 1 1 0 -1 0 0 0 0 0 800 200 0 0 20 0 1 0 0 0
1 (systemd) S 0 1 1 0 -1 0 0 0 0 0 400 100 0 0 20 0 1 0 0 0
@@exe
lrwxrwxrwx 1 root root 0 Oct  1 11:25 /proc/42/exe -> /usr/bin/top
@@hz
100
";

        let page = process_page(stdout, 50);

        assert_eq!(page.ticks_per_second, 100);
        assert_eq!(page.processes.len(), 2);
        assert_eq!(page.processes[0].pid, 42);
        assert_eq!(page.processes[0].cpu_ticks, Some(1_000), "user 800 plus system 200");
        assert_eq!(page.processes[0].executable.as_deref(), Some("/usr/bin/top"));
        assert_eq!(page.processes[1].pid, 1);
        assert_eq!(page.processes[1].cpu_ticks, Some(500));
        assert_eq!(
            page.processes[1].executable, None,
            "a process with no listing keeps nothing rather than borrowing the row above it"
        );
    }

    /// A host that does not say how it counts is counted the way every Linux host counts.
    #[test]
    fn a_page_from_a_host_that_does_not_count_its_ticks_uses_the_usual_hundred() {
        let page = process_page(
            "@@ps
   1 root 0.0 0.0 0 x x
@@hz

",
            50,
        );

        assert_eq!(page.ticks_per_second, 100);
    }

    /// The processor time each process has used, from `/proc/<pid>/stat`.
    ///
    /// The fields are counted from the program name in parentheses, and a name may contain a
    /// parenthesis of its own — which is the row that tells a parser counting from the left apart
    /// from one counting from the right.
    #[test]
    fn the_processor_time_of_each_process_is_read_from_proc_stat() {
        let lines = [
            "1 (s6-svscan) S 0 1 1 0 -1 4194560 1211 13135 3 7 420 500 20 0 1 0 677451 0 0 0",
            "2 ((weird) name) R 1 1 1 0 -1 0 0 0 0 0 7 4 20 0 1 0 0 0 0 0",
            "3 (no-time) Z 0 0 0 0 -1 0 0 0 0 0",
        ];
        let ticks = process_ticks(&lines);

        assert_eq!(ticks.get(&1), Some(&920), "user 420 plus system 500");
        assert_eq!(ticks.get(&2), Some(&11), "counted from the last parenthesis");
        assert_eq!(
            ticks.get(&3),
            None,
            "a line without the fields is not a reading of zero"
        );
    }

    #[test]
    fn where_each_program_lives_is_read_from_one_listing() {
        let lines = [
            "lrwxrwxrwx 1 root root 0 Oct  1 11:25 /proc/1/exe -> /package/admin/s6/command/s6-svscan",
            "lrwxrwxrwx 1 root root 0 Oct  1 11:25 /proc/900/exe -> /usr/bin/gone (deleted)",
            "ls: cannot read symbolic link '/proc/3/exe': No such file or directory",
        ];
        let executables = executables(&lines);

        assert_eq!(
            executables.get(&1).map(String::as_str),
            Some("/package/admin/s6/command/s6-svscan")
        );
        assert_eq!(
            executables.get(&900).map(String::as_str),
            Some("/usr/bin/gone (deleted)"),
            "a program that was replaced while running is reported as the host sees it"
        );
        assert_eq!(executables.len(), 2, "a complaint from `ls` is not a path");
    }

    /// One process in full, including the environment — the one thing here that can hold a
    /// credential, and the reason it is read for one process at a time rather than in a sample.
    #[test]
    fn one_process_is_described_by_its_own_files() {
        let stdout = "@@name
sshd
@@cmdline
/usr/sbin/sshd -D -e
@@exe
/usr/sbin/sshd
@@cwd
/
@@environ
PATH=/usr/bin:/bin
LANG=zh_CN.UTF-8
NOT-A-VARIABLE
";

        let detail = process_detail(stdout, 1741);

        assert_eq!(detail.pid, 1741);
        assert_eq!(detail.name, "sshd");
        assert_eq!(detail.command_line, "/usr/sbin/sshd -D -e");
        assert_eq!(detail.executable, "/usr/sbin/sshd");
        assert_eq!(detail.working_directory, "/");
        assert_eq!(
            detail.environment,
            vec![
                EnvironmentEntry {
                    name: "PATH".to_owned(),
                    value: "/usr/bin:/bin".to_owned(),
                },
                EnvironmentEntry {
                    name: "LANG".to_owned(),
                    value: "zh_CN.UTF-8".to_owned(),
                },
            ],
            "a line that is not NAME=value is dropped rather than shown as a nameless variable"
        );
    }

    /// A process with nothing in a file is described by what the other files said: a kernel thread
    /// has no command line and no environment, and that is an absence rather than a failure.
    #[test]
    fn a_process_with_no_command_line_is_still_described() {
        let detail = process_detail(
            "@@name
kworker/0:0H
@@cmdline
@@exe
@@cwd
@@environ
",
            42,
        );

        assert_eq!(detail.name, "kworker/0:0H");
        assert_eq!(detail.command_line, "");
        assert_eq!(detail.executable, "");
        assert_eq!(detail.working_directory, "");
        assert!(detail.environment.is_empty());
    }

    /// A command line is what a process was started with, and some are longer than any table can
    /// show. Cutting it is by characters, so a path that is not ASCII is not cut in half.
    #[test]
    fn a_long_command_line_is_cut_to_what_a_table_can_show() {
        let long = "x".repeat(MAX_COMMAND_CHARS * 2);
        assert_eq!(cap_command(&long).chars().count(), MAX_COMMAND_CHARS);
        assert!(cap_command(&long).ends_with('…'));

        let accented = "é".repeat(MAX_COMMAND_CHARS + 50);
        let cut = cap_command(&accented);
        assert_eq!(cut.chars().count(), MAX_COMMAND_CHARS);
        assert!(cut.chars().all(|character| character == 'é' || character == '…'));

        assert_eq!(cap_command("/usr/sbin/sshd -D"), "/usr/sbin/sshd -D");
    }

    #[test]
    fn the_page_list_is_bounded_however_much_it_asks_for() {
        assert_eq!(clamp_process_limit(0), 1);
        assert_eq!(clamp_process_limit(50), 50);
        assert_eq!(clamp_process_limit(usize::MAX), MAX_PAGE_PROCESSES);
    }

    /// `ss -tunapi` on a host with a web server and a resolver: a listener on two addresses, an
    /// established connection, a UDP socket that names no peer, and the indented info line that
    /// carries the only byte counters either tool produces.
    const SS_SOCKETS: &str = "\
tcp   LISTEN 0      4096         0.0.0.0:22         0.0.0.0:*    users:((\"sshd\",pid=1741,fd=3))
	 skmem:(r0,rb106496,t0,tb106496,f0,w0,o0,bl0,d0)
tcp   ESTAB  0      0      192.168.254.133:22   192.168.254.1:52344 users:((\"sshd\",pid=1787,fd=4))
	 cubic wscale:7,7 rto:204 rtt:0.5/0.2 mss:32768 bytes_sent:1400 bytes_received:560
tcp   LISTEN 0      511             [::]:80            [::]:*    users:((\"apache2\",pid=1787,fd=6))
tcp   LISTEN 0      4096        127.0.0.1:631        0.0.0.0:*    users:((\"cupsd\",pid=1752,fd=7))
udp   UNCONN 0      0      127.0.0.53:53          0.0.0.0:*    users:((\"systemd-resolve\",pid=848,fd=13))
tcp   TIME-WAIT 0   0      192.168.254.133:22   192.168.254.1:52345
";

    /// `netstat -tunap` on the same host: no counters, `TIME_WAIT` spelled with an underscore, the
    /// IPv6 wildcard as `:::`, and `-` where the tool will not attribute a socket to a process.
    const NETSTAT_SOCKETS: &str = "\nActive Internet connections (servers and established)
Proto Recv-Q Send-Q Local Address           Foreign Address         State       PID/Program name
tcp        0      0 0.0.0.0:22              0.0.0.0:*               LISTEN      1/sshd
tcp        0      0 192.168.254.133:22      192.168.254.1:52344     ESTAB       1787/sshd
tcp        0      0 :::2222                 :::*                    LISTEN      -
tcp        0      0 192.168.254.133:22      192.168.254.1:52345     TIME_WAIT   -
udp        0      0 127.0.0.11:43449        0.0.0.0:*                           -
";

    #[test]
    fn an_ss_listing_is_read_with_its_counters() {
        let sockets = sockets(SS_SOCKETS);

        assert_eq!(
            sockets.len(),
            6,
            "six sockets, and the info lines are not seven of them"
        );

        let sshd = &sockets[0];
        assert_eq!(sshd.protocol, "tcp");
        assert_eq!(sshd.state, "LISTEN");
        assert_eq!(sshd.address, "0.0.0.0");
        assert_eq!(sshd.port, 22);
        assert_eq!(sshd.pid, 1741);
        assert_eq!(sshd.process, "sshd");
        assert_eq!(sshd.peer_address, "", "a listener has no other end");
        assert_eq!(
            sshd.sent_bytes, None,
            "a waiting socket reports no counters, and the info line under it carries none either"
        );

        let established = sockets.iter().find(|row| row.state == "ESTAB").expect("the connection");
        assert_eq!(established.peer_address, "192.168.254.1");
        assert_eq!(established.peer_port, 52344);
        assert_eq!(
            established.sent_bytes,
            Some(1400),
            "the counters arrive on the line below"
        );
        assert_eq!(established.received_bytes, Some(560));

        let apache = sockets
            .iter()
            .find(|row| row.process == "apache2")
            .expect("the v6 listener");
        assert_eq!(apache.address, "::", "the brackets are the tool's, not the address's");

        let resolver = sockets
            .iter()
            .find(|row| row.protocol == "udp")
            .expect("the udp socket");
        assert_eq!(resolver.state, "UNCONN", "udp does not listen, and it is still waiting");
        assert!(
            is_listening(&resolver.state),
            "which is what the page means by listening"
        );
    }

    #[test]
    fn a_netstat_listing_is_read_the_same_way() {
        let sockets = sockets(NETSTAT_SOCKETS);

        assert_eq!(sockets.len(), 5, "the two heading lines are not sockets");

        let listener = &sockets[0];
        assert_eq!((listener.address.as_str(), listener.port), ("0.0.0.0", 22));
        assert_eq!(listener.pid, 1);
        assert_eq!(listener.process, "sshd");
        assert_eq!(
            listener.sent_bytes, None,
            "netstat reports no counters, which is not zero"
        );

        let wildcard = sockets
            .iter()
            .find(|row| row.port == 2222)
            .expect("the container's port");
        assert_eq!(wildcard.address, "::", "busybox spells the v6 wildcard `:::`");

        let unattributed = sockets
            .iter()
            .find(|row| row.state == "TIME-WAIT")
            .expect("the closing connection");
        assert_eq!(unattributed.pid, 0, "a tool that will not say leaves no process behind");
        assert_eq!(unattributed.process, "");
        assert_eq!(
            unattributed.state, "TIME-WAIT",
            "and the separator is the same either way"
        );

        let udp = sockets
            .iter()
            .find(|row| row.protocol == "udp")
            .expect("the udp socket");
        assert_eq!(
            udp.state, "",
            "netstat writes no state for udp, and nothing is invented"
        );
        assert_eq!(udp.port, 43449);
    }

    /// The list is cut before it is sent, so what survives has to be what the page is for.
    #[test]
    fn the_socket_list_is_cut_with_the_listening_sockets_kept() {
        let mut lines = String::from(
            "tcp LISTEN 0 4096 0.0.0.0:22 0.0.0.0:* users:((\"sshd\",pid=1,fd=3))
",
        );
        for port in 30_000..30_000 + MAX_SOCKETS + 20 {
            lines.push_str(&format!(
                "tcp ESTAB 0 0 10.0.0.1:{port} 10.0.0.2:{port}
"
            ));
        }

        let sockets = sockets(&lines);
        assert_eq!(sockets.len(), MAX_SOCKETS);
        assert_eq!(
            sockets[0].port, 22,
            "the listener is the one row nobody can afford to lose"
        );
        assert!(sockets.iter().any(|row| is_listening(&row.state)));
    }

    /// A socket the tool says nothing about is still a socket on a port: the address and the port
    /// are what somebody is looking for, and the process is the extra.
    #[test]
    fn a_socket_nobody_will_claim_is_still_listed() {
        let sockets = sockets(
            "tcp LISTEN 0 4096 0.0.0.0:22 0.0.0.0:*
",
        );

        assert_eq!(sockets.len(), 1);
        assert_eq!(sockets[0].port, 22);
        assert_eq!((sockets[0].pid, sockets[0].process.as_str()), (0, ""));
    }

    #[test]
    fn an_endpoint_is_read_the_way_each_tool_writes_it() {
        assert_eq!(split_endpoint("0.0.0.0:22"), Some(("0.0.0.0".to_owned(), 22)));
        assert_eq!(split_endpoint("127.0.0.53:53"), Some(("127.0.0.53".to_owned(), 53)));
        assert_eq!(split_endpoint("[::]:22"), Some(("::".to_owned(), 22)));
        assert_eq!(split_endpoint(":::2222"), Some(("::".to_owned(), 2222)));
        assert_eq!(split_endpoint("[fe80::1]:631"), Some(("fe80::1".to_owned(), 631)));
        assert_eq!(split_endpoint("0.0.0.0:*"), None, "a waiting socket has no other end");
        assert_eq!(split_endpoint("nonsense"), None);
    }

    /// `df` prints a device name that contains a space on some systems, and the columns after it
    /// are the ones that matter: reading from the left would shift every number by one.
    #[test]
    fn a_device_name_with_a_space_does_not_shift_the_numbers() {
        let lines = [
            "Filesystem 1024-blocks Used Available Capacity Mounted on",
            "map -hosts         0        0         0     100% /net",
            "map auto_home 1048576    12345    999999      2% /home",
        ];

        let disks = disks(&lines);
        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].filesystem, "map -hosts");
        assert_eq!(disks[0].mount_point, "/net");
        assert_eq!(disks[1].filesystem, "map auto_home");
        assert_eq!(disks[1].mount_point, "/home");
        assert_eq!(disks[1].size_bytes, 1_048_576 * 1024);
        assert_eq!(disks[1].available_bytes, 999_999 * 1024);
    }

    /// The heading of `df` has the same number of columns as a mount, so it is told apart by its
    /// block column being the word `1024-blocks` rather than by counting.
    #[test]
    fn the_df_heading_is_not_read_as_a_mount() {
        let disks = disks(&["Filesystem     1024-blocks     Used Available Capacity Mounted on"]);

        assert!(disks.is_empty());
    }

    /// An older kernel has no `MemAvailable`, which is not a reason to report a machine whose
    /// cache is full as a machine with no memory left.
    #[test]
    fn memory_without_an_available_column_falls_back_to_free_plus_cache() {
        let lines = [
            "MemTotal:        1000000 kB",
            "MemFree:          100000 kB",
            "Buffers:           50000 kB",
            "Cached:           200000 kB",
            "SReclaimable:      10000 kB",
            "SwapTotal:             0 kB",
            "SwapFree:              0 kB",
        ];

        let usage = memory_usage(&lines);
        assert_eq!(usage.total_bytes, 1_000_000 * 1024);
        assert_eq!(usage.available_bytes, 360_000 * 1024);
        assert_eq!(usage.swap_total_bytes, 0);
    }

    /// A host that answers nothing is a sample with nothing in it, not an error: the panel is
    /// showing a machine that is up, and what it has to say about it may be very little.
    #[test]
    fn a_host_that_reports_nothing_is_an_empty_sample() {
        let sample = parse("@@hostname\n@@uname\n@@os\n@@uptime\n@@load\n@@mem\n@@cpu\n@@cores\n@@net\n@@ps\n@@df\n");

        assert!(sample.hostname.is_none());
        assert!(sample.os.is_none());
        assert!(sample.kernel.is_none());
        assert!(sample.cores.is_none());
        assert!(sample.cpu_model.is_none());
        assert!(sample.uptime_seconds.is_none());
        assert!(sample.load.is_none());
        assert!(sample.cpu.is_none());
        assert!(
            sample.memory.is_none(),
            "an unread meminfo is not a machine with no memory"
        );
        assert!(sample.network.is_empty());
        assert!(sample.disks.is_empty());
        assert!(sample.processes.is_empty());

        let nothing = parse("");
        assert_eq!(nothing.memory, None);
        assert!(nothing.network.is_empty());
    }

    /// A `ps` this build cannot read is a host whose process table stays empty — the rest of the
    /// sample is still the answer.
    #[test]
    fn an_unreadable_process_table_does_not_take_the_sample_with_it() {
        let sample = parse("@@hostname\nweb-1\n@@ps\nps: unknown keyword pcpu\n@@df\n");

        assert_eq!(sample.hostname.as_deref(), Some("web-1"));
        assert!(sample.processes.is_empty());
    }

    #[test]
    fn a_filesystem_with_no_room_is_still_listed() {
        let lines = ["/dev/sdb1 1048576 1048576 0 100% /full"];

        let disks = disks(&lines);
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].available_bytes, 0, "no room is a reading, not a missing one");
    }
}

//! A sample of a real host, taken over a real connection.
//!
//! Ignored by default: it needs a running sshd. `deploy/lab/` has one and `make lab-up` starts it.
//!
//! ```text
//! SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 SEVENSHELL_TEST_USER=labuser \
//!   SEVENSHELL_TEST_PASSWORD=labpassword \
//!   cargo test -p sevenshell-core --test live_monitor -- --ignored
//! ```
//!
//! What only a real server can show: that the one command the monitor runs is a command a login
//! shell accepts, that the markers survive the trip, and that the counters come back as numbers a
//! second sample can be subtracted from. The parsing itself is proved against text in
//! `src/monitor.rs`, and the arithmetic on top of it in `lib/monitor/rates.spec.ts` — this test is
//! about the far side answering at all.
//!
//! A host that answers some of it and not the rest is the normal case rather than a failure: the
//! lab image is a small container, and what it does not report arrives as an absent section. The
//! assertions below are therefore about the sections every Linux host has, plus the shape of the
//! ones that may be empty.

use std::sync::Arc;

use sevenshell_core::Core;
use sevenshell_error::ErrorCode;
use sevenshell_types::{AuthMethod, HostMetrics, OpenOutcome, Session, SessionAuth, SocketRow, TerminalSettings};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;

struct Lab {
    /// Owned so the vault file outlives the test.
    _directory: TempDir,
    core: Arc<Core>,
    session_id: String,
    /// Kept alive so the terminal's output has somewhere to go; the monitor never reads it.
    events: mpsc::Receiver<sevenshell_core::CoreEvent>,
}

impl Lab {
    /// Opens a terminal, trusting whatever host key it is asked about, the way a person who has
    /// compared the fingerprint would.
    async fn open_trusting_everything(&self) -> String {
        for _ in 0..4 {
            match self.core.open_terminal(&self.session_id).await.expect("open") {
                OpenOutcome::Opened { handle, .. } => return handle,
                OpenOutcome::HostKeyUnknown { key } => self.core.trust_host_key(&key).expect("trust"),
                other => panic!("expected the session to open, got {other:?}"),
            }
        }
        panic!("the route never finished asking");
    }
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// A vault holding one session for the lab, with its password stored.
fn lab() -> Option<Lab> {
    let (Some(host), Some(port), Some(user), Some(password)) = (
        environment("SEVENSHELL_TEST_HOST"),
        environment("SEVENSHELL_TEST_PORT"),
        environment("SEVENSHELL_TEST_USER"),
        environment("SEVENSHELL_TEST_PASSWORD"),
    ) else {
        eprintln!("the SSH lab is not configured; skipping (set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD)");
        return None;
    };
    let port: u16 = port.parse().ok()?;

    let directory = tempdir().expect("temp dir");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault
        .create("the password of the vault itself, which is not the one under test")
        .expect("create vault");

    vault
        .save_session(
            Session {
                id: "lab-session".to_owned(),
                name: "lab".to_owned(),
                group_id: None,
                protocol: sevenshell_types::Protocol::Ssh,
                host,
                port,
                user,
                auth: SessionAuth::with_password(AuthMethod::Password),
                terminal: TerminalSettings::default(),
                jump: Vec::new(),
                forwards: Vec::new(),
                updated_at: 0,
            },
            Some(SessionSecret::Password(password)),
        )
        .expect("save session");

    let (core, events) = Core::new(Arc::clone(&vault));
    Some(Lab {
        _directory: directory,
        core,
        session_id: "lab-session".to_owned(),
        events,
    })
}

/// Empties the terminal's output, so a run that prints a banner does not leave it queued behind
/// the sample: the registry forwards a terminal's bytes through the same channel a long-running
/// test would otherwise fill.
fn drain(events: &mut mpsc::Receiver<sevenshell_core::CoreEvent>) {
    while events.try_recv().is_ok() {}
}

/// The sections a Linux host has whatever else it is missing, checked as a group so a failure says
/// which part of the command the far side did not answer.
fn assert_the_common_sections(sample: &HostMetrics) {
    assert!(
        sample.hostname.as_deref().is_some_and(|name| !name.is_empty()),
        "a host has a name: {sample:?}"
    );
    assert!(
        sample.kernel.is_some(),
        "uname answered but the kernel is missing: {sample:?}"
    );
    assert!(
        sample.arch.is_some(),
        "uname answered but the architecture is missing: {sample:?}"
    );
    assert!(
        sample.os.is_some(),
        "/etc/os-release or uname should have named the system: {sample:?}"
    );
    assert!(
        sample.uptime_seconds.is_some_and(|seconds| seconds >= 0.0),
        "/proc/uptime is the one file every Linux host has: {sample:?}"
    );
    assert!(sample.load.is_some(), "/proc/loadavg is read the same way: {sample:?}");

    let cpu = sample.cpu.expect("the CPU counters are in /proc/stat");
    assert!(cpu.total_ticks > 0, "a running host has spent some time: {cpu:?}");
    assert!(
        cpu.idle_ticks <= cpu.total_ticks,
        "idle time is part of the total, never more than it: {cpu:?}"
    );

    let memory = sample.memory.expect("/proc/meminfo is read the same way");
    assert!(memory.total_bytes > 0, "a container has memory: {memory:?}");
    assert!(
        memory.available_bytes <= memory.total_bytes,
        "what can be handed out is not more than there is: {memory:?}"
    );
    assert!(
        memory.swap_free_bytes <= memory.swap_total_bytes,
        "free swap is not more than swap: {memory:?}"
    );

    assert!(
        sample.network.iter().any(|entry| entry.name != "lo"),
        "a container has an interface besides loopback: {sample:?}"
    );

    // What the system page shows and the panel does not: the processor's own description and the
    // split of where its time went. Both come off `/proc/cpuinfo` and `/proc/stat`, which the lab
    // has — a host that names neither is a host that page would draw dashes for.
    let model = sample.cpu_model.as_ref().expect("/proc/cpuinfo names the processor");
    assert!(!model.name.is_empty(), "the name is what the page is for: {model:?}");
    assert!(
        model.cache_kb.is_some_and(|cache| cache > 0),
        "and its cache: {model:?}"
    );

    let states = sample.cpu.expect("counters").states;
    assert!(
        states.user_ticks + states.system_ticks + states.idle_ticks > 0,
        "a host that has been up has spent its time somewhere: {states:?}"
    );
}

#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab/"]
async fn a_real_host_is_sampled_over_an_open_terminal() {
    let Some(mut lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    drain(&mut lab.events);

    let sample = lab.core.sample_host(&handle).await.expect("a sample of a live host");

    assert_the_common_sections(&sample);
    assert!(sample.sampled_at > 0, "a sample is dated: {sample:?}");
    assert!(
        sample.round_trip_ms < 10_000,
        "the sample took longer than its own timeout: {}ms",
        sample.round_trip_ms
    );

    // The two sections that come from commands rather than from `/proc`, and the reason they are
    // asserted rather than tolerated as empty: a sample whose `ps` and `df` were not read looks
    // exactly like a host that has neither, and the panel would quietly show two empty tables
    // forever. The lab is a real Linux host with both, so an empty list here is a failure.
    assert!(
        !sample.processes.is_empty(),
        "the far side's `ps` should have listed processes: {sample:?}"
    );
    assert!(
        sample.disks.iter().any(|disk| disk.mount_point == "/"),
        "the root file system is mounted somewhere: {sample:?}"
    );

    for disk in &sample.disks {
        assert!(
            !disk.mount_point.is_empty(),
            "a mount is named by where it is: {disk:?}"
        );
        assert!(
            disk.available_bytes <= disk.size_bytes,
            "a file system cannot have more free than it has: {disk:?}"
        );
    }
    for process in &sample.processes {
        assert!(process.pid > 0, "a process has an id: {process:?}");
        assert!(!process.name.is_empty(), "and a name: {process:?}");
        assert!(!process.user.is_empty(), "and an account it runs as: {process:?}");
    }
    assert!(
        sample.processes.len() <= sevenshell_types::MAX_PROCESSES,
        "the table is cut to what fits the panel"
    );
    assert!(sample.disks.len() <= sevenshell_types::MAX_DISKS);
}

/// The counters a rate is computed from have to move, or every reading of CPU and traffic would be
/// a flat line drawn from numbers that were never a difference.
#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab/"]
async fn two_samples_of_one_host_are_counters_that_move() {
    let Some(mut lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    drain(&mut lab.events);

    let first = lab.core.sample_host(&handle).await.expect("a first sample");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let second = lab.core.sample_host(&handle).await.expect("a second sample");

    assert!(
        second.sampled_at > first.sampled_at,
        "two samples are taken at two moments: {} then {}",
        first.sampled_at,
        second.sampled_at
    );
    assert!(
        second.round_trip_ms > 0 || first.round_trip_ms > 0,
        "a sample over a network takes measurable time"
    );

    let (before, after) = (
        first.cpu.expect("counters").total_ticks,
        second.cpu.expect("counters").total_ticks,
    );
    assert!(
        after >= before,
        "CPU ticks count up, so a rate between them is a rate: {before} then {after}"
    );
}

/// A handle that is not an open terminal has nothing to ask, and the answer says so rather than
/// opening a connection of its own — which is the whole reason the monitor samples by handle.
#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab/"]
async fn sampling_a_terminal_that_is_not_open_is_refused() {
    let Some(lab) = lab() else {
        return;
    };

    let error = lab
        .core
        .sample_host("t404")
        .await
        .expect_err("there is no terminal behind that handle");

    assert_eq!(error.code(), ErrorCode::SessionNotFound);
}

/// The process page asks a question of its own, and the answer has to be a list somebody can read:
/// every row carrying how much processor time its process has used and where its program lives,
/// because those are what the page's two newest columns are made of.
///
/// The counters are checked for the property that makes them usable — they count up — rather than
/// for a value: what a busy host does between two reads is not something a test can state, but a
/// counter that went backwards would make every percentage on the page a negative share.
#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab/"]
async fn the_process_page_reads_counters_and_paths_for_every_row() {
    let Some(mut lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    drain(&mut lab.events);

    let page = lab.core.list_processes(&handle, 50).await.expect("a page of processes");

    assert!(
        page.ticks_per_second > 0,
        "the divisor for every percentage on the page"
    );
    assert!(!page.processes.is_empty(), "a running host has processes");
    assert!(page.processes.len() <= 50, "and no more than were asked for");
    for row in &page.processes {
        assert!(row.pid > 0);
        assert!(!row.name.is_empty(), "every row says what the process is: {row:?}");
        assert!(!row.user.is_empty(), "and who it runs as: {row:?}");
        // A program that is on disk is named by an absolute path. Not every process has one: a
        // kernel thread lives nowhere, and a row without a path is that rather than a broken read.
        if let Some(path) = &row.executable {
            assert!(path.starts_with('/'), "a program on disk has an absolute path: {row:?}");
        }
    }

    // Not every row has a counter, and the ones that do not are not a broken join: a read is three
    // commands, and the busiest process on a quiet host is often one of them — this very `ps` — which
    // is gone by the time the counters are read.
    let measured = page.processes.iter().filter(|row| row.cpu_ticks.is_some()).count();
    let named = page.processes.iter().filter(|row| row.executable.is_some()).count();
    assert!(measured > 0, "the read asks for processor time: {:#?}", page.processes);
    assert!(named > 0, "and for where each program lives: {:#?}", page.processes);

    // A moment later the counters have not gone backwards, whatever else happened.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let later = lab.core.list_processes(&handle, 50).await.expect("a second page");
    for row in &later.processes {
        let Some(before) = page.processes.iter().find(|earlier| earlier.pid == row.pid) else {
            continue;
        };
        assert!(
            row.cpu_ticks >= before.cpu_ticks,
            "{} used less processor time than a moment ago: {:?} then {:?}",
            row.pid,
            before.cpu_ticks,
            row.cpu_ticks
        );
    }

    // And the exclusion that makes the cut fair: the list arrives cut by CPU, so the busiest row is
    // first. What is *not* asserted is the percentage — that is the page's own arithmetic, proved
    // in `lib/monitor/processes.spec.ts` — this is about the host answering with the reading.
    // The first process, which is the one process certain to still be there: the busiest row of a
    // list is often a command from the read that produced it — this very `ps` — and looking at one
    // of those afterwards finds nothing, which is true but proves nothing.
    let detail = lab.core.process_detail(&handle, 1).await.expect("the first process");

    assert_eq!(detail.pid, 1);
    assert!(!detail.name.is_empty(), "the first process has a name: {detail:?}");
    assert!(
        !detail.command_line.is_empty(),
        "and the command line it was started with: {detail:?}"
    );
    // Its program and its directory are its owner's business: a connection that is not root reads
    // neither, and the page draws a dash there — which is why this asserts the shape of what came
    // back rather than that something did. The reading itself is proved where the connection is root.
    if !detail.executable.is_empty() {
        assert!(detail.executable.starts_with('/'), "a path is a path: {detail:?}");
    }
    if !detail.working_directory.is_empty() {
        assert!(
            detail.working_directory.starts_with('/'),
            "and so is a directory: {detail:?}"
        );
    }

    // One row that the host did measure, looked at closely: if the process is still there, the two
    // readings must agree about what it is and where it lives, because both are about one pid. A
    // process that has gone since is skipped rather than asserted about — it is gone, not wrong.
    for row in later.processes.iter().filter(|row| row.cpu_ticks.is_some()).take(5) {
        let looked = lab.core.process_detail(&handle, row.pid).await.expect("one process");
        if looked.name.is_empty() {
            continue;
        }
        assert_eq!(looked.name, row.name, "the same program, looked at two ways");
        assert_eq!(
            looked.executable,
            row.executable.clone().unwrap_or_default(),
            "and in the same place"
        );
        break;
    }

    // A process that is gone is described by nothing rather than reported as a failure: it exited
    // between the list and the look, which is the ordinary way for a process to end.
    let gone = lab
        .core
        .process_detail(&handle, 999_999)
        .await
        .expect("no such process");
    assert_eq!(gone.pid, 999_999);
    assert!(gone.name.is_empty());
    assert!(gone.environment.is_empty());
}

/// The network page reads the host's sockets, and two things have to be true of them on any host:
/// the port this test is connected to is listening, and the connection this test is holding is
/// counted under it — which is the join the page is built on, and the one a parser breaks silently.
///
/// It also covers both tools without the test knowing which one the host has. `ss` reports byte
/// counters on every socket and `netstat` reports none on any, but neither reports them on *some* —
/// so a list whose counters are patchy is a list whose `ss -i` info lines were not joined to their
/// rows, and that is what the count below is really checking.
#[tokio::test]
#[ignore = "needs a running sshd; see deploy/lab/"]
async fn the_network_page_sees_the_port_this_test_is_connected_on() {
    let Some(port) = environment("SEVENSHELL_TEST_PORT").and_then(|port| port.parse::<u16>().ok()) else {
        eprintln!("SEVENSHELL_TEST_PORT is not a port number; skipping");
        return;
    };
    let Some(mut lab) = lab() else {
        return;
    };
    let handle = lab.open_trusting_everything().await;
    drain(&mut lab.events);

    let sockets = lab.core.list_sockets(&handle).await.expect("a list of sockets");

    assert!(!sockets.is_empty(), "a listening sshd is a socket");
    let listeners: Vec<&SocketRow> = sockets.iter().filter(|row| is_listening(row)).collect();
    assert!(!listeners.is_empty(), "and it is listening: {sockets:?}");

    let sshd = listeners.iter().find(|row| row.port == port).unwrap_or_else(|| {
        panic!("nothing is listening on the port this test is connected to ({port}): {listeners:?}")
    });

    let attached = sockets
        .iter()
        .filter(|row| !is_listening(row) && row.port == port && row.protocol == sshd.protocol)
        .count();
    assert!(
        attached >= 1,
        "the terminal this test opened is a connection on that port: {sockets:?}"
    );

    for row in &sockets {
        assert!(!row.address.is_empty(), "every socket is somewhere: {row:?}");
        assert!(row.port > 0, "and on a port: {row:?}");
        if row.pid > 0 {
            assert!(!row.process.is_empty(), "a socket with an owner has a name: {row:?}");
        }
        assert_eq!(
            row.sent_bytes.is_some(),
            row.received_bytes.is_some(),
            "the two counters come off one line and arrive together: {row:?}"
        );
        // A live `ss` prints an info line under a waiting socket too, and what that line carries is
        // socket memory rather than traffic. Counters landing on one is exactly what a mis-joined
        // info line looks like, which is why this is asserted rather than assumed.
        if is_listening(row) {
            assert!(
                row.sent_bytes.is_none(),
                "a socket that is waiting has no traffic to report: {row:?}"
            );
        }
    }

    // And where the counters are, on a host that reports them at all: on the connections, including
    // the one this test is holding. A host whose tool reports none — busybox `netstat` — skips this,
    // and the counters' absence is then the honest answer rather than a missing feature.
    if sockets.iter().any(|row| row.sent_bytes.is_some()) {
        assert!(
            sockets
                .iter()
                .any(|row| !is_listening(row) && row.port == port && row.sent_bytes.is_some()),
            "the connections on the port this test holds are what the counters are for: {sockets:#?}"
        );
    }

    assert!(
        sockets.len() <= sevenshell_types::MAX_SOCKETS,
        "and the list is cut to fit"
    );
}

/// The predicate the interface uses, so the test and the page agree about what waiting means —
/// `UNCONN` included, because a UDP socket that names no peer is waiting too.
fn is_listening(row: &SocketRow) -> bool {
    row.state == "LISTEN" || row.state == "UNCONN"
}

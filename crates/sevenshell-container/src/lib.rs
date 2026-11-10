//! Container runtimes reached through their own command line.
//!
//! Docker, Podman, nerdctl and WSLC are four surfaces over the same idea: a daemon somewhere runs
//! containers and a CLI describes them as text. This crate owns the part that differs — which
//! arguments each runtime takes — and the part that is the same — parsing the answer into the wire
//! types the interface draws. It speaks no shell: every operation is an argument vector, and only
//! the remote runner turns one into a command line, quoting each argument on the way.
//!
//! Where the CLI runs is not decided here. A [`Runner`](runner::Runner) is either a process on this
//! machine or an SSH exec channel, and the manager above it asks the same questions of both.

mod commands;
mod local;
mod manager;
mod parse;
mod remote;
mod runner;

pub use manager::ContainerManager;
pub use runner::Runner;

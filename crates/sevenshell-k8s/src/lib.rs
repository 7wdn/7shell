//! A Kubernetes API client.
//!
//! Kubeconfig is read here, never in the interface: it carries bearer tokens and client keys, and
//! the one thing that crosses back is the list of contexts a person may choose between. Requests
//! are plain HTTP against the API server; lists, watches, logs and the exec channel are four
//! different shapes of the same connection, not four clients.
//!
//! The API surface is deliberately data-driven: resources are described by the caller, so a kind
//! this build has never heard of still lists, watches and reads like any other.

mod client;
mod exec;
mod kubeconfig;
mod logs;
mod metrics;
mod rest;
mod watch;

pub use client::{
    ApiDialer, ApiStream, BoxedApiStream, DialFuture, K8sClient, K8sManager, KubeContext, Kubeconfig, TunneledCluster,
};
pub use exec::{ExecSession, RunOutput, RunSession, open_exec, open_run};
pub use logs::{LogHandle, LogOptions, LogStream, open_logs, open_logs_with_options};

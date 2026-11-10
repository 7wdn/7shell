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

pub use client::{K8sClient, K8sManager, KubeContext, Kubeconfig};

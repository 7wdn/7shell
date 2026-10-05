//! The command surface the interface may call.
//!
//! A command is a thin adapter: it takes wire types in, delegates to a lower layer, and returns a
//! wire type or a [`sevenshell_error::ShellError`] out. Logic that is worth testing does not live in
//! this module tree, because nothing here can be exercised without a running webview.
//!
//! Secrets travel one way through this surface: passwords and passphrases may be *sent* to a
//! command, and no command ever returns one — not in a result, not in a diagnostic, not in an
//! error detail.

pub mod app;
pub mod batch;
pub mod clipboard;
pub mod files;
pub mod keys;
pub mod known_hosts;
pub mod monitor;
pub mod sessions;
pub mod settings;
pub mod terminal;
pub mod vault;

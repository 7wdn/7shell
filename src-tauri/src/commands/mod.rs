//! The command surface the interface may call.
//!
//! A command is a thin adapter: it takes wire types in, delegates to a lower layer,
//! and returns a wire type or a [`sevenshell_error::ShellError`] out. Logic that is
//! worth testing does not live in this module tree, because nothing here can be
//! exercised without a running webview.

pub mod app;
pub mod settings;

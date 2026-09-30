//! Environment and descriptor facts the interface needs at startup.

use serde::{Deserialize, Serialize};

use crate::generated::{DESCRIPTOR_SCHEMA_VERSION, Protocol};

/// The name people see. The repository and package identifier is `sevenshell`;
/// this is what the window title, the installers and the SSH client banner say.
pub const PRODUCT_NAME: &str = "7Shell";

/// What the interface asks for once, when it starts.
///
/// It carries no file system paths: the interface never names a local path, it only
/// receives tokens for files the user picked through a native dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub debug_build: bool,
    pub protocol_count: usize,
    pub descriptor_schema_version: u32,
}

impl AppInfo {
    pub fn current() -> Self {
        Self {
            name: PRODUCT_NAME.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            debug_build: cfg!(debug_assertions),
            protocol_count: Protocol::ALL.len(),
            descriptor_schema_version: DESCRIPTOR_SCHEMA_VERSION,
        }
    }
}

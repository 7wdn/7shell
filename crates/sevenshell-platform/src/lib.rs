//! Cross-cutting primitives: where application data lives, and how it is read and written.
//!
//! This crate is deliberately small and knows nothing about the desktop shell. A future
//! command line and a future headless mode must look for the same directory on the same
//! machine, and that is only true if the rule lives in one place that does not depend on a
//! window existing.

pub mod fonts;
pub mod paths;
pub mod settings;

pub use fonts::list_families;
pub use paths::{APP_IDENTIFIER, config_dir, settings_file};
pub use settings::{SETTINGS_SCHEMA_VERSION, load, load_from, save, save_to};

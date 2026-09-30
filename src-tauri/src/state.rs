//! What the shell keeps between commands.
//!
//! Two long-lived things: the vault (one file, one lock state) and the core (the registry of open
//! terminals, plus the receiver their output arrives on). Both are created once, in `setup`, and
//! handed to every command through managed state.

use std::sync::Arc;

use sevenshell_core::Core;
use sevenshell_vault::Vault;

use crate::dropped::PendingDrops;

/// The application state commands borrow.
pub struct ShellState {
    vault: Arc<Vault>,
    core: Arc<Core>,
    /// Files the operating system dropped or the clipboard held, waiting to be uploaded.
    pending_drops: PendingDrops,
}

impl ShellState {
    /// Opens the vault at its usual place and builds the core around it.
    ///
    /// The vault is *not* unlocked here: it starts locked, and the interface asks for the password.
    pub fn new() -> (Self, tokio::sync::mpsc::Receiver<sevenshell_core::CoreEvent>) {
        let path = sevenshell_vault::default_path().unwrap_or_else(|| {
            log::warn!("the configuration directory could not be located; the vault will not persist");
            std::path::PathBuf::from("vault.json")
        });

        let vault = Arc::new(Vault::open_at(path));
        let (core, events) = Core::new(Arc::clone(&vault));
        (
            Self {
                vault,
                core,
                pending_drops: PendingDrops::default(),
            },
            events,
        )
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    /// A handle to the core that can be moved onto another task, for work that outlives a command.
    pub fn core_arc(&self) -> Arc<Core> {
        Arc::clone(&self.core)
    }

    /// What a drop or a paste left behind, keyed by the token the interface was given.
    pub fn pending_drops(&self) -> &PendingDrops {
        &self.pending_drops
    }
}

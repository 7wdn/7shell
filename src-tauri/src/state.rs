//! What the shell keeps between commands.
//!
//! Two long-lived things: the vault (one file, one lock state) and the core (the registry of open
//! terminals, plus the receiver their output arrives on). Both are created once, in `setup`, and
//! handed to every command through managed state.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sevenshell_core::Core;
use sevenshell_vault::Vault;

use crate::dropped::PendingDrops;
use crate::uploads::PendingUploads;

/// The application state commands borrow.
pub struct ShellState {
    vault: Arc<Vault>,
    core: Arc<Core>,
    /// Files the operating system dropped or the clipboard held, waiting to be uploaded.
    pending_drops: PendingDrops,
    /// Uploads that stopped to ask about a name already on the far side, waiting for the answer.
    pending_uploads: PendingUploads,
    /// Whether the interface has said that closing the window would throw something away.
    quit_guard: AtomicBool,
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
                pending_uploads: PendingUploads::default(),
                quit_guard: AtomicBool::new(false),
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

    /// Uploads held while the interface answers the question about names already there.
    pub fn pending_uploads(&self) -> &PendingUploads {
        &self.pending_uploads
    }

    /// Whether the interface has something a close would throw away.
    ///
    /// A flag the *interface* sets rather than a question the shell answers itself: the shell cannot
    /// see an editor holding unsaved text or a transfer in flight, and a window that refused to
    /// close because Rust guessed wrong would be worse than one that closed too eagerly. Unset is the
    /// default, and it is also what a frontend that has died leaves behind.
    pub fn set_quit_guard(&self, armed: bool) {
        self.quit_guard.store(armed, Ordering::Relaxed);
    }

    pub fn quit_guard(&self) -> bool {
        self.quit_guard.load(Ordering::Relaxed)
    }
}

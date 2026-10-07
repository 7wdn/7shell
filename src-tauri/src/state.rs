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
    /// Whether closing the window ends the application or puts it away.
    ///
    /// The shell's copy of one setting rather than a read of the settings file: the file is the
    /// interface's to write and this decision is made on the way out, where a failed read would be a
    /// window that would not close.
    close_to_tray: AtomicBool,
    /// Whether the tray icon is on screen, which is what a window can be put away *to*.
    tray_shown: AtomicBool,
    /// Whether the interface has answered the close question and meant "quit".
    ///
    /// The same shape as the guard above and for the same reason: only the interface can see what is
    /// unsaved, so only the interface can say that it has been dealt with. Without it, a window set to
    /// hide would hide again on the way out of its own quit question.
    quit_approved: AtomicBool,
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
                close_to_tray: AtomicBool::new(true),
                tray_shown: AtomicBool::new(false),
                quit_approved: AtomicBool::new(false),
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

    /// Records what the settings say about closing: whether the window should go away rather than
    /// the application ending.
    pub fn set_close_to_tray(&self, put_away: bool) {
        self.close_to_tray.store(put_away, Ordering::Relaxed);
    }

    pub fn close_to_tray(&self) -> bool {
        self.close_to_tray.load(Ordering::Relaxed)
    }

    /// Records whether there is an icon to bring a hidden window back from.
    pub fn set_tray_shown(&self, shown: bool) {
        self.tray_shown.store(shown, Ordering::Relaxed);
    }

    pub fn tray_shown(&self) -> bool {
        self.tray_shown.load(Ordering::Relaxed)
    }

    /// Says that the close question has been asked and answered, so the next request is obeyed.
    pub fn approve_quit(&self) {
        self.quit_approved.store(true, Ordering::Relaxed);
        self.quit_guard.store(false, Ordering::Relaxed);
    }

    pub fn quit_approved(&self) -> bool {
        self.quit_approved.load(Ordering::Relaxed)
    }
}

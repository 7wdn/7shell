//! Orchestration: the layer that knows the order of things.
//!
//! The vault knows how to store, the transport knows how to talk and the registry knows who is
//! open. None of them knows that a host key has to be agreed with a person before a password is
//! sent to a server — that order lives here, in one place, where it can be read and tested.
//!
//! There is no dependency on the desktop shell, on purpose: the same operations have to be
//! callable from a command line later.

mod batch;
mod files;
mod sessions;

pub use batch::{BatchEvent, DEFAULT_PARALLEL, MAX_HOST_TIMEOUT, MAX_PARALLEL, MIN_HOST_TIMEOUT};
pub use files::FileSession;
pub use sessions::{Core, CoreEvent, OPEN_TIMEOUT, TransferRecord};

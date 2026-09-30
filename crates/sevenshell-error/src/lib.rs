//! Error envelope, code catalog and redaction for 7Shell.
//!
//! Every layer reports failures through [`ShellError`] so the desktop shell, the
//! command line and (later) the agent interface all carry the same shape: a stable
//! code, a message key to localize, an origin, and an explicit statement about
//! whether sensitive material left the process.
//!
//! The frontend mirrors this shape in `apps/desktop/src/lib/backend/errors.ts` and
//! `packages/app-tests/errorCatalog.contract.test.ts` fails when the two drift.

mod code;
mod envelope;
mod redact;

pub use code::ErrorCode;
pub use envelope::{ENVELOPE_VERSION, OperationOutcome, Origin, ShellError, Subsystem};
pub use redact::{MASK, MAX_DETAIL_CHARS, is_secret_key, scrub, scrub_bounded, truncate_chars};

//! A copy of the library that travels: what a snapshot carries, and where it goes.
//!
//! Three things live here, and they are the three a backup is made of.
//!
//! * **The snapshot** ([`snapshot`]) — everything worth carrying, in one file, in two forms: plain,
//!   which never contains a credential, and sealed, which contains them and nothing readable.
//! * **The plan** ([`plan`]) — what a restore *would* write, decided without touching anything, so
//!   the answer can be asserted and the write afterwards is mechanical.
//! * **The file** ([`backup`]) — the backup on disk, written so an interrupted one cannot lose the
//!   copy that was already there.
//!
//! What this crate does *not* do is write: a restore hands back the two things it decided — what the
//! vault holds and what the settings should become — and the host writes them, because it is the
//! layer that already owns the files. That is also why the settings arrive here as a value rather
//! than being read from the platform's file: a rule that reads the machine it is running on is a
//! rule that cannot be tested against a machine that is not.
//!
//! The rules this crate is held to:
//!
//! * **Credentials travel sealed, or they do not travel.** The plain builder never reads the
//!   vault's secrets; the sealed one writes them nowhere but inside a payload. There is no third
//!   way, and the file records which of the two happened rather than what was asked for.
//! * **Adding never overwrites, unless somebody said so.** An id that is taken gets a fresh one, so
//!   restoring the same file twice makes a second copy. Replacement is a switch somebody turns on,
//!   never a default.
//! * **Nothing points outside the file.** A session whose group is not carried lands at the root, a
//!   hop that is not carried is dropped and counted, and a key is either carried or named as
//!   missing — a file must not re-point at whatever happens to be on the machine it is opened on.
//! * **What could not be carried is counted.** Anything a restore could not bring is reported
//!   beside the things it did bring, because a session that will not connect later, for a reason
//!   nobody can see, is the failure this whole list exists to prevent.

mod backup;
mod plan;
mod settings;
mod snapshot;
mod snippets;
mod webdav;

pub use backup::{read, write};
pub use plan::{Destination, RestorePlan, plan_restore};
pub use settings::PortableSettings;
pub use snapshot::{
    BuiltSnapshot, MAX_SNAPSHOT_BYTES, OpenedSnapshot, PROTECTION_PASSPHRASE, SNAPSHOT_FORMAT, SNAPSHOT_SEALED_VERSION,
    SNAPSHOT_VERSION, SnapshotEnvelope, SyncSnapshot, build, open,
};
pub use snippets::{
    SNIPPET_ENVELOPE_FORMAT, SNIPPET_ENVELOPE_VERSION, SNIPPET_FILE_NAME, SNIPPET_TITLE, SnippetProvider,
    SnippetRequest, SnippetSync, api_base, create_body, file_in, is_ours, open_snippet, read_request, response_id,
    seal, snippet_url, test_request, write_request,
};
pub use webdav::{DEFAULT_REMOTE_PATH, SyncService, WebDav};
